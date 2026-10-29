use std::{io::Write, path::PathBuf};

use anyhow::Context;
use bitvec::{boxed::BitBox, order::Msb0, slice::BitSlice, vec::BitVec};
use tracing::debug_span;

use crate::spawn_utils::BlockingSpawner;

/// Magic bytes identifying the new bitv format with CRC32 checksum.
const BITV_MAGIC: [u8; 4] = [0x52, 0x42, 0x56, 0x02]; // "RBV\x02"
const BITV_HEADER_LEN: usize = 8; // 4 magic + 4 CRC32

/// Wrap a bitfield payload with a header containing magic bytes and CRC32 checksum.
pub fn bitv_with_header(payload: &[u8]) -> Vec<u8> {
    let crc = crc32fast::hash(payload);
    let mut buf = Vec::with_capacity(BITV_HEADER_LEN + payload.len());
    buf.extend_from_slice(&BITV_MAGIC);
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Result of reading a bitv file.
pub struct BitvReadResult {
    /// The bitfield payload bytes.
    pub payload: Vec<u8>,
    /// Whether a CRC32 checksum was present and verified.
    pub checksum_verified: bool,
    /// Whether the file used the new format (has magic header).
    pub has_header: bool,
}

/// Parse a bitv file buffer, handling both old (raw) and new (header+CRC) formats.
pub fn read_bitv_file(buf: &[u8]) -> BitvReadResult {
    if buf.len() >= BITV_HEADER_LEN && buf[..4] == BITV_MAGIC {
        let stored_crc = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let payload = &buf[BITV_HEADER_LEN..];
        let computed_crc = crc32fast::hash(payload);
        BitvReadResult {
            payload: payload.to_vec(),
            checksum_verified: stored_crc == computed_crc,
            has_header: true,
        }
    } else {
        // Old format: entire buffer is the raw bitfield payload
        BitvReadResult {
            payload: buf.to_vec(),
            checksum_verified: false,
            has_header: false,
        }
    }
}

pub trait BitV: Send + Sync {
    fn as_slice(&self) -> &BitSlice<u8, Msb0>;
    fn as_slice_mut(&mut self) -> &mut BitSlice<u8, Msb0>;
    fn into_dyn(self) -> Box<dyn BitV>;
    fn as_bytes(&self) -> &[u8];
    fn flush(&mut self, flush_async: bool) -> anyhow::Result<()>;
}

pub type BoxBitV = Box<dyn BitV>;

struct DiskFlushRequest {
    snapshot: BitBox<u8, Msb0>,
}

pub struct DiskBackedBitV {
    bv: BitBox<u8, Msb0>,
    flush_tx: tokio::sync::mpsc::UnboundedSender<DiskFlushRequest>,
}

impl Drop for DiskBackedBitV {
    fn drop(&mut self) {
        if self
            .flush_tx
            .send(DiskFlushRequest {
                snapshot: self.bv.clone(),
            })
            .is_err()
        {
            tracing::warn!("error flushing bitv on drop: flusher task is dead")
        }
    }
}

// NOTE on mmap. rtbit used it for a while, but it has issues on slow disks.
// We want writes to bitv to be instant in RAM. However when disk is slow, occasionally
// the writes stall which blocks the executor.
// Thus this separate "thread" of flushing was implemented.
impl DiskBackedBitV {
    pub async fn new(filename: PathBuf, spawner: BlockingSpawner) -> anyhow::Result<Self> {
        let raw_buf = tokio::fs::read(&filename)
            .await
            .with_context(|| format!("error reading {filename:?}"))?;

        let result = read_bitv_file(&raw_buf);

        // New format with CRC mismatch = definite corruption
        if result.has_header && !result.checksum_verified {
            tracing::warn!(?filename, "bitv file CRC32 mismatch, treating as corrupt");
            anyhow::bail!("bitv file CRC32 mismatch");
        }

        let bv = BitVec::from_vec(result.payload).into_boxed_bitslice();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DiskFlushRequest>();
        let flush_filename = filename.clone();
        // Detached on purpose: the task must outlive the session's cancellation
        // token so a torrent paused during shutdown can still flush. It exits
        // when every sender (the bitfield) is dropped.
        librtbit_core::spawn_utils::spawn(
            debug_span!("diskbitv-flusher", ?filename),
            format!("DiskBackedBitV::flusher {filename:?}"),
            async move {
                let filename = flush_filename;
                loop {
                    let Some(mut req) = rx.recv().await else {
                        break;
                    };
                    while let Ok(r) = rx.try_recv() {
                        req = r;
                    }

                    let tmp_path = filename.with_extension("bitv.tmp");
                    let tmp_path_clone = tmp_path.clone();
                    let filename_clone = filename.clone();

                    if let Err(e) = spawner
                        .block_in_place_with_semaphore(|| {
                            let data = bitv_with_header(req.snapshot.as_raw_slice());
                            let mut tmp_file = std::fs::OpenOptions::new()
                                .write(true)
                                .create(true)
                                .truncate(true)
                                .open(&tmp_path_clone)?;
                            tmp_file.write_all(&data)?;
                            tmp_file.sync_all()?;
                            drop(tmp_file);
                            std::fs::rename(&tmp_path_clone, &filename_clone)?;
                            Ok::<_, anyhow::Error>(())
                        })
                        .await
                    {
                        tracing::error!(?filename, "error writing bitv atomically: {e:#}");
                        // Best-effort cleanup of temp file
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                        if let Err(e) = tokio::fs::remove_file(&filename).await {
                            tracing::error!(?filename, "error removing bitv: {e:#}");
                        }
                        break;
                    }
                }

                Ok::<_, anyhow::Error>(())
            },
        );
        Ok(Self { bv, flush_tx: tx })
    }
}

#[async_trait::async_trait]
impl BitV for BitBox<u8, Msb0> {
    fn as_slice(&self) -> &BitSlice<u8, Msb0> {
        self.as_bitslice()
    }

    fn as_slice_mut(&mut self) -> &mut BitSlice<u8, Msb0> {
        self.as_mut_bitslice()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_raw_slice()
    }

    fn flush(&mut self, _flush_async: bool) -> anyhow::Result<()> {
        Ok(())
    }

    fn into_dyn(self) -> Box<dyn BitV> {
        Box::new(self)
    }
}

impl BitV for DiskBackedBitV {
    fn as_slice(&self) -> &BitSlice<u8, Msb0> {
        self.bv.as_bitslice()
    }

    fn as_slice_mut(&mut self) -> &mut BitSlice<u8, Msb0> {
        self.bv.as_mut_bitslice()
    }

    fn as_bytes(&self) -> &[u8] {
        self.bv.as_raw_slice()
    }

    fn flush(&mut self, _flush_async: bool) -> anyhow::Result<()> {
        let req = DiskFlushRequest {
            snapshot: self.bv.clone(),
        };
        self.flush_tx.send(req).context("flusher task is dead")
    }

    fn into_dyn(self) -> Box<dyn BitV> {
        Box::new(self)
    }
}

impl BitV for Box<dyn BitV> {
    fn as_slice(&self) -> &BitSlice<u8, Msb0> {
        (**self).as_slice()
    }

    fn as_slice_mut(&mut self) -> &mut BitSlice<u8, Msb0> {
        (**self).as_slice_mut()
    }

    fn as_bytes(&self) -> &[u8] {
        (**self).as_bytes()
    }

    fn flush(&mut self, flush_async: bool) -> anyhow::Result<()> {
        (**self).flush(flush_async)
    }

    fn into_dyn(self) -> Box<dyn BitV> {
        self
    }
}

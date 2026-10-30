//! Shutdown must persist piece bitfields and must not tear the uTP dispatcher
//! down while the accept loop is still running (WI-2156).

use std::{net::SocketAddr, path::Path};

use anyhow::Context;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};
use tracing_subscriber::Layer;

use crate::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerMode, ListenerOptions, Session,
    SessionOptions, create_torrent, session::SessionPersistenceConfig,
    spawn_utils::BlockingSpawner, tests::test_util::create_default_random_dir_with_torrents,
};

/// Collect warning lines that mean shutdown raced the flusher or the uTP
/// dispatcher. Installed as the default subscriber for this test only.
fn warning_capture() -> tracing::subscriber::DefaultGuard {
    use tracing_subscriber::layer::SubscriberExt;
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(|| Capture)
        .with_filter(tracing_subscriber::filter::LevelFilter::WARN);
    tracing::subscriber::set_default(tracing_subscriber::registry().with(layer))
}

struct Capture;
impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut buf_guard) = CAPTURE.lock() {
            buf_guard.extend_from_slice(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

static CAPTURE: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

fn captured() -> String {
    let buf = CAPTURE.lock().expect("log capture lock");
    String::from_utf8_lossy(&buf).into_owned()
}

async fn seeded_torrent(prefix: &str) -> anyhow::Result<(TempDir, Vec<u8>)> {
    let payload = create_default_random_dir_with_torrents(1, 32 * 1024, Some(prefix));
    let torrent = create_torrent(
        payload.path(),
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await?;
    Ok((payload, torrent.as_bytes()?.to_vec()))
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_flushes_bitfield_and_does_not_kill_utp_dispatcher() -> anyhow::Result<()> {
    CAPTURE.lock().expect("log capture lock").clear();
    let _guard = warning_capture();

    let root = TempDir::with_prefix("shutdown_flush")?;
    let persist = root.path().join("session");
    let output = root.path().join("out");
    std::fs::create_dir_all(&persist)?;
    std::fs::create_dir_all(&output)?;

    let (payload, torrent) = seeded_torrent("shutdown_flush").await?;

    let session = Session::new_with_opts(
        output.clone(),
        SessionOptions {
            disable_dht: true,
            disable_dht_persistence: true,
            disable_trackers: true,
            disable_local_service_discovery: true,
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(persist.clone()),
            }),
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpAndUtp,
                listen_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .context("session")?;

    // The payload is the torrent's own content, so initialization is a fast
    // resume check that marks every piece present — an active torrent with a
    // bitfield that shutdown must persist.
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                overwrite: true,
                paused: false,
                output_folder: Some(payload.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("handle")?;
    timeout(Duration::from_secs(15), handle.wait_until_completed())
        .await
        .context("wait")??;

    let info_hash = handle.info_hash();
    session.stop().await;
    drop(session);
    // Give the detached flusher a moment to rename its temp file.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let text = captured();
    assert!(
        !text.contains("flusher task is dead"),
        "bitfield flusher died before torrents flushed: {text}"
    );
    assert!(
        !text.contains("dispatcher dead"),
        "uTP dispatcher died while the accept loop was running: {text}"
    );

    let bitv = bitv_path(&persist, &info_hash);
    assert!(
        bitv.exists(),
        "bitfield was not persisted at {}",
        bitv.display()
    );
    let bytes = std::fs::read(&bitv)?;
    // The on-disk format is the 4-byte RBV\x02 magic, a little-endian CRC32 and
    // one bit per piece. Shutdown itself is not under test here beyond having
    // written that file.
    // On-disk magic is ASCII "RBV" plus version 0x02 (see bitv.rs), not the
    // four letters "RBV\x02". The header is that magic and a little-endian CRC32.
    const BITV_MAGIC: [u8; 4] = [0x52, 0x42, 0x56, 0x02];
    assert!(
        bytes.len() >= 8 && bytes[..4] == BITV_MAGIC,
        "persisted bitfield is missing the RBV magic: {:02x?}",
        &bytes[..bytes.len().min(8)]
    );
    let piece_bytes = handle
        .with_metadata(|metadata| metadata.lengths().piece_bitfield_bytes())
        .context("metadata")?;
    assert_eq!(
        bytes.len(),
        8 + piece_bytes,
        "RBV header plus one bit per piece"
    );
    assert!(
        bytes[8..].iter().any(|byte| *byte != 0),
        "the completed torrent's bitfield must have pieces set"
    );
    Ok(())
}

fn bitv_path(folder: &Path, info_hash: &librtbit_core::Id20) -> std::path::PathBuf {
    folder.join(format!("{info_hash:?}.bitv"))
}

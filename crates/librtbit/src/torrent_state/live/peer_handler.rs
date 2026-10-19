use std::{
    net::SocketAddr,
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, bail};
use buffers::{ByteBuf, ByteBufOwned};
use clone_to_owned::CloneToOwned;
use librtbit_core::{constants::CHUNK_SIZE, lengths::ChunkInfo, spawn_utils::spawn_with_cancel};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use peer_binary_protocol::{
    Handshake, Message, Piece, Request,
    extended::{
        ExtendedMessage,
        handshake::ExtendedHandshake,
        ut_holepunch::{HolepunchErrorCode, HolepunchMessage, HolepunchMsgType},
        ut_metadata::{UtMetadata, UtMetadataData},
        ut_pex::UtPex,
    },
};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, debug_span, error, trace, warn};

use crate::{
    Error,
    chunk_tracker::ChunkMarkingResult,
    chunk_tracker::PieceOrder,
    peer_connection::{PeerConnectionHandler, WriterRequest},
    piece_tracker::{AcquireRequest, AcquireResult},
    stream_connect::ConnectionKind,
    torrent_state::utils::{TimedExistence, timeit},
    type_aliases::{BF, PeerHandle},
};

use super::{
    TorrentStateLive, TorrentStateLocked, make_piece_bitfield,
    peer::{PeerState, PeerTx, stats::atomic::PeerCountersAtomic as AtomicPeerCounters},
};

pub(crate) struct PeerHandlerLocked {
    pub i_am_choked: bool,
}

// All peer state that would never be used by other actors should pe put here.
// This state tracks a live peer.
pub(crate) struct PeerHandler {
    pub(crate) state: Arc<TorrentStateLive>,
    pub(crate) counters: Arc<AtomicPeerCounters>,
    // Semantically, we don't need an RwLock here, as this is only requested from
    // one future (requester + manage_peer).
    //
    // However as PeerConnectionHandler takes &self everywhere, we need shared mutability.
    // RefCell would do, but tokio is unhappy when we use it.
    pub(crate) _locked: RwLock<PeerHandlerLocked>,

    // This is used to unpause chunk requester once the bitfield
    // is received.
    pub(crate) on_bitfield_notify: Notify,

    // This is used to unpause after we were choked.
    pub(crate) unchoke_notify: Notify,

    // This is used to limit the number of chunk requests we send to a peer at a time.
    pub(crate) requests_sem: Semaphore,

    pub(crate) addr: SocketAddr,
    pub(crate) incoming: bool,
    pub(crate) tx: PeerTx,

    pub(crate) first_message_received: AtomicBool,

    pub(crate) cancel_token: CancellationToken,
}

impl PeerConnectionHandler for &'_ PeerHandler {
    fn on_connected(&self, connection_time: Duration) {
        self.counters
            .outgoing_connections
            .fetch_add(1, Ordering::Relaxed);
        #[allow(clippy::cast_possible_truncation)]
        self.counters
            .total_time_connecting_ms
            .fetch_add(connection_time.as_millis() as u64, Ordering::Relaxed);
    }

    async fn on_received_message(&self, message: Message<'_>) -> anyhow::Result<()> {
        // The first message must be "bitfield", but if it's not sent,
        // assume the bitfield is all zeroes and was sent.
        if !matches!(&message, Message::Bitfield(..))
            && !self.first_message_received.swap(true, Ordering::Relaxed)
        {
            self.on_bitfield_notify.notify_waiters();
        }

        match message {
            Message::Request(request) => {
                self.on_download_request(request)
                    .context("on_download_request")?;
            }
            Message::Bitfield(b) => self
                .on_bitfield(b.clone_to_owned(None))
                .context("on_bitfield")?,
            Message::Choke => self.on_i_am_choked(),
            Message::Unchoke => self.on_i_am_unchoked(),
            Message::Interested => self.on_peer_interested(),
            Message::Piece(piece) => self
                .on_received_piece(piece)
                .await
                .context("on_received_piece")?,
            Message::KeepAlive => {
                trace!("keepalive received");
            }
            Message::Have(h) => self.on_have(h),
            Message::Port(port) => {
                if let Some(session) = self.state.shared.session.upgrade()
                    && let Some(dht) = session.get_dht()
                {
                    dht.ping(SocketAddr::new(self.addr.ip(), port)).await.ok();
                }
            }
            Message::SuggestPiece(_) | Message::AllowedFast(_) | Message::RejectRequest(_) => {
                trace!("received optional BEP 6 message");
            }
            Message::HaveAll => self.on_bitfield(
                make_piece_bitfield(&self.state.lengths)
                    .as_raw_slice()
                    .to_vec()
                    .into(),
            )?,
            Message::HaveNone => {
                self.on_bitfield(vec![0; self.state.lengths.piece_bitfield_bytes()].into())?
            }
            Message::NotInterested => {
                trace!("received \"not interested\", but we don't process it yet")
            }
            Message::Cancel(_) => {
                trace!("received \"cancel\", but we don't process it yet")
            }
            Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(
                metadata_piece_id,
            ))) => {
                if self.state.metadata.info.info().private {
                    warn!(
                        id = self.state.shared.id,
                        info_hash = ?self.state.shared.info_hash,
                        "received noncompliant ut_metadata message from {}, ignoring",
                        self.addr
                    );
                } else {
                    self.send_metadata_piece(metadata_piece_id)
                        .with_context(|| {
                            format!("error sending metadata piece {metadata_piece_id}")
                        })?;
                }
            }
            Message::Extended(ExtendedMessage::UtPex(pex)) => {
                if self.state.metadata.info.info().private {
                    warn!(
                        id = self.state.shared.id,
                        info_hash = ?self.state.shared.info_hash,
                        "received noncompliant PEX message from {}, ignoring",
                        self.addr
                    );
                } else {
                    self.on_pex_message(pex);
                }
            }
            Message::Extended(ExtendedMessage::UtHolepunch(msg)) => {
                self.on_holepunch_message(msg);
            }
            message => {
                warn!(
                    id = self.state.shared.id,
                    info_hash = ?self.state.shared.info_hash,
                    "received unsupported message {:?}, ignoring", message
                );
            }
        };
        Ok(())
    }

    fn serialize_bitfield_message_to_buf(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        let g = self.state.lock_read("serialize_bitfield_message_to_buf");
        let msg = Message::Bitfield(ByteBuf(g.get_chunks()?.get_have_pieces().as_bytes()));
        let len = msg.serialize(buf, &Default::default)?;
        trace!("sending: {:?}, length={}", &msg, len);
        Ok(len)
    }

    fn on_handshake(&self, handshake: Handshake, ckind: ConnectionKind) -> anyhow::Result<()> {
        self.state.set_peer_live(self.addr, handshake, ckind);
        Ok(())
    }

    fn on_uploaded_bytes(&self, bytes: u32) {
        self.counters
            .uploaded_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
        self.state
            .stats
            .uploaded_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
        self.state
            .session_stats
            .counters
            .uploaded_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn read_chunk(&self, chunk: &ChunkInfo, buf: &mut [u8]) -> anyhow::Result<()> {
        self.state.file_ops().read_chunk(self.addr, chunk, buf)
    }

    fn upload_slot_semaphore(&self) -> Option<Arc<tokio::sync::Semaphore>> {
        Some(self.state.upload_slot_semaphore.clone())
    }

    fn dht_port(&self) -> Option<u16> {
        self.state
            .shared
            .session
            .upgrade()
            .and_then(|session| session.get_dht().map(|dht| dht.listen_addr().port()))
    }

    fn on_extended_handshake(&self, hs: &ExtendedHandshake<ByteBuf>) -> anyhow::Result<()> {
        if !self.state.metadata.info.info().private && hs.m.ut_pex.is_some() {
            spawn_with_cancel(
                debug_span!(
                    parent: self.state.shared.span.clone(),
                    "sending_pex_to_peer",
                    peer = ?self.addr,
                ),
                format!(
                    "[{}][addr={}]sending_pex_to_peer",
                    self.state.shared.id, self.addr
                ),
                self.cancel_token.clone(),
                self.state
                    .clone()
                    .task_send_pex_to_peer(self.addr, self.tx.clone()),
            );
        }
        // Lets update outgoing Socket address for incoming connection
        if self.incoming
            && let Some(port) = hs.port()
        {
            let peer_ip = hs.ip_addr().unwrap_or(self.addr.ip());
            let outgoing_addr = SocketAddr::new(peer_ip, port);
            self.state
                .peers
                .with_peer_mut(self.addr, "update outgoing addr", |peer| {
                    peer.outgoing_address = Some(outgoing_addr)
                });
        }

        Ok(())
    }

    fn should_send_bitfield(&self) -> bool {
        if self.state.torrent().options.disable_upload() {
            return false;
        }

        self.state.get_approx_have_bytes() > 0
    }

    fn should_transmit_have(&self, id: librtbit_core::lengths::ValidPieceIndex) -> bool {
        if self.state.shared.options.disable_upload() {
            return false;
        }
        let have = self
            .state
            .peers
            .with_live(self.addr, |l| {
                l.bitfield.get(id.get_usize()).map(|p| *p).unwrap_or(true)
            })
            .unwrap_or(true);
        !have
    }

    fn update_my_extended_handshake(
        &self,
        handshake: &mut ExtendedHandshake<ByteBuf>,
    ) -> anyhow::Result<()> {
        apply_extended_handshake_privacy(
            handshake,
            self.state.metadata.info.info().private,
            self.state.metadata.info_bytes.len(),
        );
        Ok(())
    }
}

/// Populate the privacy-sensitive fields of our outgoing extended handshake.
///
/// Private torrents (BEP-27) must not use the `ut_metadata`, `ut_pex` or
/// `ut_holepunch` extensions, so for them we neither advertise those extension
/// ids nor expose `metadata_size` (advertising a size signals ut_metadata
/// support). Public torrents advertise `metadata_size` so peers can fetch the
/// info dict via ut_metadata. Incoming messages for these extensions are
/// additionally rejected at the message-handling layer.
fn apply_extended_handshake_privacy(
    handshake: &mut ExtendedHandshake<ByteBuf>,
    private: bool,
    info_bytes_len: usize,
) {
    if private {
        handshake.m.ut_metadata = None;
        handshake.m.ut_pex = None;
        handshake.m.ut_holepunch = None;
        return;
    }
    if info_bytes_len != 0
        && let Ok(len) = u32::try_from(info_bytes_len)
    {
        handshake.metadata_size = Some(len);
    }
}

impl PeerHandler {
    pub(crate) fn on_peer_died(self, error: Option<crate::Error>) -> crate::Result<()> {
        let peers = &self.state.peers;
        let handle = self.addr;
        let mut pe = match peers.states.get_mut(&handle) {
            Some(peer) => TimedExistence::new(peer, "on_peer_died"),
            None => {
                warn!(
                    id = self.state.shared.id,
                    info_hash = ?self.state.shared.info_hash,
                    addr=?handle,
                    "bug: peer not found in table. Forgetting it forever"
                );
                return Ok(());
            }
        };
        let prev = pe.value_mut().take_state(peers);

        match prev {
            PeerState::Connecting(_) => {}
            PeerState::Live(live) => {
                let mut g = self.state.lock_write("mark_chunk_requests_canceled");

                // Release all pieces owned by this peer (fixes the bug where pieces
                // could be in both queue_pieces AND inflight_pieces after peer death)
                let released = g.get_pieces_mut()?.release_pieces_owned_by(self.addr);
                if released > 0 {
                    trace!(
                        "peer dead, released {} in-flight pieces back to queue",
                        released
                    );
                }

                // Also handle any chunk-level inflight requests
                let had_inflight = !live.inflight_requests.is_empty();
                for req in live.inflight_requests {
                    trace!(
                        "peer dead, marking chunk request cancelled, index={}, chunk={}",
                        req.piece_index.get(),
                        req.chunk_index
                    );
                }

                if released > 0 || had_inflight {
                    self.state.new_pieces_notify.notify_waiters();
                }
            }
            PeerState::NotNeeded => {
                // Restore it as std::mem::take() replaced it above.
                pe.value_mut().set_state(PeerState::NotNeeded, peers);
                return Ok(());
            }
            s @ PeerState::Queued | s @ PeerState::Dead => {
                warn!(
                    id = self.state.shared.id,
                    info_hash = ?self.state.shared.info_hash,
                    addr = ?handle,
                    "bug: peer was in a wrong state {s:?}, ignoring it forever"
                );
                // Prevent deadlocks.
                drop(pe);
                self.state.peers.drop_peer(handle);
                return Ok(());
            }
        };

        let _error = match error {
            Some(e) => e,
            None => {
                trace!("peer died without errors, not re-queueing");
                pe.value_mut().set_state(PeerState::NotNeeded, peers);
                return Ok(());
            }
        };

        self.counters.errors.fetch_add(1, Ordering::Relaxed);

        if self.state.is_finished_and_no_active_streams() {
            debug!("torrent finished, not re-queueing");
            pe.value_mut().set_state(PeerState::NotNeeded, peers);
            return Ok(());
        }

        pe.value_mut().set_state(PeerState::Dead, peers);

        if self.incoming {
            // do not retry incoming peers
            debug!(
                peer = handle.to_string(),
                "incoming peer died, not re-queueing"
            );
            return Ok(());
        }

        let backoff = pe.value_mut().stats.backoff.next();

        // Prevent deadlocks.
        drop(pe);

        // When backoff is exhausted, instead of permanently dropping the peer,
        // reset its backoff and schedule a long retry. This ensures peers can be
        // retried when re-discovery triggers (e.g., after a network disruption).
        let dur = match backoff {
            Some(dur) => dur,
            None => {
                debug!(
                    "backoff exhausted, resetting with long retry interval ({}s)",
                    super::FROZEN_PEER_RETRY_INTERVAL.as_secs()
                );
                let reset_ok = peers
                    .with_peer_mut(handle, "reset_backoff", |peer| {
                        peer.stats.reset_backoff();
                    })
                    .is_some();
                if !reset_ok {
                    return Ok(());
                }
                super::FROZEN_PEER_RETRY_INTERVAL
            }
        };

        if cfg!(feature = "_disable_reconnect_test") {
            return Ok(());
        }
        self.state.clone().spawn(
            debug_span!(
                parent: self.state.shared.span.clone(),
                "wait_for_peer",
                peer = ?handle,
                duration = format!("{dur:?}")
            ),
            format!("[{}][addr={}]wait_for_peer", self.state.shared.id, handle),
            async move {
                trace!("waiting to reconnect again");
                tokio::time::sleep(dur).await;
                trace!("finished waiting");
                let should_requeue = self
                    .state
                    .peers
                    .with_peer_mut(handle, "dead_to_queued", |peer| {
                        match peer.get_state() {
                            PeerState::Dead => {
                                peer.set_state(PeerState::Queued, &self.state.peers);
                                true
                            }
                            // Peer reconnected (e.g. via incoming connection) while we were
                            // waiting. No need to queue - it's already connected or queued.
                            PeerState::Live(_) | PeerState::Connecting(_) | PeerState::Queued => {
                                trace!(
                                    state = peer.get_state().name(),
                                    "peer is no longer dead, skipping requeue"
                                );
                                false
                            }
                            // Don't need this peer anymore.
                            PeerState::NotNeeded => false,
                        }
                    })
                    .unwrap_or(false);
                if should_requeue {
                    // try_send: drop the peer if queue is full
                    let _ = self.state.peer_queue_tx.try_send(handle);
                }
                Ok::<_, Error>(())
            },
        );
        Ok(())
    }

    /// Acquire a piece for this peer: try steal (10x) -> reserve -> steal (3x).
    ///
    /// Returns the piece index to download, or None if no pieces are available.
    fn acquire_next_piece(&self) -> crate::Result<Option<librtbit_core::lengths::ValidPieceIndex>> {
        // Steal info to process after releasing the peer lock
        let mut steal_info: Option<(SocketAddr, librtbit_core::lengths::ValidPieceIndex)> = None;

        let result = self
            .state
            .peers
            .with_live_mut(self.addr, "acquire_next_piece", |live| {
                if self.lock_read("i am choked").i_am_choked {
                    debug!("we are choked, can't acquire piece");
                    return Ok(None);
                }
                let mut g = self.state.lock_write("acquire_next_piece");

                let bf = &live.bitfield;
                // Extract references to disjoint fields
                let TorrentStateLocked {
                    pieces,
                    file_priorities,
                    ..
                } = &mut **g;
                let pieces = pieces.as_mut().ok_or(Error::ChunkTrackerEmpty)?;
                let result = pieces.acquire_piece(AcquireRequest {
                    peer: self.addr,
                    peer_avg_time: self.counters.average_piece_download_time(),
                    priority_pieces: self.state.streams.iter_next_pieces(&self.state.lengths),
                    file_priorities,
                    file_infos: &self.state.metadata.file_infos,
                    order: PieceOrder {
                        sequential: self
                            .state
                            .shared
                            .sequential_download
                            .load(Ordering::Relaxed),
                        first_last_piece_priority: self
                            .state
                            .shared
                            .first_last_piece_priority
                            .load(Ordering::Relaxed),
                    },
                    peer_has_piece: |p| bf.get(p.get() as usize).map(|v| *v) == Some(true),
                    can_steal: |p| {
                        self.state.per_piece_locks[p.get_usize()]
                            .try_write()
                            .is_some()
                    },
                });

                match result {
                    AcquireResult::Reserved(piece) => {
                        trace!("reserved piece {}", piece);
                        Ok(Some(piece))
                    }
                    AcquireResult::Stolen { piece, from_peer } => {
                        debug!("stole piece {} from {}", piece, from_peer);
                        // Store steal info to process after releasing peer lock to avoid deadlock
                        steal_info = Some((from_peer, piece));
                        Ok(Some(piece))
                    }
                    AcquireResult::NoneAvailable => Ok(None),
                }
            })
            .transpose()
            .map(|r| r.flatten());

        // Process steal notification outside the peer lock to avoid deadlock
        if let Some((from_peer, piece)) = steal_info {
            self.state.peers.on_steal(from_peer, self.addr, piece);
        }

        result
    }

    fn on_download_request(&self, request: Request) -> anyhow::Result<()> {
        if self.state.torrent().options.disable_upload() {
            anyhow::bail!("upload disabled, but peer requested a piece")
        }

        let piece_index = match self.state.lengths.validate_piece_index(request.index) {
            Some(p) => p,
            None => {
                anyhow::bail!(
                    "received {request:?}, but it is not a valid chunk request (piece index is invalid). Ignoring."
                );
            }
        };

        let chunk_info = match self.state.lengths.chunk_info_from_received_data(
            piece_index,
            request.begin,
            request.length,
        ) {
            Some(d) => d,
            None => {
                anyhow::bail!(
                    "received {request:?}, but it is not a valid chunk request (chunk data is invalid). Ignoring."
                );
            }
        };

        if !self
            .state
            .lock_read("is_chunk_ready_to_upload")
            .get_chunks()?
            .is_chunk_ready_to_upload(&chunk_info)
        {
            anyhow::bail!(
                "got request for a chunk that is not ready to upload. chunk {:?}",
                chunk_info
            );
        }

        self.state
            .ratelimit_upload_tx
            .send((self.tx.clone(), chunk_info))?;
        Ok(())
    }

    fn on_have(&self, have: u32) {
        self.state
            .peers
            .with_live_mut(self.addr, "on_have", |live| {
                // If bitfield wasn't allocated yet, let's do it. Some clients start empty so they never
                // send bitfields.
                if live.bitfield.is_empty() {
                    live.bitfield = make_piece_bitfield(&self.state.lengths);
                }
                match live.bitfield.get_mut(have as usize) {
                    Some(mut v) => *v = true,
                    None => {
                        warn!(
                            id = self.state.shared.id,
                            info_hash = ?self.state.shared.info_hash,
                            addr = ?self.addr,
                            "received have {} out of range",
                            have
                        );
                        return;
                    }
                };
                trace!("updated bitfield with have={}", have);
                if let Some(true) = live
                    .bitfield
                    .get(..self.state.lengths.total_pieces() as usize)
                    .map(|s| s.all())
                {
                    debug!("peer has full torrent");
                }
            });
        self.on_bitfield_notify.notify_waiters();
    }

    fn on_bitfield(&self, bitfield: ByteBufOwned) -> anyhow::Result<()> {
        if bitfield.as_ref().len() != self.state.lengths.piece_bitfield_bytes() {
            anyhow::bail!(
                "dropping peer as its bitfield has unexpected size. Got {}, expected {}",
                bitfield.as_ref().len(),
                self.state.lengths.piece_bitfield_bytes(),
            );
        }
        let bf = BF::from_boxed_slice(bitfield.0.to_vec().into_boxed_slice());
        if let Some(true) = bf
            .get(..self.state.lengths.total_pieces() as usize)
            .map(|s| s.all())
        {
            debug!("peer has full torrent");
        }
        self.state.peers.update_bitfield(self.addr, bf);
        self.on_bitfield_notify.notify_waiters();
        Ok(())
    }

    async fn wait_for_any_notify(&self, notify: &Notify, check: impl Fn() -> bool) {
        // To remove possibility of races, we first grab a token, then check
        // if we need it, and only if so, await.
        let notified = notify.notified();
        if check() {
            return;
        }
        notified.await;
    }

    async fn wait_for_bitfield(&self) {
        self.wait_for_any_notify(&self.on_bitfield_notify, || {
            self.state
                .peers
                .with_live(self.addr, |live| !live.bitfield.is_empty())
                .unwrap_or_default()
        })
        .await;
    }

    async fn wait_for_unchoke(&self) {
        self.wait_for_any_notify(&self.unchoke_notify, || {
            !self.lock_read("wait_for_unchoke:i_am_choked").i_am_choked
        })
        .await;
    }

    // The job of this is to request chunks and also to keep peer alive.
    // The moment this ends, the peer is disconnected.
    pub(crate) async fn task_peer_chunk_requester(&self) -> crate::Result<()> {
        let handle = self.addr;
        self.wait_for_bitfield().await;

        let mut update_interest = {
            let mut current = false;
            move |h: &PeerHandler, new_value: bool| -> crate::Result<()> {
                if new_value != current {
                    h.tx.send(if new_value {
                        WriterRequest::Message(Message::Interested)
                    } else {
                        WriterRequest::Message(Message::NotInterested)
                    })
                    .ok()
                    .ok_or(Error::PeerTaskDead)?;
                    current = new_value;
                }
                Ok(())
            }
        };

        loop {
            // If we have full torrent, we don't need to request more pieces.
            // However we might still need to seed them to the peer.
            if self.state.is_finished_and_no_active_streams() {
                update_interest(self, false)?;
                if self
                    .state
                    .peers
                    .is_peer_not_interested_and_has_full_torrent(
                        self.addr,
                        self.state.lengths.total_pieces() as usize,
                    )
                {
                    debug!("nothing left to do, neither of us is interested, disconnecting peer");
                    self.tx
                        .send(WriterRequest::Disconnect(Ok(())))
                        .ok()
                        .ok_or(Error::PeerTaskDead)?;
                    // wait until the receiver gets the message so that it doesn't finish with an error.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    return Ok(());
                } else {
                    // TODO: wait for a notification of interest, e.g. update of selected files or new streams or change
                    // in peer interest.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            }

            update_interest(self, true)?;
            aframe!(self.wait_for_unchoke()).await;

            // Acquire a piece using the strategy: try steal (10x) -> reserve -> steal (3x).
            let new_piece_notify = self.state.new_pieces_notify.notified();
            let next = match self.acquire_next_piece()? {
                Some(next) => next,
                None => {
                    debug!("no pieces to request");
                    match aframe!(tokio::time::timeout(
                        // Half of default rw timeout not to race with it.
                        Duration::from_secs(5),
                        new_piece_notify
                    ))
                    .await
                    {
                        Ok(()) => debug!("woken up, new pieces might be available"),
                        Err(_) => debug!("woken up by sleep timer"),
                    }
                    continue;
                }
            };

            for chunk in self.state.lengths.iter_chunk_infos(next) {
                let request = Request {
                    index: next.get(),
                    begin: chunk.offset,
                    length: chunk.size,
                };

                match self
                    .state
                    .peers
                    .with_live_mut(handle, "add chunk request", |live| {
                        live.inflight_requests.insert(chunk)
                    }) {
                    Some(true) => {}
                    Some(false) => {
                        // This request was already in-flight for this peer for this chunk.
                        // This might happen in theory, but not very likely.
                        //
                        // Example:
                        // someone stole a piece from us, and then died, the piece became "needed" again, and we reserved it
                        // all before the piece request was processed by us.
                        warn!(
                            id = self.state.shared.id,
                            info_hash = ?self.state.shared.info_hash,
                            addr = ?self.addr,
                            "we already requested {:?} previously",
                            chunk
                        );
                        continue;
                    }
                    // peer died
                    None => return Ok(()),
                };

                let Some(request_length) = NonZeroU32::new(request.length) else {
                    warn!(
                        addr = ?self.addr,
                        "peer sent request with zero length, ignoring"
                    );
                    continue;
                };

                self.state
                    .ratelimits
                    .prepare_for_download(request_length)
                    .await?;

                if let Some(session) = self.state.torrent().session.upgrade() {
                    session
                        .ratelimits
                        .prepare_for_download(request_length)
                        .await?;
                }

                loop {
                    match aframe!(tokio::time::timeout(
                        Duration::from_secs(5),
                        aframe!(self.requests_sem.acquire())
                    ))
                    .await
                    {
                        Ok(acq) => break acq?.forget(),
                        Err(_) => continue,
                    };
                }

                if self
                    .tx
                    .send(WriterRequest::Message(Message::Request(request)))
                    .is_err()
                {
                    return Ok(());
                }
            }
        }
    }

    fn on_i_am_choked(&self) {
        self.lock_write("i_am_choked = true").i_am_choked = true;
    }

    fn on_peer_interested(&self) {
        trace!("peer is interested");
        self.state.peers.mark_peer_interested(self.addr, true);
    }

    fn on_i_am_unchoked(&self) {
        trace!("we are unchoked");
        self.lock_write("i_am_choked = false").i_am_choked = false;
        self.unchoke_notify.notify_waiters();
        // 128 should be more than enough to maintain 100mbps
        // for a single peer that has 100ms ping
        // https://www.desmos.com/calculator/x3szur87ps
        self.requests_sem.add_permits(128);
    }

    async fn on_received_piece(&self, piece: Piece<ByteBuf<'_>>) -> anyhow::Result<()> {
        let piece_index = self
            .state
            .lengths
            .validate_piece_index(piece.index)
            .with_context(|| format!("peer sent an invalid piece {}", piece.index))?;
        let chunk_info = match self.state.lengths.chunk_info_from_received_data(
            piece_index,
            piece.begin,
            piece.len().try_into().context("bug")?,
        ) {
            Some(i) => i,
            None => {
                anyhow::bail!("peer sent us an invalid piece {:?}", piece,);
            }
        };

        self.requests_sem.add_permits(1);

        // Peer chunk/byte counters.
        self.counters
            .fetched_bytes
            .fetch_add(piece.len() as u64, Ordering::Relaxed);
        self.counters.fetched_chunks.fetch_add(1, Ordering::Relaxed);

        self.state
            .peers
            .with_live_mut(self.addr, "inflight_requests.remove", |h| {
                if !h.inflight_requests.remove(&chunk_info) {
                    anyhow::bail!(
                        "peer sent us a piece we did not ask. Requested pieces: {:?}. Got: {:?}",
                        h.inflight_requests,
                        piece,
                    );
                }
                Ok(())
            })
            .context("peer not found")??;

        // This one is used to calculate download speed.
        self.state
            .stats
            .fetched_bytes
            .fetch_add(piece.len() as u64, Ordering::Relaxed);
        self.state
            .session_stats
            .counters
            .fetched_bytes
            .fetch_add(piece.len() as u64, Ordering::Relaxed);

        fn write_to_disk(
            state: &TorrentStateLive,
            addr: PeerHandle,
            counters: &AtomicPeerCounters,
            piece: &Piece<ByteBuf<'_>>,
            chunk_info: &ChunkInfo,
        ) -> anyhow::Result<()> {
            let index = piece.index;

            // If someone stole the piece by now, ignore it.
            // However if they didn't, don't let them steal it while we are writing.
            // So that by the time we are done writing AND if it was the last piece,
            // we can actually checksum etc.
            // Otherwise it might get into some weird state.
            let ppl_guard = {
                let g = state.lock_read("check_steal");

                let ppl = state
                    .per_piece_locks
                    .get(piece.index as usize)
                    .map(|l| l.read());

                match g.get_pieces()?.get_inflight(chunk_info.piece_index) {
                    Some(inflight) if inflight.peer == addr => {}
                    Some(inflight) => {
                        debug!(
                            "in-flight piece {} was stolen by {}, ignoring",
                            chunk_info.piece_index, inflight.peer
                        );
                        return Ok(());
                    }
                    None => {
                        debug!(
                            "in-flight piece {} not found. it was probably completed by someone else",
                            chunk_info.piece_index
                        );
                        return Ok(());
                    }
                };

                ppl
            };

            // While we hold per piece lock, noone can steal it.
            // So we can proceed writing knowing that the piece is ours now and will still be by the time
            // the write is finished.
            //

            if !cfg!(feature = "_disable_disk_write_net_benchmark") {
                match state.file_ops().write_chunk(addr, piece, chunk_info) {
                    Ok(()) => {}
                    Err(e) => {
                        error!(
                            id = state.shared.id,
                            info_hash = ?state.shared.info_hash,
                            "FATAL: error writing chunk to disk: {e:#}"
                        );
                        return state.on_fatal_error(e);
                    }
                };
            }

            let full_piece_download_time = {
                let mut g = state.lock_write("mark_chunk_downloaded");
                let chunk_marking_result = g.get_pieces_mut()?.mark_chunk_downloaded(piece);
                trace!(?piece, chunk_marking_result=?chunk_marking_result);

                match chunk_marking_result {
                    Some(ChunkMarkingResult::Completed) => {
                        trace!("piece={} done, will write and checksum", piece.index);
                        // Remove from inflight to prevent others from stealing it during hash check.
                        g.get_pieces_mut()?.take_inflight(chunk_info.piece_index)
                    }
                    Some(ChunkMarkingResult::PreviouslyCompleted) => {
                        // TODO: we might need to send cancellations here.
                        debug!("piece={} was done by someone else, ignoring", piece.index);
                        return Ok(());
                    }
                    Some(ChunkMarkingResult::NotCompleted) => None,
                    None => {
                        anyhow::bail!(
                            "bogus data received: {piece:?}, cannot map this to a chunk, dropping peer"
                        );
                    }
                }
            };

            // We don't care about per piece lock anymore, as it's removed from inflight pieces.
            // It shouldn't impact perf anyway, but dropping just in case.
            drop(ppl_guard);

            let full_piece_download_time = match full_piece_download_time {
                Some(t) => t,
                None => return Ok(()),
            };

            match state
                .file_ops()
                .check_piece(chunk_info.piece_index)
                .with_context(|| format!("error checking piece={index}"))?
            {
                true => {
                    {
                        let mut g = state.lock_write("mark_piece_downloaded");
                        g.get_pieces_mut()?
                            .mark_piece_hash_ok(chunk_info.piece_index);
                    }

                    // Global piece counters.
                    let piece_len = state.lengths.piece_length(chunk_info.piece_index) as u64;
                    state
                        .stats
                        .downloaded_and_checked_bytes
                        // This counter is used to compute "is_finished", so using
                        // stronger ordering.
                        .fetch_add(piece_len, Ordering::Release);
                    state
                        .stats
                        .downloaded_and_checked_pieces
                        // This counter is used to compute "is_finished", so using
                        // stronger ordering.
                        .fetch_add(1, Ordering::Release);
                    state
                        .stats
                        .have_bytes
                        .fetch_add(piece_len, Ordering::Relaxed);
                    #[allow(clippy::cast_possible_truncation)]
                    state.stats.total_piece_download_ms.fetch_add(
                        full_piece_download_time.as_millis() as u64,
                        Ordering::Relaxed,
                    );

                    // Per-peer piece counters.
                    counters.on_piece_completed(piece_len, full_piece_download_time);
                    state.peers.reset_peer_backoff(addr);

                    trace!(piece = index, "successfully downloaded and verified");

                    state.on_piece_completed(chunk_info.piece_index)?;

                    state.transmit_haves(chunk_info.piece_index);
                }
                false => {
                    warn!(
                        id = state.shared.id,
                        info_hash = ?state.shared.info_hash,
                        ?addr,
                        "checksum for piece={} did not validate. disconnecting peer.", index
                    );
                    state
                        .lock_write("mark_piece_broken")
                        .get_pieces_mut()?
                        .mark_piece_hash_failed(chunk_info.piece_index);
                    state.new_pieces_notify.notify_waiters();
                    anyhow::bail!("i am probably a bogus peer. dying.")
                }
            };
            Ok(())
        }

        self.state
            .shared
            .spawner
            .block_in_place_with_semaphore(|| {
                write_to_disk(&self.state, self.addr, &self.counters, &piece, &chunk_info)
            })
            .await
            .with_context(|| format!("error processing received chunk {chunk_info:?}"))?;

        Ok(())
    }

    fn send_metadata_piece(&self, piece_id: u32) -> anyhow::Result<()> {
        let data = &self.state.metadata.info_bytes;
        let metadata_size = data.len();
        if metadata_size == 0 {
            anyhow::bail!("peer requested for info metadata but we don't have it")
        }
        let total_pieces: usize = (metadata_size as u64)
            .div_ceil(CHUNK_SIZE as u64)
            .try_into()?;

        if piece_id as usize > total_pieces {
            bail!("piece out of bounds")
        }

        let offset = piece_id * CHUNK_SIZE;
        let end = (offset + CHUNK_SIZE).min(data.len().try_into()?);
        let total_size: u32 = data
            .len()
            .try_into()
            .context("can't send metadata: len doesn't fit into u32")?;
        let data = data.slice(offset as usize..end as usize);

        self.tx
            .send(WriterRequest::UtMetadata(UtMetadata::Data(
                UtMetadataData::from_bytes(piece_id, total_size, data.into()),
            )))
            .context("error sending UtMetadata: channel closed")?;
        Ok(())
    }

    fn on_pex_message(&self, msg: UtPex<ByteBuf<'_>>) {
        msg.dropped_peers()
            .chain(msg.added_peers())
            .for_each(|peer| {
                self.state
                    .add_peer_if_not_seen(peer.addr)
                    .map_err(|error| {
                        warn!(
                            id = self.state.shared.id,
                            info_hash = ?self.state.shared.info_hash,
                            ?peer,
                            "failed to add peer: {error:#}"
                        );
                        error
                    })
                    .ok();
            });
    }

    fn on_holepunch_message(&self, msg: HolepunchMessage) {
        if self.state.metadata.info.info().private {
            warn!(
                id = self.state.shared.id,
                info_hash = ?self.state.shared.info_hash,
                "received noncompliant holepunch message from {}, ignoring (private torrent)",
                self.addr
            );
            return;
        }

        match msg.msg_type {
            HolepunchMsgType::Rendezvous => {
                let target_addr = msg.addr;
                // Try to relay Connect to the target peer
                let relayed = self.state.peers.with_live(target_addr, |live| {
                    // Send Connect message to the target telling it about the initiator
                    let connect_to_target = HolepunchMessage {
                        msg_type: HolepunchMsgType::Connect,
                        addr: self.addr,
                        error_code: None,
                    };
                    let _ = live.tx.send(WriterRequest::UtHolepunch(connect_to_target));

                    // Send Connect message back to the initiator telling it about the target
                    let connect_to_initiator = HolepunchMessage {
                        msg_type: HolepunchMsgType::Connect,
                        addr: target_addr,
                        error_code: None,
                    };
                    let _ = self
                        .tx
                        .send(WriterRequest::UtHolepunch(connect_to_initiator));
                });

                if relayed.is_none() {
                    // Target peer is not connected, send Error back to initiator
                    let error_msg = HolepunchMessage {
                        msg_type: HolepunchMsgType::Error,
                        addr: target_addr,
                        error_code: Some(HolepunchErrorCode::NotConnected),
                    };
                    let _ = self.tx.send(WriterRequest::UtHolepunch(error_msg));
                }
            }
            HolepunchMsgType::Connect => {
                // A rendezvous peer is telling us to connect to this peer
                let peer_addr = msg.addr;
                self.state
                    .add_peer_if_not_seen(peer_addr)
                    .map_err(|error| {
                        warn!(
                            id = self.state.shared.id,
                            info_hash = ?self.state.shared.info_hash,
                            addr = %peer_addr,
                            "holepunch: failed to add peer: {error:#}"
                        );
                        error
                    })
                    .ok();
            }
            HolepunchMsgType::Error => {
                debug!(
                    id = self.state.shared.id,
                    info_hash = ?self.state.shared.info_hash,
                    addr = %msg.addr,
                    error_code = ?msg.error_code,
                    "holepunch error received, ignoring"
                );
            }
        }
    }

    fn lock_read(
        &self,
        reason: &'static str,
    ) -> TimedExistence<RwLockReadGuard<'_, PeerHandlerLocked>> {
        TimedExistence::new(timeit(reason, || self._locked.read()), reason)
    }
    fn lock_write(
        &self,
        reason: &'static str,
    ) -> TimedExistence<RwLockWriteGuard<'_, PeerHandlerLocked>> {
        TimedExistence::new(timeit(reason, || self._locked.write()), reason)
    }
}

#[cfg(test)]
mod tests {
    use super::apply_extended_handshake_privacy;
    use peer_binary_protocol::extended::handshake::ExtendedHandshake;

    #[test]
    fn private_torrent_strips_metadata_extensions() {
        let mut hs = ExtendedHandshake::new();
        // Sanity: the default (public) handshake advertises the extensions.
        assert!(hs.m.ut_metadata.is_some());
        assert!(hs.m.ut_pex.is_some());
        assert!(hs.m.ut_holepunch.is_some());

        apply_extended_handshake_privacy(&mut hs, true, 1234);

        assert_eq!(hs.m.ut_metadata, None);
        assert_eq!(hs.m.ut_pex, None);
        assert_eq!(hs.m.ut_holepunch, None);
        assert_eq!(hs.metadata_size, None);
    }

    #[test]
    fn public_torrent_advertises_metadata_size() {
        let mut hs = ExtendedHandshake::new();

        apply_extended_handshake_privacy(&mut hs, false, 4096);

        assert_eq!(hs.metadata_size, Some(4096));
        assert!(hs.m.ut_metadata.is_some());
        assert!(hs.m.ut_pex.is_some());
        assert!(hs.m.ut_holepunch.is_some());
    }
}

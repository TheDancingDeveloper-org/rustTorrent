// The main logic of rtbit is here - connecting to peers, reading and writing messages
// to them, tracking peer state etc.
//
// ## Architecture
// There are many tasks cooperating to download the torrent. Tasks communicate both with message passing
// and shared memory.
//
// ### Shared locked state
// Shared state is access by almost all actors through RwLocks.
//
// There's one source of truth (TorrentStateLocked) for which chunks we have, need, and what peers are we waiting them from.
//
// Peer states that are important to the outsiders (tasks other than manage_peer) are in a sharded hash-map (DashMap)
//
// ### Tasks (actors)
// Peer adder task:
// - spawns new peers as they become known. It pulls them from a queue. The queue is filled in by DHT and torrent trackers.
//   Also gets updated when peers are reconnecting after errors.
//
// Each peer has one main task "manage_peer". It's composed of 2 futures running as one task through tokio::select:
// - "manage_peer" - this talks to the peer over network and calls callbacks on PeerHandler. The callbacks are not async,
//   and are supposed to finish quickly (apart from writing to disk, which is accounted for as "spawn_blocking").
// - "peer_chunk_requester" - this continuously sends requests for chunks to the peer.
//   it may steal chunks/pieces from other peers.
//
// ## Peer lifecycle
// State transitions:
// - queued (initial state) -> connected
// - connected -> live
// - ANY STATE -> dead (on error)
// - ANY STATE -> not_needed (when we don't need to talk to the peer anymore)
//
// When the peer dies, it's rescheduled with exponential backoff.
//
// > NOTE: deadlock notice:
// > peers and stateLocked are behind 2 different locks.
// > if you lock them in different order, this may deadlock.
// >
// > so don't lock them both at the same time at all, or at the worst lock them in the
// > same order (peers one first, then the global one).

pub mod peer;
mod peer_handler;
pub mod peers;
pub mod stats;
mod tasks;

use std::{
    borrow::Cow,
    collections::HashSet,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Context;
use buffers::ByteBufOwned;
use librtbit_core::{
    hash_id::Id20,
    lengths::{ChunkInfo, Lengths, ValidPieceIndex},
    spawn_utils::spawn_with_cancel,
    speed_estimator::SpeedEstimator,
    torrent_metainfo::ValidatedTorrentMetaV1Info,
};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use peer_binary_protocol::Handshake;
use tokio::sync::{
    Notify, Semaphore,
    mpsc::{Sender, unbounded_channel},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, debug_span, error, info, trace, warn};

use crate::{
    Error,
    chunk_tracker::{ChunkTracker, HaveNeededSelected},
    file_ops::FileOps,
    limits::Limits,
    peer_connection::WriterRequest,
    piece_tracker::PieceTracker,
    session::CheckedIncomingConnection,
    session_stats::SessionStats,
    stream_connect::ConnectionKind,
    torrent_state::{peer::Peer, utils::atomic_inc},
    type_aliases::{BF, FilePriorities, FileStorage, PeerHandle},
};

use self::{
    peer::{
        PeerState,
        stats::snapshot::{PeerStatsFilter, PeerStatsSnapshot},
    },
    peers::PeerStates,
    stats::{atomic::AtomicStats, snapshot::StatsSnapshot},
};

use super::{
    ManagedTorrentShared, TorrentMetadata,
    paused::TorrentStatePaused,
    streaming::TorrentStreams,
    utils::{TimedExistence, timeit},
};

pub(crate) fn make_piece_bitfield(lengths: &Lengths) -> BF {
    BF::from_boxed_slice(vec![0; lengths.piece_bitfield_bytes()].into_boxed_slice())
}

pub(crate) struct TorrentStateLocked {
    // Coordinates piece state: what chunks we have, need, and what pieces are in-flight.
    // If this is None, the torrent was paused, and this live state is useless, and needs to be dropped.
    pub(crate) pieces: Option<PieceTracker>,

    // The sorted file list in which order to download them.
    pub(crate) file_priorities: FilePriorities,

    // If this is None, then it was already used
    fatal_errors_tx: Option<tokio::sync::oneshot::Sender<anyhow::Error>>,

    unflushed_bitv_bytes: u64,
}

impl TorrentStateLocked {
    pub(crate) fn get_chunks(&self) -> crate::Result<&ChunkTracker> {
        self.pieces
            .as_ref()
            .map(|p| p.chunks())
            .ok_or(Error::ChunkTrackerEmpty)
    }

    pub(crate) fn get_pieces(&self) -> crate::Result<&PieceTracker> {
        self.pieces.as_ref().ok_or(Error::ChunkTrackerEmpty)
    }

    pub(crate) fn get_pieces_mut(&mut self) -> crate::Result<&mut PieceTracker> {
        self.pieces.as_mut().ok_or(Error::ChunkTrackerEmpty)
    }

    fn try_flush_bitv(&mut self, shared: &ManagedTorrentShared, flush_async: bool) {
        if self.unflushed_bitv_bytes == 0 {
            return;
        }
        trace!("trying to flush bitfield");
        if let Some(Err(e)) = self
            .pieces
            .as_mut()
            .map(|pt| pt.flush_have_pieces(flush_async))
        {
            warn!(id=?shared.id, info_hash = ?shared.info_hash, "error flushing bitfield: {e:#}");
            // Don't reset unflushed_bitv_bytes on error — retry on next flush attempt
        } else {
            trace!("flushed bitfield");
            self.unflushed_bitv_bytes = 0;
        }
    }
}

const FLUSH_BITV_EVERY_BYTES: u64 = 16 * 1024 * 1024;

/// How often the dead peer pruning task runs.
const PEER_PRUNE_INTERVAL: Duration = Duration::from_secs(60);
/// Peers in Dead/NotNeeded state are removed from the map after this duration.
const PEER_PRUNE_RETENTION: Duration = Duration::from_secs(300);

pub enum AddIncomingPeerResult {
    Added,
    AlreadyActive,
    ConcurrencyLimitReached,
}

/// Minimum number of active (live + connecting) peers before triggering re-discovery.
const MIN_PEERS_THRESHOLD: u32 = 5;

/// Minimum interval between re-discovery attempts to prevent spamming.
const REDISCOVERY_COOLDOWN: Duration = Duration::from_secs(30);

/// How often the peer health monitor checks peer counts.
const PEER_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Long backoff duration assigned to peers whose backoff was exhausted,
/// so they are retried eventually rather than dropped permanently.
const FROZEN_PEER_RETRY_INTERVAL: Duration = Duration::from_secs(600);

pub struct TorrentStateLive {
    pub(crate) peers: PeerStates,
    pub(crate) shared: Arc<ManagedTorrentShared>,
    pub(crate) metadata: Arc<TorrentMetadata>,
    _locked: RwLock<TorrentStateLocked>,

    pub(crate) files: FileStorage,

    pub(crate) per_piece_locks: Vec<RwLock<()>>,

    pub(crate) stats: AtomicStats,
    pub(crate) lengths: Lengths,

    // Limits how many active (occupying network resources) peers there are at a moment in time.
    pub(crate) peer_semaphore: Arc<Semaphore>,

    /// Fixed BEP 3 upload slots. A peer remains choked until it owns a permit.
    pub(crate) upload_slot_semaphore: Arc<Semaphore>,

    // The queue for peer manager to connect to them.
    // Bounded to prevent memory exhaustion from DHT/PEX flooding.
    pub(crate) peer_queue_tx: Sender<SocketAddr>,

    pub(crate) finished_notify: Notify,
    pub(crate) new_pieces_notify: Notify,

    /// Notified when active peer count drops below threshold,
    /// signaling that new peer discovery should be triggered.
    pub(crate) rediscovery_notify: Notify,

    pub(crate) down_speed_estimator: SpeedEstimator,
    pub(crate) up_speed_estimator: SpeedEstimator,
    pub(crate) cancellation_token: CancellationToken,

    pub(crate) session_stats: Arc<SessionStats>,

    pub(crate) streams: Arc<TorrentStreams>,
    pub(crate) have_broadcast_tx: tokio::sync::broadcast::Sender<ValidPieceIndex>,

    pub(crate) ratelimit_upload_tx: tokio::sync::mpsc::UnboundedSender<(
        tokio::sync::mpsc::UnboundedSender<WriterRequest>,
        ChunkInfo,
    )>,
    pub(crate) ratelimits: Limits,
}

impl TorrentStateLive {
    pub(crate) fn new(
        paused: TorrentStatePaused,
        fatal_errors_tx: tokio::sync::oneshot::Sender<anyhow::Error>,
        cancellation_token: CancellationToken,
    ) -> anyhow::Result<Arc<Self>> {
        // Bound peer queue to prevent memory exhaustion from DHT/PEX peer flooding.
        // 10000 is generous — the peer_adder task drains this quickly.
        let (peer_queue_tx, peer_queue_rx) = tokio::sync::mpsc::channel(10000);
        let session = paused
            .shared
            .session
            .upgrade()
            .context("session is dead, cannot start torrent")?;
        let session_stats = session.stats.clone();
        let down_speed_estimator = SpeedEstimator::default();
        let up_speed_estimator = SpeedEstimator::default();

        let have_bytes = paused.chunk_tracker.get_hns().have_bytes;
        let lengths = *paused.chunk_tracker.get_lengths();

        // TODO: make it configurable
        let file_priorities = {
            let mut pri = (0..paused.metadata.file_infos.len()).collect::<Vec<usize>>();
            // sort by filename, cause many torrents have random sort order.
            pri.sort_unstable_by_key(|id| {
                paused
                    .metadata
                    .file_infos
                    .get(*id)
                    .map(|fi| fi.relative_filename.as_path())
            });
            pri
        };

        let (have_broadcast_tx, _) = tokio::sync::broadcast::channel(128);

        let (ratelimit_upload_tx, ratelimit_upload_rx) = tokio::sync::mpsc::unbounded_channel::<(
            tokio::sync::mpsc::UnboundedSender<WriterRequest>,
            ChunkInfo,
        )>();
        // Read the runtime override (seeded from options.ratelimits) so a
        // per-torrent limit set while paused takes effect on this live state.
        let ratelimits = Limits::new(*paused.shared.ratelimit_override.read());

        let state = Arc::new(TorrentStateLive {
            shared: paused.shared.clone(),
            metadata: paused.metadata.clone(),
            peers: PeerStates {
                session_stats: session_stats.peers.clone(),
                stats: Default::default(),
                states: Default::default(),
                live_outgoing_peers: Default::default(),
            },
            _locked: RwLock::new(TorrentStateLocked {
                pieces: Some(PieceTracker::new(paused.chunk_tracker)),
                file_priorities,
                fatal_errors_tx: Some(fatal_errors_tx),
                unflushed_bitv_bytes: 0,
            }),
            files: paused.files,
            stats: AtomicStats {
                have_bytes: AtomicU64::new(have_bytes),
                ..Default::default()
            },
            lengths,
            peer_semaphore: Arc::new(Semaphore::new(
                paused.shared.options.peer_limit.unwrap_or(200),
            )),
            upload_slot_semaphore: Arc::new(Semaphore::new(4)),
            new_pieces_notify: Notify::new(),
            peer_queue_tx,
            finished_notify: Notify::new(),
            rediscovery_notify: Notify::new(),
            down_speed_estimator,
            up_speed_estimator,
            cancellation_token,
            have_broadcast_tx,
            session_stats,
            streams: paused.streams,
            per_piece_locks: (0..lengths.total_pieces())
                .map(|_| RwLock::new(()))
                .collect(),
            ratelimit_upload_tx,
            ratelimits,
        });

        state.spawn(
            debug_span!(parent: state.shared.span.clone(), "speed_estimator_updater"),
            format!("[{}]speed_estimator_updater", state.shared.id),
            {
                let state = Arc::downgrade(&state);
                async move {
                    loop {
                        let state = match state.upgrade() {
                            Some(state) => state,
                            None => return Ok(()),
                        };
                        let now = Instant::now();
                        let stats = state.stats_snapshot();
                        let fetched = stats.fetched_bytes;
                        let remaining = state
                            .lock_read("get_remaining_bytes")
                            .get_chunks()?
                            .get_remaining_bytes();
                        state
                            .down_speed_estimator
                            .add_snapshot(fetched, Some(remaining), now);
                        state
                            .up_speed_estimator
                            .add_snapshot(stats.uploaded_bytes, None, now);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            },
        );

        state.spawn(
            debug_span!(parent: state.shared.span.clone(), "peer_adder"),
            format!("[{}]peer_adder", state.shared.id),
            state.clone().task_peer_adder(peer_queue_rx),
        );

        state.spawn(
            debug_span!(parent: state.shared.span.clone(), "upload_scheduler"),
            format!("[{}]upload_scheduler", state.shared.id),
            state.clone().task_upload_scheduler(ratelimit_upload_rx),
        );

        state.spawn(
            debug_span!(parent: state.shared.span.clone(), "dead_peer_pruner"),
            format!("[{}]dead_peer_pruner", state.shared.id),
            {
                let state = Arc::downgrade(&state);
                async move {
                    let mut interval = tokio::time::interval(PEER_PRUNE_INTERVAL);
                    loop {
                        interval.tick().await;
                        let state = match state.upgrade() {
                            Some(s) => s,
                            None => return Ok(()),
                        };
                        state.peers.prune_dead_peers(PEER_PRUNE_RETENTION);
                    }
                }
            },
        );

        state.spawn(
            debug_span!(parent: state.shared.span.clone(), "peer_health_monitor"),
            format!("[{}]peer_health_monitor", state.shared.id),
            state.clone().task_peer_health_monitor(),
        );

        if !state.shared.web_seed_urls.is_empty() {
            info!(urls = ?state.shared.web_seed_urls, "torrent has webseed URLs");
        }

        Ok(state)
    }

    #[track_caller]
    pub(crate) fn spawn(
        &self,
        span: tracing::Span,
        name: impl Into<Cow<'static, str>>,
        fut: impl std::future::Future<Output = crate::Result<()>> + Send + 'static,
    ) {
        spawn_with_cancel(span, name, self.cancellation_token.clone(), fut);
    }

    pub fn down_speed_estimator(&self) -> &SpeedEstimator {
        &self.down_speed_estimator
    }

    pub fn up_speed_estimator(&self) -> &SpeedEstimator {
        &self.up_speed_estimator
    }

    pub(crate) fn add_incoming_peer(
        self: &Arc<Self>,
        checked_peer: CheckedIncomingConnection,
    ) -> anyhow::Result<AddIncomingPeerResult> {
        use dashmap::mapref::entry::Entry;
        let (tx, rx) = unbounded_channel();
        let permit = match self.peer_semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                debug!("limit of live peers reached, dropping incoming peer");
                self.peers.with_peer(checked_peer.addr, |p| {
                    atomic_inc(&p.stats.counters.incoming_connections);
                });
                return Ok(AddIncomingPeerResult::ConcurrencyLimitReached);
            }
        };

        let counters = match self.peers.states.entry(checked_peer.addr) {
            Entry::Occupied(mut occ) => {
                let peer = occ.get_mut();
                if let Err(e) = peer.incoming_connection(
                    checked_peer.handshake.peer_id,
                    tx.clone(),
                    &self.peers,
                    checked_peer.kind,
                ) {
                    match e {
                        peer::IncomingConnectionResult::AlreadyActive => {
                            debug!(
                                addr = %checked_peer.addr,
                                kind = %checked_peer.kind,
                                "peer already active, ignoring incoming connection"
                            );
                            return Ok(AddIncomingPeerResult::AlreadyActive);
                        }
                    }
                }
                peer.stats.counters.clone()
            }
            Entry::Vacant(vac) => {
                atomic_inc(&self.peers.stats.seen);
                let peer = Peer::new_live_for_incoming_connection(
                    *vac.key(),
                    checked_peer.handshake.peer_id,
                    tx.clone(),
                    &self.peers,
                    checked_peer.kind,
                );
                let counters = peer.stats.counters.clone();
                vac.insert(peer);
                counters
            }
        };
        atomic_inc(&counters.incoming_connections);

        self.spawn(
            debug_span!(
                parent: self.shared.span.clone(),
                "manage_incoming_peer",
                addr = %checked_peer.addr
            ),
            format!(
                "[{}][addr={}]manage_incoming_peer",
                self.shared.id, checked_peer.addr
            ),
            aframe!(
                self.clone()
                    .task_manage_incoming_peer(checked_peer, counters, tx, rx, permit)
            ),
        );
        Ok(AddIncomingPeerResult::Added)
    }

    pub fn torrent(&self) -> &ManagedTorrentShared {
        &self.shared
    }

    pub fn info(&self) -> &ValidatedTorrentMetaV1Info<ByteBufOwned> {
        &self.metadata.info
    }
    pub fn info_hash(&self) -> Id20 {
        self.shared.info_hash
    }
    pub fn peer_id(&self) -> Id20 {
        self.shared.peer_id
    }
    pub(crate) fn file_ops(&self) -> FileOps<'_> {
        FileOps::new(&self.metadata.info, &*self.files, &self.metadata.file_infos)
    }

    pub(crate) fn lock_read(
        &self,
        reason: &'static str,
    ) -> TimedExistence<RwLockReadGuard<'_, TorrentStateLocked>> {
        TimedExistence::new(timeit(reason, || self._locked.read()), reason)
    }
    pub(crate) fn lock_write(
        &self,
        reason: &'static str,
    ) -> TimedExistence<RwLockWriteGuard<'_, TorrentStateLocked>> {
        TimedExistence::new(timeit(reason, || self._locked.write()), reason)
    }

    fn set_peer_live(&self, handle: PeerHandle, h: Handshake, connection_kind: ConnectionKind) {
        self.peers.with_peer_mut(handle, "set_peer_live", |p| {
            p.connecting_to_live(h.peer_id, &self.peers, connection_kind);
        });
    }

    pub fn get_uploaded_bytes(&self) -> u64 {
        self.stats.uploaded_bytes.load(Ordering::Relaxed)
    }
    pub fn get_downloaded_bytes(&self) -> u64 {
        self.stats
            .downloaded_and_checked_bytes
            .load(Ordering::Acquire)
    }

    pub fn get_approx_have_bytes(&self) -> u64 {
        self.stats.have_bytes.load(Ordering::Relaxed)
    }

    pub fn get_hns(&self) -> Option<HaveNeededSelected> {
        self.lock_read("get_hns")
            .get_chunks()
            .ok()
            .map(|c| *c.get_hns())
    }

    fn transmit_haves(&self, index: ValidPieceIndex) {
        let _ = self.have_broadcast_tx.send(index);
    }

    pub(crate) fn add_peer_if_not_seen(&self, addr: SocketAddr) -> crate::Result<bool> {
        match self.peers.add_if_not_seen(addr) {
            Some(handle) => handle,
            None => return Ok(false),
        };

        // try_send: drop the peer if queue is full (backpressure)
        let _ = self.peer_queue_tx.try_send(addr);
        Ok(true)
    }

    pub fn stats_snapshot(&self) -> StatsSnapshot {
        use Ordering::*;
        let downloaded_bytes = self.stats.downloaded_and_checked_bytes.load(Relaxed);
        let (connected_seeders, connected_leechers) = self
            .peers
            .seed_leecher_counts(self.lengths.total_pieces() as usize);
        StatsSnapshot {
            downloaded_and_checked_bytes: downloaded_bytes,
            downloaded_and_checked_pieces: self.stats.downloaded_and_checked_pieces.load(Relaxed),
            fetched_bytes: self.stats.fetched_bytes.load(Relaxed),
            uploaded_bytes: self.stats.uploaded_bytes.load(Relaxed),
            total_piece_download_ms: self.stats.total_piece_download_ms.load(Relaxed),
            peer_stats: self.peers.stats(),
            connected_seeders,
            connected_leechers,
        }
    }

    pub fn per_peer_stats_snapshot(&self, filter: PeerStatsFilter) -> PeerStatsSnapshot {
        PeerStatsSnapshot {
            peers: self
                .peers
                .states
                .iter()
                .filter(|e| filter.state.matches(e.value().get_state()))
                .map(|e| (e.key().to_string(), e.value().into()))
                .collect(),
        }
    }

    pub async fn wait_until_completed(&self) {
        if self.is_finished() {
            return;
        }
        self.finished_notify.notified().await;
    }

    pub fn pause(&self) -> anyhow::Result<TorrentStatePaused> {
        // Flush while the bitfield's flusher task is still alive. The bitfield
        // is moved into the paused state below and dropped later, possibly after
        // session shutdown has cancelled background tasks; a drop-time flush
        // then fails with "flusher task is dead" and the last pieces are lost.
        {
            let mut g = self.lock_write("pause_flush");
            g.try_flush_bitv(&self.shared, false);
        }
        self.cancellation_token.cancel();

        let mut g = self.lock_write("pause");

        // It should be impossible to make a fatal error after pausing.
        g.fatal_errors_tx.take();

        let piece_tracker = g
            .pieces
            .take()
            .context("bug: pausing already paused torrent")?;
        // into_chunks() will requeue any in-flight pieces
        let chunk_tracker = piece_tracker.into_chunks();

        Ok(TorrentStatePaused {
            shared: self.shared.clone(),
            metadata: self.metadata.clone(),
            files: self.files.take()?,
            chunk_tracker,
            streams: self.streams.clone(),
        })
    }

    pub(crate) fn on_fatal_error(&self, e: anyhow::Error) -> anyhow::Result<()> {
        let mut g = self.lock_write("fatal_error");
        let tx = g
            .fatal_errors_tx
            .take()
            .context("fatal_errors_tx already taken")?;
        let res = anyhow::anyhow!("fatal error: {e:?}");
        if tx.send(e).is_err() {
            error!(id=self.shared.id, info_hash=?self.shared.info_hash, "fatal error receiver is dead, cancelling torrent");
            self.cancellation_token.cancel();
        }
        Err(res)
    }

    pub(crate) fn update_only_files(&self, only_files: &HashSet<usize>) -> anyhow::Result<()> {
        let mut g = self.lock_write("update_only_files");
        let pt = g.get_pieces_mut()?;
        let hns = pt.update_only_files(&self.metadata.file_infos, only_files)?;
        if !hns.finished() {
            self.reconnect_all_not_needed_peers();
        }
        Ok(())
    }

    // If we have all selected pieces but not necessarily all pieces.
    pub(crate) fn is_finished(&self) -> bool {
        self.get_hns().map(|h| h.finished()).unwrap_or_default()
    }

    fn has_active_streams_unfinished_files(&self, state: &TorrentStateLocked) -> bool {
        let chunks = match state.get_chunks() {
            Ok(c) => c,
            Err(_) => return false,
        };
        self.streams
            .streamed_file_ids()
            .any(|file_id| !chunks.is_file_finished(&self.metadata.file_infos[file_id]))
    }

    // We might have the torrent "finished" i.e. no selected files. But if someone is streaming files despite
    // them being selected, we aren't fully "finished".
    pub(crate) fn is_finished_and_no_active_streams(&self) -> bool {
        self.is_finished()
            && !self.has_active_streams_unfinished_files(
                &self.lock_read("is_finished_and_dont_need_peers"),
            )
    }

    /// Give a finished file its final name. A no-op unless the file is still
    /// under a `.part` name; failures are logged and leave the file where it is.
    fn finalize_file(&self, file_id: usize, relative: &std::path::Path) {
        if let Err(e) = self.files.rename_file(file_id, relative) {
            debug!(
                id = self.shared.id,
                ?relative,
                "could not finalize file: {e:#}"
            );
        }
    }

    /// Finalize every file, for torrents that were already complete when restored.
    pub(crate) fn finalize_all_files(&self) {
        for (idx, fi) in self.metadata.file_infos.iter().enumerate() {
            if !fi.attrs.padding {
                self.finalize_file(idx, &fi.relative_filename);
            }
        }
    }

    pub(crate) fn on_piece_completed(&self, id: ValidPieceIndex) -> anyhow::Result<()> {
        if let Err(e) = self.files.on_piece_completed(id) {
            debug!(?id, "file storage errored in on_piece_completed(): {e:#}");
        }
        let mut g = self.lock_write("on_piece_completed");
        let locked = &mut **g;
        let pieces = locked.get_pieces_mut()?;

        // if we have all the pieces of the file, reopen it read only
        for (idx, file_info) in self
            .metadata
            .file_infos
            .iter()
            .enumerate()
            .skip_while(|(_, fi)| !fi.piece_range.contains(&id.get()))
            .take_while(|(_, fi)| fi.piece_range.contains(&id.get()))
        {
            let remaining = pieces.update_file_have_on_piece_completed(id, idx, file_info);
            if remaining == 0 {
                self.finalize_file(idx, &file_info.relative_filename);
            }
        }

        self.streams
            .wake_streams_on_piece_completed(id, self.metadata.lengths());

        locked.unflushed_bitv_bytes += self.metadata.lengths().piece_length(id) as u64;
        if locked.unflushed_bitv_bytes >= FLUSH_BITV_EVERY_BYTES {
            locked.try_flush_bitv(&self.shared, true)
        }

        let chunks = locked.get_chunks()?;
        if chunks.is_finished() {
            if chunks.get_selected_pieces()[id.get_usize()] {
                locked.try_flush_bitv(&self.shared, false);
                info!(id=self.shared.id, info_hash=?self.shared.info_hash, "torrent finished downloading");
            }
            self.finished_notify.notify_waiters();

            if !self.has_active_streams_unfinished_files(locked) {
                // prevent deadlocks.
                drop(g);
                // There is not point being connected to peers that have all the torrent, when
                // we don't need anything from them, and they don't need anything from us.
                self.disconnect_all_peers_that_have_full_torrent();
            }
        }
        Ok(())
    }

    fn disconnect_all_peers_that_have_full_torrent(&self) {
        for mut pe in self.peers.states.iter_mut() {
            if let PeerState::Live(l) = pe.value().get_state()
                && l.has_full_torrent(self.lengths.total_pieces() as usize)
            {
                let prev = pe.value_mut().set_not_needed(&self.peers);
                let _ = prev
                    .take_live_no_counters()
                    .unwrap()
                    .tx
                    .send(WriterRequest::Disconnect(Ok(())));
            }
        }
    }

    pub(crate) fn reconnect_all_not_needed_peers(&self) {
        self.peers
            .states
            .iter_mut()
            .filter_map(|mut p| p.value_mut().reconnect_not_needed_peer(&self.peers))
            .map(|socket_addr| self.peer_queue_tx.try_send(socket_addr))
            .take_while(|r| r.is_ok())
            .last();
    }

    /// Returns the number of active peers (live + connecting).
    pub(crate) fn get_active_peer_count(&self) -> u32 {
        let stats = self.peers.stats();
        stats.live + stats.connecting
    }

    /// Reset backoff timers for all dead peers and re-queue them.
    /// This is called during re-discovery to give dead peers another chance.
    pub(crate) fn requeue_dead_peers(&self) {
        let mut requeued = 0u32;
        for mut entry in self.peers.states.iter_mut() {
            let peer = entry.value_mut();
            if matches!(peer.get_state(), PeerState::Dead) {
                peer.stats.reset_backoff();
                peer.set_state(PeerState::Queued, &self.peers);
                let addr = peer.addr;
                // Don't hold the lock while sending
                drop(entry);
                if self.peer_queue_tx.try_send(addr).is_err() {
                    break;
                }
                requeued += 1;
            }
        }
        if requeued > 0 {
            debug!(
                id = self.shared.id,
                requeued, "re-queued dead peers during rediscovery"
            );
        }
    }
}

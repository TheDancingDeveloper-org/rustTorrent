pub mod initializing;
pub mod live;
pub mod paused;
pub mod stats;
mod streaming;
pub mod utils;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context;
use anyhow::bail;
use arc_swap::ArcSwapOption;
use buffers::ByteBufOwned;
use bytes::Bytes;
use futures::FutureExt;
use futures::future::BoxFuture;
use librtbit_core::hash_id::Id20;
use librtbit_core::lengths::Lengths;

use librtbit_core::spawn_utils::spawn_with_cancel;
use librtbit_core::torrent_metainfo::ValidatedTorrentMetaV1Info;
pub use live::*;
use parking_lot::RwLock;

use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use tracing::debug_span;
use tracing::trace;
use tracing::warn;

use crate::Session;
use crate::chunk_tracker::ChunkTracker;
use crate::file_info::FileInfo;
use crate::limits::LimitsConfig;
use crate::session::TorrentId;
use crate::spawn_utils::BlockingSpawner;
use crate::storage::BoxStorageFactory;
use crate::stream_connect::StreamConnector;
use crate::torrent_state::stats::LiveStats;
use crate::type_aliases::FileInfos;
use crate::type_aliases::PeerStream;

use initializing::TorrentStateInitializing;

use self::paused::TorrentStatePaused;
pub use self::stats::{TorrentStats, TorrentStatsState};
pub use self::streaming::FileStream;

// State machine transitions.
//
// - error -> initializing
// - initializing -> paused
// - paused -> live
// - live -> paused
//
// - initializing -> error
// - live -> error
pub enum ManagedTorrentState {
    Initializing(Arc<TorrentStateInitializing>),
    Paused(TorrentStatePaused),
    Live(Arc<TorrentStateLive>),
    Error(anyhow::Error),

    // This is used when swapping between states, outside world should never see it.
    None,
}

impl ManagedTorrentState {
    pub fn name(&self) -> &'static str {
        match self {
            ManagedTorrentState::Initializing(_) => "initializing",
            ManagedTorrentState::Paused(_) => "paused",
            ManagedTorrentState::Live(_) => "live",
            ManagedTorrentState::Error(_) => "error",
            ManagedTorrentState::None => "<invalid: none>",
        }
    }

    fn assert_paused(self) -> TorrentStatePaused {
        match self {
            Self::Paused(paused) => paused,
            _ => panic!("Expected paused state"),
        }
    }

    pub(crate) fn take(&mut self) -> Self {
        std::mem::replace(self, Self::None)
    }
}

/// The [`TorrentStats::error`] flag on a stopped torrent whose payload is
/// missing or short on read-only storage.
pub const READ_ONLY_INCOMPLETE_PAYLOAD: &str =
    "payload is missing or incomplete on read-only storage";

pub(crate) struct ManagedTorrentLocked {
    // The torrent might not be in "paused" state technically,
    // but the intention might be for it to stay paused.
    //
    // This should change only on "unpause".
    pub(crate) paused: bool,
    pub(crate) state: ManagedTorrentState,
    pub(crate) only_files: Option<Vec<usize>>,
}

#[derive(Default)]
pub(crate) struct ManagedTorrentOptions {
    pub force_tracker_interval: Option<Duration>,
    pub peer_connect_timeout: Option<Duration>,
    pub peer_read_write_timeout: Option<Duration>,
    pub allow_overwrite: bool,
    pub output_folder: PathBuf,
    pub output_folder_root: Option<PathBuf>,
    pub initial_peers: Vec<SocketAddr>,
    pub peer_limit: Option<usize>,
    /// Keep files under a `.part` name until they finish downloading.
    pub part_files: bool,
    #[cfg(feature = "disable-upload")]
    pub _disable_upload: bool,
}

impl ManagedTorrentOptions {
    #[cfg(feature = "disable-upload")]
    pub fn disable_upload(&self) -> bool {
        self._disable_upload
    }

    #[cfg(not(feature = "disable-upload"))]
    pub const fn disable_upload(&self) -> bool {
        false
    }
}

// Torrent bencodee "info" + some precomputed fields based on it for frequent access.
pub struct TorrentMetadata {
    pub info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
    pub torrent_bytes: Bytes,
    pub info_bytes: Bytes,
    pub file_infos: FileInfos,
}

impl TorrentMetadata {
    pub(crate) fn new(
        info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
        torrent_bytes: Bytes,
        info_bytes: Bytes,
    ) -> anyhow::Result<Self> {
        let file_infos = info
            .iter_file_details_ext()
            .map(|fd| {
                Ok::<_, anyhow::Error>(FileInfo {
                    relative_filename: fd.details.filename.to_pathbuf(),
                    offset_in_torrent: fd.offset,
                    piece_range: fd.pieces,
                    len: fd.details.len,
                    attrs: fd.details.attrs(),
                })
            })
            .collect::<anyhow::Result<Vec<FileInfo>>>()?;

        Ok(Self {
            info,
            torrent_bytes,
            info_bytes,
            file_infos,
        })
    }

    pub fn lengths(&self) -> &Lengths {
        self.info.lengths()
    }

    /// Return a copy of this metadata with the `relative_filename` of the given
    /// files replaced. `renames` is a list of `(file_id, new_relative_path)`.
    /// Only paths change — offsets, lengths and piece ranges are preserved, so
    /// the chunk tracker stays valid.
    pub(crate) fn with_renamed_files(&self, renames: &[(usize, std::path::PathBuf)]) -> Self {
        let mut file_infos = self.file_infos.clone();
        for (file_id, new_path) in renames {
            if let Some(fi) = file_infos.get_mut(*file_id) {
                fi.relative_filename = new_path.clone();
            }
        }
        Self {
            info: self.info.clone(),
            torrent_bytes: self.torrent_bytes.clone(),
            info_bytes: self.info_bytes.clone(),
            file_infos,
        }
    }
}

/// Validate a batch of file renames against the current file set: each id must
/// exist, each new path must be relative and free of `.`/`..`/root components,
/// and the resulting set of paths must stay unique (no collisions).
pub(crate) fn validate_renames(
    file_infos: &crate::type_aliases::FileInfos,
    renames: &[(usize, std::path::PathBuf)],
) -> anyhow::Result<()> {
    use std::path::Component;

    if renames.is_empty() {
        bail!("no files to rename");
    }

    let mut paths: Vec<std::path::PathBuf> = file_infos
        .iter()
        .map(|f| f.relative_filename.clone())
        .collect();

    for (file_id, new_relative) in renames {
        if *file_id >= file_infos.len() {
            bail!("no such file id {file_id}");
        }
        if new_relative.as_os_str().is_empty() {
            bail!("new path for file {file_id} is empty");
        }
        if new_relative.is_absolute() {
            bail!("new path {new_relative:?} must be relative to the torrent root");
        }
        for component in new_relative.components() {
            if !matches!(component, Component::Normal(_)) {
                bail!("new path {new_relative:?} must not contain '.', '..', or a root");
            }
        }
        paths[*file_id] = new_relative.clone();
    }

    let mut seen = std::collections::HashSet::with_capacity(paths.len());
    for path in &paths {
        if !seen.insert(path) {
            bail!("rename would create two files at the same path: {path:?}");
        }
    }

    Ok(())
}

/// Common information about torrent shared among all possible states.
///
// The reason it's not inlined into ManagedTorrent is to break the Arc cycle:
// ManagedTorrent contains the current torrent state, which in turn needs access to a bunch
// of stuff, but it shouldn't access the state.
pub struct ManagedTorrentShared {
    pub id: TorrentId,
    pub info_hash: Id20,
    pub(crate) spawner: BlockingSpawner,
    pub trackers: RwLock<HashSet<url::Url>>,
    pub peer_id: Id20,
    pub span: tracing::Span,
    pub(crate) options: ManagedTorrentOptions,
    pub(crate) connector: Arc<StreamConnector>,
    pub(crate) storage_factory: BoxStorageFactory,
    pub(crate) session: Weak<Session>,

    // "dn" from magnet link
    pub(crate) magnet_name: Option<String>,

    /// BEP 19: WebSeed URLs parsed from the torrent's url-list field.
    pub web_seed_urls: Vec<String>,

    /// Category assigned to this torrent.
    pub category: RwLock<Option<String>>,

    /// Tags assigned to this torrent. Each is also registered in the session's
    /// tag registry.
    pub tags: RwLock<std::collections::BTreeSet<String>>,

    /// Per-torrent speed limits, mutable at runtime. Seeded from the add-time
    /// options; read when (re)constructing the live limiter so a limit set
    /// while paused (or before a pause/unpause) survives. `None` bps = no limit.
    pub(crate) ratelimit_override: RwLock<LimitsConfig>,

    /// Display-name override set via the compat rename endpoint; takes
    /// precedence over the metadata/magnet name. `None` = use the torrent's
    /// own name. Not persisted across restarts.
    pub(crate) name_override: RwLock<Option<String>>,

    /// Current storage root, mutable at runtime so a whole-torrent relocation
    /// (`set_location`) can re-anchor storage. Seeded from
    /// `options.output_folder`; read by the storage factory's `create`. Not
    /// persisted across restarts.
    pub(crate) output_folder_override: RwLock<std::path::PathBuf>,

    /// Live per-tracker announce status (seeds/peers per tracker etc).
    pub tracker_status: Arc<tracker_comms::TrackerStatusRegistry>,

    /// Stable lifecycle timestamps used by compatibility APIs and persistence.
    pub added_on: u64,
    pub completion_on: AtomicU64,

    /// Per-torrent cancellation token, child of session token.
    /// Used to cancel init tasks, checking, etc. when the torrent is deleted/forgotten.
    /// Wrapped in a Mutex so it can be reset after a pause-during-init.
    pub(crate) cancellation_token: parking_lot::Mutex<CancellationToken>,

    /// Share limits (ratio and seeding time) checked by the session's policy pass.
    pub(crate) share_limits: RwLock<crate::session::ShareLimits>,

    /// `(uploaded_bytes, seen_at)` from the last policy pass that saw the upload
    /// counter move. Drives the inactive-seeding-time limit; not persisted.
    pub(crate) upload_activity: parking_lot::Mutex<(u64, u64)>,

    /// Set while the torrent waits for a queue slot. Queued torrents are paused.
    pub(crate) queued: AtomicBool,

    /// Position in the queue; lower runs first. Starts as the torrent id, so
    /// torrents queue in the order they were added until they are moved.
    pub(crate) queue_position: std::sync::atomic::AtomicI64,

    /// Set by force-start: the torrent runs regardless of the queue limits and
    /// does not take a queue slot. A user pause clears it.
    pub(crate) force_start: AtomicBool,

    /// Piece-picker mode: download the pieces of each file in ascending order.
    pub(crate) sequential_download: AtomicBool,

    /// Piece-picker mode: fetch the first and last piece of every file before
    /// any other queued piece.
    pub(crate) first_last_piece_priority: AtomicBool,

    /// Set while the low-disk guard holds this download. The guard resumes it.
    pub(crate) held_for_disk: AtomicBool,

    /// Set while the torrent is paused because it hit its share limit. Only a
    /// user resume clears it; the policy pass never resumes such a torrent.
    pub(crate) share_paused: AtomicBool,

    /// Bytes uploaded in earlier runs of this torrent. Each live run starts its
    /// own counter at zero, so a pause adds that run's total here.
    pub(crate) uploaded_bytes_base: std::sync::atomic::AtomicU64,
}

impl ManagedTorrentShared {
    /// Cancel the per-torrent token and replace it with a fresh child of the session token.
    /// Returns the old (now-cancelled) token. Used when pausing during initialization.
    pub(crate) fn cancel_and_reset_token(&self) {
        let mut guard = self.cancellation_token.lock();
        guard.cancel();
        if let Some(session) = self.session.upgrade() {
            *guard = session.cancellation_token().child_token();
        }
    }

    /// Cancel the per-torrent token permanently. Used when deleting/forgetting a torrent.
    pub(crate) fn cancel_token(&self) {
        self.cancellation_token.lock().cancel();
    }

    /// Create a child token from the per-torrent token for spawning tasks.
    pub(crate) fn child_token(&self) -> CancellationToken {
        self.cancellation_token.lock().child_token()
    }
}

pub struct ManagedTorrent {
    // Static torrent configuration that doesn't change.
    pub shared: Arc<ManagedTorrentShared>,
    // Torrent metadata. Maybe be None when the magnet is resolving (not implemented yet)
    pub metadata: ArcSwapOption<TorrentMetadata>,
    pub(crate) state_change_notify: Notify,
    pub(crate) locked: RwLock<ManagedTorrentLocked>,
}

impl ManagedTorrent {
    pub fn id(&self) -> TorrentId {
        self.shared.id
    }

    pub fn name(&self) -> Option<String> {
        if let Some(name) = self.shared.name_override.read().clone() {
            return Some(name);
        }
        if let Some(m) = &*self.metadata.load() {
            return m
                .info
                .name()
                .map(|n| n.into_owned())
                .or_else(|| self.shared.magnet_name.clone());
        }
        self.shared.magnet_name.clone()
    }

    /// The torrent's current storage root. Reflects a `set_location`
    /// relocation, unlike the add-time `options.output_folder`.
    pub fn output_folder(&self) -> std::path::PathBuf {
        self.shared.output_folder_override.read().clone()
    }

    /// Files whose path differs from the one in the torrent's info dict, as
    /// `(file_id, current_relative_path)`. Empty when nothing was renamed or
    /// the metadata is not resolved yet.
    pub fn renamed_files(&self) -> Vec<(usize, std::path::PathBuf)> {
        let metadata = self.metadata.load();
        let Some(metadata) = metadata.as_ref() else {
            return Vec::new();
        };
        metadata
            .info
            .iter_file_details_ext()
            .zip(metadata.file_infos.iter())
            .enumerate()
            .filter_map(|(file_id, (original, current))| {
                let original = original.details.filename.to_pathbuf();
                (original != current.relative_filename)
                    .then(|| (file_id, current.relative_filename.clone()))
            })
            .collect()
    }

    /// Set (or clear, with `None`) the display-name override for this torrent.
    /// An empty/whitespace name clears the override.
    pub fn set_display_name(&self, name: Option<String>) {
        let name = name.filter(|n| !n.trim().is_empty());
        *self.shared.name_override.write() = name;
    }

    /// Rename one or more files within the torrent. Only supported while the
    /// torrent is stopped (paused); moves the files on disk, keeps the storage
    /// handles valid, and updates the torrent metadata. All-or-nothing: a
    /// mid-move failure rolls back the renames already applied.
    ///
    /// `renames` is a list of `(file_id, new_relative_path)`. A rename whose
    /// destination already exists on disk is refused (never clobbers), which
    /// also makes a colliding/cyclic batch fail safely. Not persisted across
    /// restarts.
    pub fn rename_files(&self, renames: &[(usize, std::path::PathBuf)]) -> anyhow::Result<()> {
        let mut g = self.locked.write();
        let paused = match &mut g.state {
            ManagedTorrentState::Paused(paused) => paused,
            ManagedTorrentState::Live(_) => {
                bail!("torrent must be stopped before renaming files")
            }
            _ => bail!("torrent is not in a renamable state (must be stopped)"),
        };

        validate_renames(&paused.metadata.file_infos, renames)?;

        // Capture old relative paths (for rollback and directory pruning)
        // before we start mutating the storage.
        let old_paths: Vec<(usize, std::path::PathBuf)> = renames
            .iter()
            .map(|(file_id, _)| {
                (
                    *file_id,
                    paused.metadata.file_infos[*file_id]
                        .relative_filename
                        .clone(),
                )
            })
            .collect();

        for (applied, (file_id, new_relative)) in renames.iter().enumerate() {
            if let Err(error) = paused.files.rename_file(*file_id, new_relative) {
                // Roll back the renames already applied, in reverse order.
                for (rollback_id, old_relative) in old_paths[..applied].iter().rev() {
                    let _ = paused.files.rename_file(*rollback_id, old_relative);
                }
                return Err(error).context("failed to rename file on disk; rolled back");
            }
        }

        // Update the metadata (source of truth for deletion, display and any
        // future storage re-init).
        let new_metadata = Arc::new(paused.metadata.with_renamed_files(renames));
        paused.metadata = new_metadata.clone();
        self.metadata.store(Some(new_metadata));

        // Best-effort prune of source directories left empty by the move.
        for (_, old_relative) in &old_paths {
            if let Some(parent) = old_relative.parent()
                && !parent.as_os_str().is_empty()
            {
                let _ = paused.files.remove_directory_if_empty(parent);
            }
        }

        Ok(())
    }

    /// Relocate the torrent's files to a new root directory. Only supported
    /// while the torrent is stopped (paused): moves every file to the same
    /// relative path under `new_root`, keeps the storage handles valid, and
    /// re-anchors the storage root. Same-filesystem only — a cross-device move
    /// is refused (and rolled back) rather than copied. Not persisted across
    /// restarts.
    pub fn set_location(&self, new_root: std::path::PathBuf) -> anyhow::Result<()> {
        let mut g = self.locked.write();
        let paused = match &mut g.state {
            ManagedTorrentState::Paused(paused) => paused,
            ManagedTorrentState::Live(_) => {
                bail!("torrent must be stopped before changing its location")
            }
            _ => bail!("torrent is not in a relocatable state (must be stopped)"),
        };

        std::fs::create_dir_all(&new_root)
            .with_context(|| format!("error creating destination directory {new_root:?}"))?;
        paused.files.move_root(&new_root)?;
        *self.shared.output_folder_override.write() = new_root;
        Ok(())
    }

    pub fn shared(&self) -> &ManagedTorrentShared {
        &self.shared
    }

    pub fn with_metadata<R>(
        &self,
        mut f: impl FnMut(&Arc<TorrentMetadata>) -> R,
    ) -> anyhow::Result<R> {
        let r = self.metadata.load();
        let r = r.as_ref().context("torrent is not resolved")?;
        Ok(f(r))
    }

    pub fn info_hash(&self) -> Id20 {
        self.shared.info_hash
    }

    pub fn only_files(&self) -> Option<Vec<usize>> {
        self.locked.read().only_files.clone()
    }

    /// Current per-torrent speed limits (the runtime override). `None` bps means
    /// unlimited in that direction.
    pub fn rate_limits(&self) -> LimitsConfig {
        *self.shared.ratelimit_override.read()
    }

    /// Set the per-torrent download limit (`None` = unlimited). Persisted on the
    /// runtime override and applied to the live limiter immediately if live.
    pub fn set_download_limit(&self, bps: Option<std::num::NonZeroU32>) {
        self.shared.ratelimit_override.write().download_bps = bps;
        if let Some(live) = self.live() {
            live.ratelimits.set_download_bps(bps);
        }
    }

    /// Set the per-torrent upload limit (`None` = unlimited). See
    /// [`Self::set_download_limit`].
    pub fn set_upload_limit(&self, bps: Option<std::num::NonZeroU32>) {
        self.shared.ratelimit_override.write().upload_bps = bps;
        if let Some(live) = self.live() {
            live.ratelimits.set_upload_bps(bps);
        }
    }

    /// Force an immediate re-announce to trackers (and a fresh peer discovery
    /// from DHT/trackers). Returns false if the torrent is not live, in which
    /// case there is no announce loop to signal.
    pub fn reannounce(&self) -> bool {
        self.shared.tracker_status.request_reannounce();
        match self.live() {
            Some(live) => {
                live.rediscovery_notify.notify_one();
                true
            }
            None => false,
        }
    }

    pub fn with_state<R>(&self, f: impl FnOnce(&ManagedTorrentState) -> R) -> R {
        f(&self.locked.read().state)
    }

    pub(crate) fn with_state_mut<R>(&self, f: impl FnOnce(&mut ManagedTorrentState) -> R) -> R {
        f(&mut self.locked.write().state)
    }

    pub(crate) fn with_chunk_tracker<R>(
        &self,
        f: impl FnOnce(&ChunkTracker) -> R,
    ) -> anyhow::Result<R> {
        let g = self.locked.read();
        match &g.state {
            ManagedTorrentState::Paused(p) => Ok(f(&p.chunk_tracker)),
            ManagedTorrentState::Live(l) => Ok(f(l
                .lock_read("chunk_tracker")
                .get_chunks()
                .context("error getting chunks")?)),
            _ => bail!("no chunk tracker, torrent neither paused nor live"),
        }
    }

    /// Get the live state if the torrent is live.
    pub fn live(&self) -> Option<Arc<TorrentStateLive>> {
        let g = self.locked.read();
        match &g.state {
            ManagedTorrentState::Live(live) => Some(live.clone()),
            _ => None,
        }
    }

    // Get live torrent but wait a bit until it's initialized if it is
    pub(crate) async fn live_wait_initializing(
        &self,
        duration: Duration,
    ) -> Option<Arc<TorrentStateLive>> {
        timeout(duration, self.wait_until_initialized())
            .await
            .ok()?
            .ok()?;
        self.live()
    }

    fn stop_with_error(&self, error: anyhow::Error) {
        let mut g = self.locked.write();

        match g.state.take() {
            ManagedTorrentState::Live(live) => {
                if let Err(err) = live.pause() {
                    warn!(
                        id = self.shared.id,
                        info_hash = ?self.shared.info_hash,
                        "error pausing live torrent during fatal error handling: {err:#}",
                    );
                }
            }
            ManagedTorrentState::Error(e) => {
                warn!(
                    id = self.shared.id,
                    info_hash = ?self.shared.info_hash,
                    "bug: torrent already was in error state when trying to stop it. Previous error was: {e:#}",
                );
            }
            ManagedTorrentState::None => {
                warn!(
                    id = self.shared.id,
                    info_hash = ?self.shared.info_hash,
                    "bug: torrent encountered in None state during fatal error handling"
                )
            }
            _ => {}
        };

        self.state_change_notify.notify_waiters();

        g.state = ManagedTorrentState::Error(error)
    }

    /// peer_rx: the peer stream. If start_paused=false, must be set.
    /// start_paused: if set, the torrent will initialize (check file integrity), but will not start
    pub(crate) fn start(
        self: &Arc<Self>,
        peer_rx: Option<PeerStream>,
        start_paused: bool,
    ) -> anyhow::Result<()> {
        fn _start<'a>(
            t: &'a Arc<ManagedTorrent>,
            peer_rx: Option<PeerStream>,
            start_paused: bool,
            session: Arc<Session>,
            g: Option<parking_lot::RwLockWriteGuard<'a, ManagedTorrentLocked>>,
            token: CancellationToken,
        ) -> anyhow::Result<()> {
            let mut g = g.unwrap_or_else(|| t.locked.write());

            match &g.state {
                ManagedTorrentState::Live(_) => {
                    bail!("torrent is already live");
                }
                ManagedTorrentState::Initializing(init) => {
                    let init = init.clone();
                    let t = t.clone();
                    let span = t.shared().span.clone();
                    let token = token.clone();

                    spawn_with_cancel(
                        debug_span!(parent: span.clone(), "initialize_and_start"),
                        "initialize_and_start",
                        token.clone(),
                        async move {
                            let concurrent_init_semaphore =
                                session.concurrent_initialize_semaphore.clone();
                            init.queued_for_init
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            let _permit = concurrent_init_semaphore
                                .acquire()
                                .await
                                .context("bug: concurrent init semaphore was closed")?;
                            init.queued_for_init
                                .store(false, std::sync::atomic::Ordering::Relaxed);

                            match init.check().await {
                                Ok(paused) => {
                                    let mut g = t.locked.write();
                                    if let ManagedTorrentState::Initializing(_) = &g.state {
                                    } else {
                                        debug!(
                                            "no need to start torrent anymore, as it switched state from initializing"
                                        );
                                        return Ok(());
                                    }

                                    g.state = ManagedTorrentState::Paused(paused);
                                    t.state_change_notify.notify_waiters();
                                    _start(&t, peer_rx, start_paused, session, Some(g), token)
                                }
                                Err(err) => {
                                    let result = anyhow::anyhow!("{err:?}");
                                    t.locked.write().state = ManagedTorrentState::Error(err);
                                    t.state_change_notify.notify_waiters();
                                    Err(result)
                                }
                            }
                        },
                    );
                    Ok(())
                }
                ManagedTorrentState::Paused(paused) => {
                    if start_paused {
                        return Ok(());
                    }
                    // Read-only storage cannot write the missing pieces, so a
                    // torrent whose payload is missing or short stays stopped
                    // (and is flagged in its stats) rather than downloading.
                    if paused.files.is_read_only() && !paused.chunk_tracker.is_finished() {
                        g.paused = true;
                        debug!("payload incomplete on read-only storage; keeping torrent stopped");
                        return Ok(());
                    }
                    let paused = g.state.take().assert_paused();
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let live = TorrentStateLive::new(paused, tx, token.clone())?;
                    g.state = ManagedTorrentState::Live(live.clone());
                    t.state_change_notify.notify_waiters();

                    spawn_fatal_errors_receiver(t, rx, token);
                    if let Some(peer_rx) = peer_rx {
                        spawn_peer_adder(&live, peer_rx);
                    }
                    Ok(())
                }
                ManagedTorrentState::Error(_) => {
                    let metadata = t
                        .metadata
                        .load_full()
                        .context("torrent metadata was not loaded")?;
                    // Use false: fast resume validation already checks bitfield size,
                    // validates at least 1 piece per file, and probabilistically checks
                    // remaining pieces. Passing true would force a full disk recheck
                    // even for transient errors (network, tracker timeouts).
                    let initializing = Arc::new(TorrentStateInitializing::new(
                        t.shared.clone(),
                        metadata.clone(),
                        g.only_files.clone(),
                        t.shared
                            .storage_factory
                            .create_and_init(t.shared(), &metadata)?,
                        false,
                    ));
                    g.state = ManagedTorrentState::Initializing(initializing.clone());
                    t.state_change_notify.notify_waiters();

                    // Recurse.
                    _start(t, peer_rx, start_paused, session, Some(g), token)
                }
                ManagedTorrentState::None => bail!("bug: torrent is in empty state"),
            }
        }

        let session = self
            .shared
            .session
            .upgrade()
            .context("session is dead, cannot start torrent")?;
        let mut g = self.locked.write();
        g.paused = start_paused;
        let cancellation_token = self.shared.child_token();

        _start(
            self,
            peer_rx,
            start_paused,
            session,
            Some(g),
            cancellation_token,
        )
    }

    pub fn is_paused(&self) -> bool {
        self.locked.read().paused
    }

    /// True while the torrent is waiting for a download or seed slot.
    pub fn queue_position(&self) -> i64 {
        self.shared.queue_position.load(Ordering::Relaxed)
    }

    pub fn is_force_started(&self) -> bool {
        self.shared.force_start.load(Ordering::Relaxed)
    }

    pub fn is_sequential_download(&self) -> bool {
        self.shared.sequential_download.load(Ordering::Relaxed)
    }

    pub fn set_sequential_download(&self, enabled: bool) {
        self.shared
            .sequential_download
            .store(enabled, Ordering::Relaxed);
    }

    pub fn is_first_last_piece_priority(&self) -> bool {
        self.shared
            .first_last_piece_priority
            .load(Ordering::Relaxed)
    }

    pub fn set_first_last_piece_priority(&self, enabled: bool) {
        self.shared
            .first_last_piece_priority
            .store(enabled, Ordering::Relaxed);
    }

    pub fn is_queued(&self) -> bool {
        self.shared.queued.load(Ordering::Relaxed)
    }

    /// Replace the current state with a fresh initializer that ignores any
    /// persisted bitfield and hashes every selected piece from storage.
    /// Returns whether the torrent was live and should resume after checking.
    pub(crate) fn prepare_force_recheck(&self) -> anyhow::Result<bool> {
        let metadata = self
            .metadata
            .load_full()
            .context("torrent metadata was not loaded")?;
        let mut locked = self.locked.write();
        let (files, resume_after_check) = match locked.state.take() {
            ManagedTorrentState::Initializing(initializing) => {
                locked.state = ManagedTorrentState::Initializing(initializing);
                bail!("torrent is already checking")
            }
            ManagedTorrentState::Live(live) => {
                // Cancels all live peer and tracker tasks and returns ownership
                // of the existing storage for the checker.
                let paused = match live.pause() {
                    Ok(paused) => paused,
                    Err(error) => {
                        locked.state = ManagedTorrentState::Error(anyhow::anyhow!(
                            "failed to pause torrent for integrity check: {error:#}"
                        ));
                        return Err(error);
                    }
                };
                (paused.files, true)
            }
            ManagedTorrentState::Paused(paused) => (paused.files, false),
            ManagedTorrentState::Error(error) => {
                match self
                    .shared
                    .storage_factory
                    .create_and_init(self.shared(), &metadata)
                {
                    Ok(files) => (files, false),
                    Err(create_error) => {
                        locked.state = ManagedTorrentState::Error(error);
                        return Err(create_error);
                    }
                }
            }
            ManagedTorrentState::None => {
                locked.state = ManagedTorrentState::None;
                bail!("bug: torrent is in empty state")
            }
        };

        locked.state = ManagedTorrentState::Initializing(Arc::new(TorrentStateInitializing::new(
            self.shared.clone(),
            metadata,
            locked.only_files.clone(),
            files,
            true,
        )));
        locked.paused = !resume_after_check;
        self.state_change_notify.notify_waiters();
        Ok(resume_after_check)
    }

    pub(crate) fn add_peer_stream(&self, peer_rx: PeerStream) -> anyhow::Result<()> {
        let live = self.live().context("torrent is not live")?;
        let weak = Arc::downgrade(&live);
        live.spawn(
            debug_span!(parent: live.shared.span.clone(), "added_peer_source"),
            format!("[{}]added_peer_source", live.shared.id),
            async move { drain_peer_stream(&weak, peer_rx).await },
        );
        Ok(())
    }

    /// Pause the torrent if it's live or initializing.
    pub(crate) fn pause(&self) -> anyhow::Result<()> {
        let mut g = self.locked.write();
        match &g.state {
            ManagedTorrentState::Live(live) => {
                let paused = live.pause()?;
                self.shared
                    .uploaded_bytes_base
                    .fetch_add(live.get_uploaded_bytes(), Ordering::Relaxed);
                g.state = ManagedTorrentState::Paused(paused);
                g.paused = true;
                self.state_change_notify.notify_waiters();
                Ok(())
            }
            ManagedTorrentState::Initializing(_) => {
                // Cancel the init task via the per-torrent cancellation token,
                // then reset the token so the torrent can be restarted later.
                // Transition to Error state so it can be re-initialized on unpause.
                self.shared.cancel_and_reset_token();
                g.state =
                    ManagedTorrentState::Error(anyhow::anyhow!("paused during initialization"));
                g.paused = true;
                self.state_change_notify.notify_waiters();
                Ok(())
            }
            ManagedTorrentState::Paused(_) => {
                bail!("torrent is already paused");
            }
            ManagedTorrentState::Error(_) => {
                bail!("can't pause torrent in error state")
            }
            ManagedTorrentState::None => bail!("bug: torrent is in empty state"),
        }
    }

    /// Bytes uploaded across every run of this torrent, including the current one.
    pub fn uploaded_bytes(&self) -> u64 {
        let live = self.with_state(|s| match s {
            ManagedTorrentState::Live(l) => l.get_uploaded_bytes(),
            _ => 0,
        });
        self.shared.uploaded_bytes_base.load(Ordering::Relaxed) + live
    }

    /// Get stats.
    pub fn stats(&self) -> TorrentStats {
        use stats::TorrentStatsState as S;
        let mut resp = TorrentStats {
            total_bytes: self
                .metadata
                .load()
                .as_ref()
                .map(|r| r.info.lengths().total_length())
                .unwrap_or_default(),
            file_progress: Vec::new(),
            state: S::Error,
            error: None,
            progress_bytes: 0,
            uploaded_bytes: 0,
            finished: false,
            live: None,
            queued_for_init: None,
        };

        resp.uploaded_bytes = self.shared.uploaded_bytes_base.load(Ordering::Relaxed);
        self.with_state(|s| {
            match s {
                ManagedTorrentState::Initializing(i) => {
                    resp.state = S::Initializing;
                    resp.progress_bytes = i.checked_bytes.load(Ordering::Relaxed);
                    resp.queued_for_init = Some(i.is_queued_for_init());
                }
                ManagedTorrentState::Paused(p) => {
                    resp.state = S::Paused;
                    let hns = p.hns();
                    resp.total_bytes = hns.total();
                    resp.progress_bytes = hns.progress();
                    resp.finished = hns.finished();
                    resp.file_progress = p.chunk_tracker.per_file_have_bytes().to_owned();
                    if p.files.is_read_only() && !resp.finished {
                        resp.error = Some(READ_ONLY_INCOMPLETE_PAYLOAD.to_owned());
                    }
                }
                ManagedTorrentState::Live(l) => {
                    resp.state = S::Live;
                    let live_stats = LiveStats::from(l.as_ref());
                    let hns = l.get_hns().unwrap_or_default();
                    resp.total_bytes = hns.total();
                    resp.progress_bytes = hns.progress();
                    resp.finished = hns.finished();
                    resp.uploaded_bytes += l.get_uploaded_bytes();
                    resp.file_progress = l
                        .lock_read("file_progress")
                        .get_chunks()
                        .ok()
                        .map(|c| c.per_file_have_bytes().to_owned())
                        .unwrap_or_default();
                    resp.live = Some(live_stats);
                }
                ManagedTorrentState::Error(e) => {
                    resp.state = S::Error;
                    resp.error = Some(format!("{e:?}"))
                }
                ManagedTorrentState::None => {
                    resp.state = S::Error;
                    resp.error = Some("bug: torrent in broken \"None\" state".to_string());
                }
            }
            resp
        })
    }

    #[inline(never)]
    pub fn wait_until_initialized(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            // TODO: rewrite, this polling is horrible
            loop {
                let done = self.with_state(|s| match s {
                    ManagedTorrentState::Initializing(_) => Ok(false),
                    ManagedTorrentState::Error(e) => bail!("{e:?}"),
                    ManagedTorrentState::None => bail!("bug: torrent state is None"),
                    _ => Ok(true),
                })?;
                if done {
                    return Ok(());
                }
                let _ = timeout(
                    Duration::from_millis(100),
                    self.state_change_notify.notified(),
                )
                .await;
            }
        }
        .boxed()
    }

    #[inline(never)]
    pub fn wait_until_completed(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            // TODO: rewrite, this polling is horrible
            let live = loop {
                let live = self.with_state(|s| match s {
                    ManagedTorrentState::Initializing(_) | ManagedTorrentState::Paused(_) => {
                        Ok(None)
                    }
                    ManagedTorrentState::Live(l) => Ok(Some(l.clone())),
                    ManagedTorrentState::Error(e) => bail!("{e:?}"),
                    ManagedTorrentState::None => bail!("bug: torrent state is None"),
                })?;
                if let Some(live) = live {
                    break live;
                }
                let _ = timeout(Duration::from_secs(1), self.state_change_notify.notified()).await;
            };

            live.wait_until_completed().await;
            Ok(())
        }
        .boxed()
    }

    // Returns true if needed to unpause torrent.
    // This is just implementation detail - it's easier to pause/unpause than to tinker with internals.
    pub(crate) fn update_only_files(&self, only_files: &HashSet<usize>) -> anyhow::Result<()> {
        let metadata = self.metadata.load();
        let metadata = metadata.as_ref().context("torrent is not resolved")?;
        let file_count = metadata.file_infos.len();
        for f in only_files.iter().copied() {
            if f >= file_count {
                anyhow::bail!("only_files contains invalid value {f}")
            }
        }

        // if live, need to update chunk tracker
        // - if already finished: need to pause, then unpause (to reopen files etc)
        // if paused, need to update chunk tracker

        let mut g = self.locked.write();
        match &mut g.state {
            ManagedTorrentState::Initializing(_) => bail!("can't update initializing torrent"),
            ManagedTorrentState::Error(_) => {}
            ManagedTorrentState::None => {}
            ManagedTorrentState::Paused(p) => {
                p.update_only_files(only_files)?;
            }
            ManagedTorrentState::Live(l) => {
                l.update_only_files(only_files)?;
            }
        };

        g.only_files = Some(only_files.iter().copied().collect());
        Ok(())
    }
}

pub type ManagedTorrentHandle = Arc<ManagedTorrent>;

fn spawn_fatal_errors_receiver(
    state: &Arc<ManagedTorrent>,
    rx: tokio::sync::oneshot::Receiver<anyhow::Error>,
    token: CancellationToken,
) {
    let span = state.shared.span.clone();
    let id = state.shared.id;
    let info_hash = state.shared.info_hash;
    let state = Arc::downgrade(state);
    spawn_with_cancel::<&'static str>(
        debug_span!(parent: span, "fatal_errors_receiver"),
        "fatal_errors_receiver",
        token,
        async move {
            let e = match rx.await {
                Ok(e) => e,
                Err(_) => return Ok(()),
            };
            if let Some(state) = state.upgrade() {
                state.stop_with_error(e);
            } else {
                warn!(
                    ?id,
                    ?info_hash,
                    "tried to stop the torrent with error, but couldn't upgrade the arc"
                );
            }
            Ok(())
        },
    );
}

fn spawn_peer_adder(live: &Arc<TorrentStateLive>, peer_rx: PeerStream) {
    live.spawn(
        debug_span!(parent: live.torrent().span.clone(), "external_peer_adder"),
        format!("[{}]external_peer_adder", live.shared.id),
        {
            let live = live.clone();
            async move {
                let live = {
                    let weak = Arc::downgrade(&live);
                    drop(live);
                    weak
                };

                drain_peer_stream(&live, peer_rx).await?;

                // The initial peer stream is exhausted. Now wait for rediscovery
                // signals and create new peer streams on demand.
                debug!("initial peer_rx exhausted, waiting for rediscovery signals");
                loop {
                    let state = match live.upgrade() {
                        Some(s) => s,
                        None => return Ok(()),
                    };

                    // Wait for rediscovery_notify to be signaled by the health monitor.
                    // We must hold the Arc alive while awaiting since Notified borrows Notify.
                    state.rediscovery_notify.notified().await;

                    // Don't re-discover if we're finished.
                    if state.is_finished_and_no_active_streams() {
                        continue;
                    }

                    debug!(
                        id = state.shared.id,
                        "rediscovery signal received, requesting new peers from DHT/trackers"
                    );

                    let session = match state.shared.session.upgrade() {
                        Some(s) => s,
                        None => return Ok(()),
                    };

                    // Create a fresh peer stream from DHT and trackers.
                    let is_private = state.metadata.info.info().private;
                    let new_peer_rx = session.make_peer_rx(
                        state.shared.info_hash,
                        state.shared.trackers.read().iter().cloned().collect(),
                        true, // announce
                        state.shared.options.force_tracker_interval,
                        Vec::new(), // no initial peers on re-discovery
                        is_private,
                        Some(state.shared.tracker_status.clone()),
                    );
                    drop(state);

                    if let Some(rx) = new_peer_rx {
                        drain_peer_stream(&live, rx).await?;
                        debug!("rediscovery peer stream exhausted");
                    }
                }
            }
        },
    );
}

/// Drains a peer stream, adding each discovered peer to the torrent.
/// Returns when the stream ends or the torrent is no longer live.
async fn drain_peer_stream(
    live: &std::sync::Weak<TorrentStateLive>,
    mut peer_rx: PeerStream,
) -> crate::Result<()> {
    loop {
        match timeout(Duration::from_secs(5), peer_rx.next()).await {
            Ok(Some(peer)) => {
                trace!(?peer, "received peer");
                let state = match live.upgrade() {
                    Some(state) => state,
                    None => return Ok(()),
                };
                state.add_peer_if_not_seen(peer)?;
            }
            Ok(None) => {
                return Ok(());
            }
            // If timeout, check if the torrent is live.
            Err(_) if live.strong_count() == 0 => {
                debug!("timed out waiting for peers, torrent isn't live");
                return Ok(());
            }
            Err(_) => continue,
        }
    }
}

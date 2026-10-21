//! qBittorrent WebUI API v2 compatibility layer.
//!
//! This module implements a subset of the qBittorrent WebUI API v2 so that
//! *arr apps (Sonarr, Radarr, etc.) can use rtbit as a download client by
//! pretending to be qBittorrent.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    num::{NonZeroU16, NonZeroU32},
    path::Path,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    extract::{Multipart, Query, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode, header::RETRY_AFTER, header::SET_COOKIE};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, SessionPersistenceConfig,
    alt_speed::AltSpeedConfig,
    api::{Api, TorrentIdOrHash},
    create_torrent,
    limits::LimitsConfig,
    session::{QueueMove, ShareLimits},
    spawn_utils::BlockingSpawner,
    stream_connect::ConnectionKind,
    torrent_state::{peer::stats::snapshot::PeerStatsFilter, stats::TorrentStatsState},
};

use super::{ApiState, qbit_sync::SyncSnapshots};

mod app;
mod auth;
mod categories;
mod clientdata;
mod log;
mod sync;
mod tags;
#[cfg(test)]
mod tests;
mod torrent_actions;
mod torrent_limits;
mod torrentcreator;
mod torrents;
mod transfer;

use app::*;
use auth::*;
use categories::*;
use clientdata::*;
use log::*;
use sync::*;
use tags::*;
use torrent_actions::*;
use torrent_limits::*;
use torrentcreator::*;
use torrents::*;
use transfer::*;

// ---------------------------------------------------------------------------
// Shared state for qBit compat layer
// ---------------------------------------------------------------------------

/// Idle time after which a compat session cookie stops being accepted. Every
/// authenticated request refreshes the timestamp (sliding expiry).
const SESSION_IDLE_TTL: Duration = Duration::from_secs(3600);

/// In-memory session store for qBittorrent auth cookies.
struct QbitSessions {
    /// Session id -> time of last use.
    sessions: RwLock<HashMap<String, Instant>>,
    idle_ttl: Duration,
}

impl QbitSessions {
    fn new() -> Self {
        Self::with_idle_ttl(SESSION_IDLE_TTL)
    }

    fn with_idle_ttl(idle_ttl: Duration) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            idle_ttl,
        }
    }

    fn create_session(&self) -> String {
        let sid: String = (0..32)
            .map(|_| format!("{:02x}", rand::random::<u8>()))
            .collect();
        let mut sessions = self.sessions.write();
        // Drop idle entries here so abandoned sessions cannot accumulate.
        sessions.retain(|_, last_used| last_used.elapsed() < self.idle_ttl);
        sessions.insert(sid.clone(), Instant::now());
        sid
    }

    /// Returns true for a live session and, if so, slides its expiry forward.
    fn validate_session(&self, sid: &str) -> bool {
        let mut sessions = self.sessions.write();
        match sessions.get_mut(sid) {
            Some(last_used) if last_used.elapsed() < self.idle_ttl => {
                *last_used = Instant::now();
                true
            }
            Some(_) => {
                sessions.remove(sid);
                false
            }
            None => false,
        }
    }

    fn remove_session(&self, sid: &str) {
        self.sessions.write().remove(sid);
    }

    /// Drop every session idle longer than the TTL. Runs periodically so SIDs
    /// from clients that re-login without logging out cannot pile up.
    fn purge_expired(&self) -> usize {
        let mut sessions = self.sessions.write();
        let before = sessions.len();
        sessions.retain(|_, last_used| last_used.elapsed() < self.idle_ttl);
        before - sessions.len()
    }
}

/// Spawn a task that purges expired compat sessions every `idle_ttl / 4`. The
/// task holds a weak reference and exits once the compat state is dropped.
fn spawn_session_purge(state: &Arc<QbitState>) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let period = (state.sessions.idle_ttl / 4).max(Duration::from_secs(1));
    let weak = Arc::downgrade(state);
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(period);
        interval.tick().await;
        loop {
            interval.tick().await;
            let Some(state) = weak.upgrade() else { break };
            let purged = state.sessions.purge_expired();
            if purged > 0 {
                debug!(purged, "qbit compat: purged expired sessions");
            }
        }
    });
}

/// On-disk shape of the tag store the compat layer kept before tags moved
/// into the session (`qbit-tags.json`).
#[derive(Default, Deserialize)]
#[serde(default)]
struct LegacyTagStore {
    all: BTreeSet<String>,
    per_torrent: HashMap<String, BTreeSet<String>>,
}

/// Import a legacy compat tag store into the session's tags, then rename it to
/// `*.migrated` so the import runs once. Tags for torrents that are no longer
/// in the session are dropped. A file that cannot be read or parsed is logged
/// and left in place.
async fn migrate_legacy_tag_store(api: &Api, path: &Path) {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            warn!(?path, %error, "qbit compat: cannot read legacy tag store");
            return;
        }
    };
    let legacy: LegacyTagStore = match serde_json::from_slice(&bytes) {
        Ok(legacy) => legacy,
        Err(error) => {
            warn!(?path, %error, "qbit compat: ignoring unreadable legacy tag store");
            return;
        }
    };
    let all: Vec<String> = legacy.all.into_iter().collect();
    if let Err(error) = api.api_create_tags(&all).await {
        warn!(%error, "qbit compat: error importing legacy tags");
        return;
    }
    for (hash, tags) in legacy.per_torrent {
        let Ok(idx) = TorrentIdOrHash::parse(&hash) else {
            continue;
        };
        if api.mgr_handle(idx).is_err() {
            debug!(%hash, "qbit compat: legacy tags for unknown torrent dropped");
            continue;
        }
        let tags: Vec<String> = tags.into_iter().collect();
        if let Err(error) = api.api_add_torrent_tags(idx, &tags).await {
            warn!(%hash, %error, "qbit compat: error importing legacy torrent tags");
        }
    }
    let migrated = path.with_extension("json.migrated");
    match tokio::fs::rename(path, &migrated).await {
        Ok(()) => info!(?path, "qbit compat: imported legacy tag store"),
        Err(error) => warn!(?path, %error, "qbit compat: cannot rename legacy tag store"),
    }
}

pub(crate) struct QbitState {
    api_state: ApiState,
    sessions: QbitSessions,
    creator: QbitTorrentCreator,
    sync: SyncSnapshots,
    clientdata: ClientDataStore,
}

// ---------------------------------------------------------------------------
// Serializable response types (avoids json! macro recursion limit issues)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct QbitTorrentInfo {
    added_on: u64,
    amount_left: u64,
    auto_tmm: bool,
    availability: i32,
    category: String,
    completed: u64,
    completion_on: i64,
    content_path: String,
    dl_limit: i32,
    dlspeed: u64,
    download_path: String,
    downloaded: u64,
    downloaded_session: u64,
    eta: i64,
    f_l_piece_prio: bool,
    force_start: bool,
    hash: String,
    inactive_seeding_time_limit: i32,
    infohash_v1: String,
    infohash_v2: String,
    last_activity: u64,
    magnet_uri: String,
    max_inactive_seeding_time: i32,
    max_ratio: f64,
    max_seeding_time: i32,
    name: String,
    num_complete: u32,
    num_incomplete: u32,
    num_leechs: u32,
    num_seeds: u32,
    priority: u32,
    progress: f64,
    ratio: f64,
    ratio_limit: f64,
    save_path: String,
    seeding_time: u64,
    seeding_time_limit: i32,
    seen_complete: i64,
    seq_dl: bool,
    size: u64,
    state: String,
    super_seeding: bool,
    tags: String,
    time_active: u64,
    total_size: u64,
    tracker: String,
    trackers_count: usize,
    up_limit: i32,
    uploaded: u64,
    uploaded_session: u64,
    upspeed: u64,
}

#[derive(Serialize)]
struct QbitTorrentProperties {
    save_path: String,
    creation_date: i64,
    piece_size: u64,
    comment: String,
    total_wasted: u64,
    total_uploaded: u64,
    total_uploaded_session: u64,
    total_downloaded: u64,
    total_downloaded_session: u64,
    up_limit: i32,
    dl_limit: i32,
    time_elapsed: u64,
    seeding_time: u64,
    nb_connections: u32,
    nb_connections_limit: i32,
    share_ratio: f64,
    addition_date: u64,
    completion_date: i64,
    created_by: String,
    dl_speed_avg: u64,
    dl_speed: u64,
    eta: i64,
    last_seen: u64,
    peers: u32,
    peers_total: u32,
    pieces_have: u32,
    pieces_num: u32,
    reannounce: u32,
    seeds: u32,
    seeds_total: u32,
    total_size: u64,
    up_speed_avg: u64,
    up_speed: u64,
}

#[derive(Serialize)]
struct QbitFileInfo {
    index: usize,
    name: String,
    size: u64,
    progress: f64,
    priority: u8,
    is_seed: bool,
    piece_range: [u32; 2],
    availability: f64,
}

fn qbit_save_path(handle: &crate::torrent_state::ManagedTorrentHandle) -> String {
    let shared = handle.shared();
    // After a relocation the override diverges from the add-time output_folder;
    // report the current root. Otherwise keep the qbit-savepath (output_folder_root)
    // behaviour used for multi-file name prefixing.
    let current_root = shared.output_folder_override.read();
    if *current_root != shared.options.output_folder {
        return current_root.to_string_lossy().into_owned();
    }
    shared
        .options
        .output_folder_root
        .as_ref()
        .unwrap_or(&shared.options.output_folder)
        .to_string_lossy()
        .into_owned()
}

fn qbit_content_path(handle: &crate::torrent_state::ManagedTorrentHandle, name: &str) -> String {
    Path::new(&qbit_save_path(handle))
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn qbit_file_name(
    torrent_name: Option<&str>,
    file_name: &str,
    is_multi_file: bool,
    has_save_path_root: bool,
) -> String {
    if is_multi_file && has_save_path_root {
        torrent_name
            .map(|root| {
                Path::new(root)
                    .join(file_name)
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| file_name.to_owned())
    } else {
        file_name.to_owned()
    }
}

#[derive(Serialize)]
struct QbitBuildInfo {
    qt: &'static str,
    libtorrent: &'static str,
    boost: &'static str,
    openssl: &'static str,
    bitness: u32,
}

#[derive(Serialize)]
struct QbitTransferInfo {
    dl_info_speed: u64,
    dl_info_data: u64,
    up_info_speed: u64,
    up_info_data: u64,
    dl_rate_limit: u64,
    up_rate_limit: u64,
    dht_nodes: u64,
    connection_status: &'static str,
}

#[derive(Serialize)]
struct QbitPreferences {
    save_path: String,
    temp_path_enabled: bool,
    temp_path: String,
    max_connec: i32,
    max_connec_per_torrent: i32,
    max_uploads: i32,
    max_uploads_per_torrent: i32,
    dl_limit: i64,
    up_limit: i64,
    dht: bool,
    pex: bool,
    lsd: bool,
    encryption: u32,
    queueing_enabled: bool,
    max_active_downloads: i64,
    max_active_uploads: i64,
    max_active_torrents: i64,
    locale: &'static str,
    web_ui_port: u16,
    listen_port: u16,
    announce_port: u16,
    alt_dl_limit: i64,
    alt_up_limit: i64,
    scheduler_enabled: bool,
    // Seeding limits are reported disabled: rtbit does not enforce them
    // globally (per-torrent limits go through torrents/setShareLimits). The
    // values are qBittorrent's defaults, which *arr apps read back as-is.
    max_ratio_enabled: bool,
    max_ratio: f64,
    max_ratio_act: u8,
    max_seeding_time_enabled: bool,
    max_seeding_time: i64,
    max_inactive_seeding_time_enabled: bool,
    max_inactive_seeding_time: i64,
}

#[derive(Deserialize)]
struct SetPreferencesForm {
    json: String,
}

/// The preference fields rtbit can act on. Everything else a client sends
/// (read-modify-write clients post the whole preferences object back) is
/// ignored, so unknown or unsupported keys do not fail the request.
#[derive(Deserialize)]
struct SetPreferences {
    #[serde(default)]
    announce_port: Option<u16>,
    #[serde(default)]
    queueing_enabled: Option<bool>,
    #[serde(default)]
    max_active_downloads: Option<i64>,
    #[serde(default)]
    max_active_uploads: Option<i64>,
    #[serde(default)]
    dl_limit: Option<i64>,
    #[serde(default)]
    up_limit: Option<i64>,
    #[serde(default)]
    alt_dl_limit: Option<i64>,
    #[serde(default)]
    alt_up_limit: Option<i64>,
}

/// A preferences byte-rate limit as rtbit stores it: 0 or negative means
/// unlimited.
fn pref_limit(limit: i64) -> Option<u64> {
    u64::try_from(limit).ok().filter(|&n| n != 0)
}

/// An rtbit byte-rate limit as preferences report it: 0 means unlimited.
fn pref_limit_to_qbit(limit: Option<u64>) -> i64 {
    limit.map_or(0, |n| i64::try_from(n).unwrap_or(i64::MAX))
}

/// The queue limit qBittorrent reports for an rtbit limit: -1 for unlimited.
fn limit_to_qbit(limit: Option<usize>) -> i64 {
    limit.map_or(-1, |n| i64::try_from(n).unwrap_or(i64::MAX))
}

/// The rtbit limit for a qBittorrent queue limit, where a negative value means
/// unlimited.
fn limit_from_qbit(limit: i64) -> Option<usize> {
    usize::try_from(limit).ok()
}

/// qBittorrent's default queue limit, used when queueing is switched on
/// without limits and none were set before.
const DEFAULT_QUEUE_LIMIT: usize = 3;

/// Work out the queue limits a preferences update asks for, or `None` if it
/// does not touch queueing. rtbit has no separate on/off switch: queueing is
/// on while either limit is set, so turning it off clears both limits.
fn queue_limits_update(
    prefs: &SetPreferences,
    current: (Option<usize>, Option<usize>),
) -> Option<(Option<usize>, Option<usize>)> {
    if prefs.queueing_enabled.is_none()
        && prefs.max_active_downloads.is_none()
        && prefs.max_active_uploads.is_none()
    {
        return None;
    }
    let currently_enabled = current.0.is_some() || current.1.is_some();
    if !prefs.queueing_enabled.unwrap_or(currently_enabled) {
        return Some((None, None));
    }
    let pick = |requested: Option<i64>, current: Option<usize>| match requested {
        Some(limit) => limit_from_qbit(limit),
        None if currently_enabled => current,
        None => Some(DEFAULT_QUEUE_LIMIT),
    };
    Some((
        pick(prefs.max_active_downloads, current.0),
        pick(prefs.max_active_uploads, current.1),
    ))
}

// ---------------------------------------------------------------------------
// Router construction
// ---------------------------------------------------------------------------

/// Import the pre-backend `qbit-tags.json` store, if one is left in the
/// session data directory, in the background.
fn spawn_legacy_tag_migration(api: Api) {
    // Tests build many routers; keep them off the user's real data directory.
    if cfg!(test) {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    match SessionPersistenceConfig::default_json_persistence_folder() {
        Ok(dir) => {
            runtime.spawn(async move {
                migrate_legacy_tag_store(&api, &dir.join("qbit-tags.json")).await;
            });
        }
        Err(error) => debug!(%error, "qbit compat: no data directory, skipping tag migration"),
    }
}

/// Build the qBittorrent v2 API router. Should be nested at `/api/v2`.
pub(crate) fn make_qbit_router(api_state: ApiState) -> Router {
    spawn_legacy_tag_migration(api_state.api.clone());
    let qbit_state = Arc::new(QbitState {
        api_state: api_state.clone(),
        sessions: QbitSessions::new(),
        creator: QbitTorrentCreator::default(),
        sync: SyncSnapshots::default(),
        clientdata: ClientDataStore::open(clientdata_path()),
    });
    spawn_session_purge(&qbit_state);

    // Auth endpoints (no auth required to reach these)
    let auth_router = Router::new()
        .route("/login", post(h_auth_login))
        .route("/logout", post(h_auth_logout));

    // App info endpoints
    let app_router = Router::new()
        .route("/version", get(h_app_version))
        .route("/webapiVersion", get(h_app_webapi_version))
        .route("/buildInfo", get(h_app_build_info))
        .route("/defaultSavePath", get(h_app_default_save_path))
        .route("/preferences", get(h_app_preferences))
        .route("/setPreferences", post(h_app_set_preferences));

    // Torrent endpoints
    let torrents_router = Router::new()
        .route("/info", get(h_torrents_info))
        .route("/count", get(h_torrents_count))
        .route("/properties", get(h_torrents_properties))
        .route("/files", get(h_torrents_files))
        .route("/trackers", get(h_torrents_trackers))
        .route("/webseeds", get(h_torrents_webseeds))
        .route("/export", get(h_torrents_export))
        .route("/pieceStates", get(h_torrents_piece_states))
        .route("/pieceHashes", get(h_torrents_piece_hashes))
        .route("/add", post(h_torrents_add))
        .route("/pause", post(h_torrents_pause))
        .route("/resume", post(h_torrents_resume))
        .route("/stop", post(h_torrents_stop))
        .route("/start", post(h_torrents_start))
        .route("/setForceStart", post(h_torrents_set_force_start))
        .route(
            "/toggleSequentialDownload",
            post(h_torrents_toggle_sequential_download),
        )
        .route(
            "/toggleFirstLastPiecePrio",
            post(h_torrents_toggle_first_last_piece_prio),
        )
        .route("/topPrio", post(h_torrents_top_prio))
        .route("/bottomPrio", post(h_torrents_bottom_prio))
        .route("/increasePrio", post(h_torrents_increase_prio))
        .route("/decreasePrio", post(h_torrents_decrease_prio))
        .route("/recheck", post(h_torrents_recheck))
        .route("/reannounce", post(h_torrents_reannounce))
        .route("/rename", post(h_torrents_rename))
        .route("/renameFile", post(h_torrents_rename_file))
        .route("/renameFolder", post(h_torrents_rename_folder))
        .route("/delete", post(h_torrents_delete))
        .route("/addTrackers", post(h_torrents_add_trackers))
        .route("/removeTrackers", post(h_torrents_remove_trackers))
        .route("/editTracker", post(h_torrents_edit_tracker))
        .route("/addPeers", post(h_torrents_add_peers))
        .route("/filePrio", post(h_torrents_file_prio))
        .route("/downloadLimit", get(h_torrents_download_limit))
        .route("/uploadLimit", get(h_torrents_upload_limit))
        .route("/setDownloadLimit", post(h_torrents_set_download_limit))
        .route("/setUploadLimit", post(h_torrents_set_upload_limit))
        .route("/setShareLimits", post(h_torrents_set_share_limits))
        .route("/setLocation", post(h_torrents_set_location))
        .route("/setSavePath", post(h_torrents_set_location))
        .route("/setCategory", post(h_torrents_set_category))
        .route("/tags", get(h_torrents_tags))
        .route("/createTags", post(h_torrents_create_tags))
        .route("/deleteTags", post(h_torrents_delete_tags))
        .route("/addTags", post(h_torrents_add_tags))
        .route("/removeTags", post(h_torrents_remove_tags))
        .route("/setTags", post(h_torrents_set_tags))
        .route("/categories", get(h_categories))
        .route("/createCategory", post(h_create_category))
        .route("/editCategory", post(h_edit_category))
        .route("/removeCategories", post(h_remove_categories));

    // Transfer info + session speed limits
    let transfer_router = Router::new()
        .route("/info", get(h_transfer_info))
        .route("/downloadLimit", get(h_transfer_download_limit))
        .route("/uploadLimit", get(h_transfer_upload_limit))
        .route("/setDownloadLimit", post(h_transfer_set_download_limit))
        .route("/setUploadLimit", post(h_transfer_set_upload_limit))
        .route("/getSpeedLimits", get(h_transfer_get_speed_limits))
        .route("/setSpeedLimits", post(h_transfer_set_speed_limits))
        .route("/speedLimitsMode", get(h_transfer_speed_limits_mode))
        .route(
            "/toggleSpeedLimitsMode",
            post(h_transfer_toggle_speed_limits_mode),
        )
        .route(
            "/setSpeedLimitsMode",
            post(h_transfer_set_speed_limits_mode),
        )
        .route("/pauseSession", post(h_transfer_pause_session))
        .route("/resumeSession", post(h_transfer_resume_session))
        .route("/banPeers", post(h_transfer_ban_peers));

    let log_router = Router::new()
        .route("/main", get(h_log_main))
        .route("/peers", get(h_log_peers));

    // Sync (main polling endpoint). Must stay nested inside protected_router so
    // it runs behind the SID-cookie auth layer.
    let sync_router = Router::new()
        .route("/maindata", get(h_sync_maindata))
        .route("/torrentPeers", get(h_sync_torrent_peers));

    // Torrent creator task API (also behind the auth layer).
    let torrentcreator_router = Router::new()
        .route("/addTask", post(h_torrentcreator_add_task))
        .route("/status", get(h_torrentcreator_status))
        .route("/torrentFile", get(h_torrentcreator_torrent_file))
        .route("/deleteTask", post(h_torrentcreator_delete_task));

    // WebUI client-state key/value store.
    let clientdata_router = Router::new()
        .route("/load", get(h_clientdata_load))
        .route("/store", post(h_clientdata_store));

    let protected_router = Router::new()
        .nest("/app", app_router)
        .nest("/torrents", torrents_router)
        .nest("/transfer", transfer_router)
        .nest("/log", log_router)
        .nest("/sync", sync_router)
        .nest("/torrentcreator", torrentcreator_router)
        .nest("/clientdata", clientdata_router)
        .route_layer({
            let qbit_state_for_layer = qbit_state.clone();
            axum::middleware::from_fn(
                move |headers: HeaderMap,
                      request: axum::extract::Request,
                      next: axum::middleware::Next| {
                    let qbit_state = qbit_state_for_layer.clone();
                    async move {
                        let auth_configured = qbit_state.api_state.opts.basic_auth.is_some()
                            || qbit_state
                                .api_state
                                .opts
                                .credential_store
                                .as_ref()
                                .is_some_and(|store| store.has_credentials());
                        if !auth_configured {
                            // A credential store with no credentials means setup is pending.
                            if qbit_state.api_state.opts.credential_store.is_some() {
                                return Err((StatusCode::FORBIDDEN, "Setup required"));
                            }
                            return Ok(next.run(request).await);
                        }
                        let sid = extract_sid(&headers);
                        let valid = sid
                            .as_deref()
                            .is_some_and(|s| qbit_state.sessions.validate_session(s));
                        if valid {
                            Ok(next.run(request).await)
                        } else {
                            Err((StatusCode::FORBIDDEN, "Not authenticated"))
                        }
                    }
                },
            )
        });

    Router::new()
        .nest("/auth", auth_router)
        .merge(protected_router)
        .layer(axum::middleware::from_fn(reject_cross_origin_posts))
        .with_state(qbit_state)
}

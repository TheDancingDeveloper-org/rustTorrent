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

// ---------------------------------------------------------------------------
// Torrent-creator task store (bridges the native create_torrent)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Serialize)]
enum CreatorStatus {
    Running,
    Finished,
    Failed,
}

#[derive(Clone)]
struct CreatorTask {
    status: CreatorStatus,
    source_path: String,
    format: String,
    error: Option<String>,
    torrent_bytes: Option<Bytes>,
    info_hash: Option<String>,
}

/// In-memory registry of torrent-creation tasks. rtbit creates torrents
/// synchronously via `create_torrent`; each qBittorrent "task" runs that on a
/// spawned future and records the result here. Not persisted across restarts.
#[derive(Default)]
struct QbitTorrentCreator {
    tasks: RwLock<HashMap<String, CreatorTask>>,
}

impl QbitTorrentCreator {
    fn new_task(&self, source_path: String, format: String) -> String {
        let id: String = (0..16)
            .map(|_| format!("{:02x}", rand::random::<u8>()))
            .collect();
        self.tasks.write().insert(
            id.clone(),
            CreatorTask {
                status: CreatorStatus::Running,
                source_path,
                format,
                error: None,
                torrent_bytes: None,
                info_hash: None,
            },
        );
        id
    }

    fn finish(&self, id: &str, torrent_bytes: Option<Bytes>, info_hash: String) {
        if let Some(task) = self.tasks.write().get_mut(id) {
            task.status = CreatorStatus::Finished;
            task.torrent_bytes = torrent_bytes;
            task.info_hash = Some(info_hash);
        }
    }

    fn fail(&self, id: &str, error: String) {
        if let Some(task) = self.tasks.write().get_mut(id) {
            task.status = CreatorStatus::Failed;
            task.error = Some(error);
        }
    }

    fn get(&self, id: &str) -> Option<CreatorTask> {
        self.tasks.read().get(id).cloned()
    }

    fn all(&self) -> Vec<(String, CreatorTask)> {
        self.tasks
            .read()
            .iter()
            .map(|(id, task)| (id.clone(), task.clone()))
            .collect()
    }

    fn remove(&self, id: &str) -> bool {
        self.tasks.write().remove(id).is_some()
    }
}

/// WebUI API v2 share-limit encoding: `-1` (and `-2`, "use the global
/// limit", which rtbit leaves disabled) mean no limit.
fn ratio_from_qbit(raw: f64) -> Option<f64> {
    (raw >= 0.0).then_some(raw)
}

fn ratio_to_qbit(limit: Option<f64>) -> f64 {
    limit.unwrap_or(-1.0)
}

/// Minutes on the wire, `Duration` in the session; negative means no limit.
fn minutes_from_qbit(raw: i64) -> Option<std::time::Duration> {
    u64::try_from(raw)
        .ok()
        .map(|m| std::time::Duration::from_secs(m.saturating_mul(60)))
}

fn minutes_to_qbit(limit: Option<std::time::Duration>) -> i32 {
    limit.map_or(-1, |d| i32::try_from(d.as_secs() / 60).unwrap_or(i32::MAX))
}

fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Clone, Serialize)]
struct QbitCategory {
    name: String,
    #[serde(rename = "savePath")]
    save_path: String,
}

/// Combined qBit compat state.
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
// Auth endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct LoginForm {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

async fn h_auth_login(
    State(state): State<Arc<QbitState>>,
    super::super::auth::ClientAddr(client): super::super::auth::ClientAddr,
    body: Bytes,
) -> impl IntoResponse {
    use super::super::auth::{Role, Verdict, busy_response, verify_credentials};

    let limiter = &state.api_state.login_limiter;
    if let Some(wait) = limiter.check(client, Instant::now()) {
        let retry_after = wait.as_secs().max(1).to_string();
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(RETRY_AFTER, retry_after)],
            "Fails.".to_string(),
        )
            .into_response();
    }

    let form: LoginForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();

    let stored_credentials = state
        .api_state
        .opts
        .credential_store
        .as_ref()
        .filter(|store| store.has_credentials());
    let verdict = match (stored_credentials, &state.api_state.opts.basic_auth) {
        (Some(store), _) => {
            match verify_credentials(limiter, Arc::clone(store), form.username, form.password).await
            {
                // The compat session carries no role, so a read-only account is refused here
                // rather than given write access through it.
                Verdict::Valid(Role::ReadOnly) => Verdict::Invalid,
                other => other,
            }
        }
        (None, Some((expected_user, expected_pass))) => {
            if super::super::constant_time_eq(form.username.as_bytes(), expected_user.as_bytes())
                && super::super::constant_time_eq(
                    form.password.as_bytes(),
                    expected_pass.as_bytes(),
                )
            {
                Verdict::Valid(Role::Admin)
            } else {
                Verdict::Invalid
            }
        }
        // No credentials at all: only acceptable when there is no credential store either.
        // With a store, setup has not happened yet and login must fail.
        (None, None) => {
            if state.api_state.opts.credential_store.is_none() {
                Verdict::Valid(Role::Admin)
            } else {
                Verdict::Invalid
            }
        }
    };

    match verdict {
        Verdict::Busy => busy_response(),
        Verdict::Valid(_) => {
            limiter.record_success(client);
            let sid = state.sessions.create_session();
            let cookie = format!("SID={sid}; Path=/; HttpOnly; SameSite=Strict");
            (StatusCode::OK, [(SET_COOKIE, cookie)], "Ok.".to_string()).into_response()
        }
        Verdict::Invalid => {
            limiter.record_failure(client, Instant::now());
            (StatusCode::FORBIDDEN, "Fails.".to_string()).into_response()
        }
    }
}

async fn h_auth_logout(State(state): State<Arc<QbitState>>, headers: HeaderMap) -> &'static str {
    if let Some(sid) = extract_sid(&headers) {
        state.sessions.remove_session(&sid);
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// App info endpoints
// ---------------------------------------------------------------------------

/// Advertised application version. *arr apps gate features on this string, so
/// it stays on the 5.x line that matches the advertised WebAPI version below.
const COMPAT_APP_VERSION: &str = "v5.0.0";

async fn h_app_version() -> &'static str {
    COMPAT_APP_VERSION
}

async fn h_app_webapi_version() -> &'static str {
    "2.11.3"
}

async fn h_app_default_save_path(State(state): State<Arc<QbitState>>) -> String {
    state.api_state.api.api_output_folder()
}

async fn h_app_build_info() -> impl IntoResponse {
    axum::Json(QbitBuildInfo {
        qt: "N/A",
        libtorrent: "N/A",
        boost: "N/A",
        openssl: "N/A",
        bitness: 64,
    })
}

async fn h_app_preferences(State(state): State<Arc<QbitState>>) -> axum::Json<QbitPreferences> {
    let save_path = state.api_state.api.api_output_folder();
    let session = state.api_state.api.session();
    let (download, upload) = session.normal_rate_limits();
    let alt = session.alt_speed_status().config;

    axum::Json(QbitPreferences {
        save_path,
        temp_path_enabled: false,
        temp_path: String::new(),
        max_connec: -1,
        max_connec_per_torrent: -1,
        max_uploads: -1,
        max_uploads_per_torrent: -1,
        dl_limit: pref_limit_to_qbit(download.map(|v| u64::from(v.get()))),
        up_limit: pref_limit_to_qbit(upload.map(|v| u64::from(v.get()))),
        dht: true,
        pex: true,
        lsd: true,
        encryption: 0,
        queueing_enabled: session.queueing_enabled(),
        max_active_downloads: limit_to_qbit(session.max_active_downloads()),
        max_active_uploads: limit_to_qbit(session.max_active_seeds()),
        // rtbit has no combined limit; downloads and seeds are limited separately.
        max_active_torrents: -1,
        locale: "en",
        web_ui_port: state.api_state.opts.web_ui_port.unwrap_or(3030),
        listen_port: state.api_state.api.api_listen_port(),
        announce_port: state.api_state.api.api_announce_port(),
        alt_dl_limit: pref_limit_to_qbit(alt.download_rate),
        alt_up_limit: pref_limit_to_qbit(alt.upload_rate),
        scheduler_enabled: false,
        max_ratio_enabled: false,
        max_ratio: 2.0,
        max_ratio_act: 0,
        max_seeding_time_enabled: false,
        max_seeding_time: 1440,
    })
}

/// `app/setPreferences`. `announce_port`, the queue settings
/// (`queueing_enabled`, `max_active_downloads`, `max_active_uploads`) and the
/// normal and alternative speed limits (`dl_limit`, `up_limit`,
/// `alt_dl_limit`, `alt_up_limit`, bytes/s, 0 or -1 unlimited) are applied;
/// other keys are ignored. `announce_port`, when present, must be a
/// valid port. Absent keys leave their setting alone, so
/// read-modify-write clients that post their whole preferences object work.
async fn h_app_set_preferences(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let Ok(form) = serde_urlencoded::from_bytes::<SetPreferencesForm>(&body) else {
        return qbit_error(
            StatusCode::BAD_REQUEST,
            "preferences form field `json` is required",
        );
    };
    let Ok(preferences) = serde_json::from_str::<SetPreferences>(&form.json) else {
        return qbit_error(
            StatusCode::BAD_REQUEST,
            "preferences `json` is not a valid object",
        );
    };
    if let Some(announce_port) = preferences.announce_port {
        let Some(port) = NonZeroU16::new(announce_port) else {
            return qbit_error(
                StatusCode::BAD_REQUEST,
                "announce_port must be between 1 and 65535",
            );
        };
        state.api_state.api.api_set_announce_port(port);
    }
    let session = state.api_state.api.session();
    let current = (session.max_active_downloads(), session.max_active_seeds());
    if let Some((downloads, seeds)) = queue_limits_update(&preferences, current)
        && let Err(error) = session.set_queue_limits(downloads, seeds).await
    {
        warn!(%error, "qbit compat: error saving queue limits");
    }
    if preferences.dl_limit.is_some() || preferences.up_limit.is_some() {
        let (download, upload) = session.normal_rate_limits();
        let bps = |limit: i64| pref_limit(limit).and_then(limit_to_bps);
        session.set_normal_rate_limits(
            preferences.dl_limit.map_or(download, bps),
            preferences.up_limit.map_or(upload, bps),
        );
    }
    if preferences.alt_dl_limit.is_some() || preferences.alt_up_limit.is_some() {
        let current = session.alt_speed_status().config;
        session.set_alt_speed_config(AltSpeedConfig {
            download_rate: preferences
                .alt_dl_limit
                .map_or(current.download_rate, pref_limit),
            upload_rate: preferences
                .alt_up_limit
                .map_or(current.upload_rate, pref_limit),
        });
    }
    (StatusCode::OK, String::new()).into_response()
}

// ---------------------------------------------------------------------------
// Transfer info
// ---------------------------------------------------------------------------

async fn h_transfer_info(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let session_stats = state.api_state.api.api_session_stats();
    let config = state.api_state.api.session().ratelimits.get_config();
    axum::Json(QbitTransferInfo {
        dl_info_speed: session_stats.download_speed.as_bytes(),
        dl_info_data: session_stats.counters.fetched_bytes,
        up_info_speed: session_stats.upload_speed.as_bytes(),
        up_info_data: session_stats.counters.uploaded_bytes,
        dl_rate_limit: bps_to_u64(config.download_bps),
        up_rate_limit: bps_to_u64(config.upload_bps),
        dht_nodes: dht_nodes(&state),
        connection_status: "connected",
    })
}

/// Nodes in the DHT routing tables (IPv4 + IPv6); 0 when DHT is disabled.
fn dht_nodes(state: &QbitState) -> u64 {
    state.api_state.api.api_dht_stats().map_or(0, |s| {
        (s.routing_table_size + s.routing_table_size_v6) as u64
    })
}

fn bps_to_u64(bps: Option<NonZeroU32>) -> u64 {
    bps.map_or(0, |v| u64::from(v.get()))
}

/// Parse a qBittorrent byte-rate limit (0 means unlimited).
fn limit_to_bps(limit: u64) -> Option<NonZeroU32> {
    NonZeroU32::new(u32::try_from(limit).unwrap_or(u32::MAX))
}

#[derive(Deserialize, Default)]
struct LimitForm {
    #[serde(default)]
    limit: u64,
}

#[derive(Deserialize, Default)]
struct ModeForm {
    #[serde(default)]
    mode: u8,
}

async fn h_transfer_download_limit(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let config = state.api_state.api.session().ratelimits.get_config();
    axum::Json(bps_to_u64(config.download_bps))
}

async fn h_transfer_upload_limit(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let config = state.api_state.api.session().ratelimits.get_config();
    axum::Json(bps_to_u64(config.upload_bps))
}

async fn h_transfer_set_download_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: LimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let session = state.api_state.api.session();
    let (_, upload) = session.normal_rate_limits();
    session.set_normal_rate_limits(limit_to_bps(form.limit), upload);
    "Ok."
}

async fn h_transfer_set_upload_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: LimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let session = state.api_state.api.session();
    let (download, _) = session.normal_rate_limits();
    session.set_normal_rate_limits(download, limit_to_bps(form.limit));
    "Ok."
}

#[derive(Serialize)]
struct QbitSpeedLimits {
    up_limit: u64,
    dl_limit: u64,
    alt_up_limit: u64,
    alt_dl_limit: u64,
}

#[derive(Deserialize)]
struct SpeedLimitsForm {
    up_limit: u64,
    dl_limit: u64,
    alt_up_limit: u64,
    alt_dl_limit: u64,
}

/// `transfer/getSpeedLimits` — normal and alternative session limits in
/// bytes/s (0 = unlimited), over the native limiter and `/speed/alt` config.
async fn h_transfer_get_speed_limits(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let session = state.api_state.api.session();
    let (download, upload) = session.normal_rate_limits();
    let alt = session.alt_speed_status().config;
    axum::Json(QbitSpeedLimits {
        up_limit: bps_to_u64(upload),
        dl_limit: bps_to_u64(download),
        alt_up_limit: alt.upload_rate.unwrap_or(0),
        alt_dl_limit: alt.download_rate.unwrap_or(0),
    })
}

/// `transfer/setSpeedLimits` — sets all four limits at once; every field is
/// required, as upstream.
async fn h_transfer_set_speed_limits(State(state): State<Arc<QbitState>>, body: Bytes) -> Response {
    let Ok(form) = serde_urlencoded::from_bytes::<SpeedLimitsForm>(&body) else {
        return qbit_error(
            StatusCode::BAD_REQUEST,
            "up_limit, dl_limit, alt_up_limit and alt_dl_limit are required integers",
        );
    };
    let session = state.api_state.api.session();
    session.set_normal_rate_limits(limit_to_bps(form.dl_limit), limit_to_bps(form.up_limit));
    session.set_alt_speed_config(AltSpeedConfig {
        download_rate: (form.alt_dl_limit != 0).then_some(form.alt_dl_limit),
        upload_rate: (form.alt_up_limit != 0).then_some(form.alt_up_limit),
    });
    (StatusCode::OK, "Ok.").into_response()
}

/// qBittorrent alternative-speed mode maps onto our alt-speed toggle: `1` when
/// alternative limits are active, `0` otherwise.
async fn h_transfer_speed_limits_mode(State(state): State<Arc<QbitState>>) -> &'static str {
    if state.api_state.api.session().alt_speed_enabled() {
        "1"
    } else {
        "0"
    }
}

async fn h_transfer_toggle_speed_limits_mode(State(state): State<Arc<QbitState>>) -> &'static str {
    let session = state.api_state.api.session();
    session.set_alt_speed_enabled(!session.alt_speed_enabled());
    "Ok."
}

async fn h_transfer_set_speed_limits_mode(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: ModeForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    state
        .api_state
        .api
        .session()
        .set_alt_speed_enabled(form.mode != 0);
    "Ok."
}

/// `transfer/pauseSession` — qBittorrent pauses the whole session; we have no
/// global pause, so we pause every torrent.
async fn h_transfer_pause_session(State(state): State<Arc<QbitState>>) -> &'static str {
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, "all") {
        if let Err(error) = api.api_torrent_action_pause(idx).await {
            warn!(%error, "qbit compat: error pausing session torrent");
        }
    }
    "Ok."
}

/// `transfer/resumeSession` — resume every torrent (see pauseSession).
async fn h_transfer_resume_session(State(state): State<Arc<QbitState>>) -> &'static str {
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, "all") {
        if let Err(error) = api.api_torrent_action_start(idx).await {
            warn!(%error, "qbit compat: error resuming session torrent");
        }
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// Torrent management endpoints
// ---------------------------------------------------------------------------

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Map rtbit torrent state to qBittorrent state string.
///
/// We advertise WebAPI 2.11.3, so we emit the post-2.11 `stoppedDL`/`stoppedUP`
/// names (renamed from `pausedDL`/`pausedUP` in 2.11); clients that switch on
/// the advertised version expect these.
///
/// A torrent the queue paused reports `queuedDL`/`queuedUP` rather than
/// stopped, and a force-started live torrent reports `forcedDL`/`forcedUP`.
fn map_state(state: TorrentStatsState, finished: bool, queued: bool, forced: bool) -> &'static str {
    match (state, finished) {
        (TorrentStatsState::Initializing, _) => "metaDL",
        (TorrentStatsState::Live, false) if forced => "forcedDL",
        (TorrentStatsState::Live, true) if forced => "forcedUP",
        (TorrentStatsState::Live, false) => "downloading",
        (TorrentStatsState::Live, true) => "uploading",
        (TorrentStatsState::Paused, false) if queued => "queuedDL",
        (TorrentStatsState::Paused, true) if queued => "queuedUP",
        (TorrentStatsState::Paused, false) => "stoppedDL",
        (TorrentStatsState::Paused, true) => "stoppedUP",
        (TorrentStatsState::Error, _) => "error",
    }
}

#[derive(Deserialize, Default)]
struct TorrentsInfoQuery {
    filter: Option<String>,
    category: Option<String>,
    tag: Option<String>,
    hashes: Option<String>,
    sort: Option<String>,
    reverse: Option<bool>,
    limit: Option<usize>,
    offset: Option<usize>,
}

/// Check if a torrent matches the qBit filter.
fn matches_filter(
    filter: &str,
    qbit_state: &str,
    stats: &crate::torrent_state::stats::TorrentStats,
) -> bool {
    match filter {
        "all" => true,
        "downloading" => matches!(
            qbit_state,
            "downloading" | "metaDL" | "forcedDL" | "queuedDL"
        ),
        "seeding" => matches!(qbit_state, "uploading" | "forcedUP" | "queuedUP"),
        "completed" => stats.finished,
        // `paused` was renamed to `stopped` in 2.11; accept both spellings.
        "paused" | "stopped" => qbit_state == "stoppedDL" || qbit_state == "stoppedUP",
        "active" => is_active_state(qbit_state),
        "inactive" => !is_active_state(qbit_state),
        // `resumed` was renamed to `running` in 2.11; accept both spellings.
        "resumed" | "running" => qbit_state != "stoppedDL" && qbit_state != "stoppedUP",
        "stalled" | "stalled_uploading" | "stalled_downloading" => {
            matches!(stats.state, TorrentStatsState::Live)
                && stats
                    .live
                    .as_ref()
                    .map(|l| l.download_speed.mbps < 0.001 && l.upload_speed.mbps < 0.001)
                    .unwrap_or(true)
        }
        "errored" => qbit_state == "error",
        _ => true,
    }
}

fn is_active_state(qbit_state: &str) -> bool {
    matches!(
        qbit_state,
        "downloading" | "uploading" | "forcedDL" | "forcedUP"
    )
}

/// Build the qBittorrent `torrents/info` view of a single torrent. Shared by
/// `torrents/info` and `sync/maindata`.
fn build_torrent_info(
    handle: &crate::torrent_state::ManagedTorrentHandle,
    stats: &crate::torrent_state::stats::TorrentStats,
    now: u64,
) -> QbitTorrentInfo {
    let info_hash = handle.shared().info_hash.as_string();
    let name = handle
        .name()
        .unwrap_or_else(|| format!("torrent_{}", handle.id()));
    let output_folder = qbit_save_path(handle);
    let content_path = qbit_content_path(handle, &name);
    let forced = handle.is_force_started();
    let qbit_state = map_state(stats.state, stats.finished, handle.is_queued(), forced);
    let limits = handle.rate_limits();
    let category = handle.shared().category.read().clone().unwrap_or_default();
    let share_limits = *handle.shared().share_limits.read();
    let ratio_limit = ratio_to_qbit(share_limits.max_ratio);
    let seeding_time_limit = minutes_to_qbit(share_limits.max_seeding_time);
    let inactive_seeding_time_limit = minutes_to_qbit(share_limits.max_inactive_seeding_time);

    let dl_speed = stats
        .live
        .as_ref()
        .map(|l| l.download_speed.as_bytes())
        .unwrap_or(0);
    let up_speed = stats
        .live
        .as_ref()
        .map(|l| l.upload_speed.as_bytes())
        .unwrap_or(0);

    let progress = if stats.total_bytes > 0 {
        stats.progress_bytes as f64 / stats.total_bytes as f64
    } else {
        0.0
    };

    let eta = stats
        .total_bytes
        .saturating_sub(stats.progress_bytes)
        .checked_div(dl_speed)
        .map(|seconds| i64::try_from(seconds.min(8_640_000)).unwrap_or(8_640_000))
        .unwrap_or(8_640_000);

    let num_seeds = stats
        .live
        .as_ref()
        .map(|l| l.snapshot.connected_seeders)
        .unwrap_or(0);
    let num_leechs = stats
        .live
        .as_ref()
        .map(|l| l.snapshot.connected_leechers)
        .unwrap_or(0);
    let added_on = handle.shared().added_on;
    let completion_on = handle
        .shared()
        .completion_on
        .load(std::sync::atomic::Ordering::Relaxed);
    let ratio = if stats.progress_bytes == 0 {
        0.0
    } else {
        stats.uploaded_bytes as f64 / stats.progress_bytes as f64
    };
    let time_active = now.saturating_sub(added_on);
    let seeding_time = completion_on
        .checked_sub(added_on)
        .map_or(0, |_| now.saturating_sub(completion_on));

    let tracker = handle
        .shared()
        .trackers
        .read()
        .iter()
        .next()
        .map(|u| u.to_string())
        .unwrap_or_default();

    let trackers_count = handle.shared().trackers.read().len();

    QbitTorrentInfo {
        added_on,
        amount_left: stats.total_bytes.saturating_sub(stats.progress_bytes),
        auto_tmm: false,
        availability: -1,
        category,
        completed: stats.progress_bytes,
        completion_on: if completion_on > 0 {
            i64::try_from(completion_on).unwrap_or(i64::MAX)
        } else {
            -1
        },
        content_path,
        dl_limit: qbit_torrent_limit(limits.download_bps),
        dlspeed: dl_speed,
        download_path: String::new(),
        downloaded: stats.progress_bytes,
        downloaded_session: 0,
        eta,
        f_l_piece_prio: false,
        force_start: forced,
        hash: info_hash.clone(),
        inactive_seeding_time_limit,
        infohash_v1: info_hash,
        infohash_v2: String::new(),
        last_activity: if completion_on > 0 {
            completion_on
        } else {
            added_on
        },
        max_inactive_seeding_time: inactive_seeding_time_limit,
        magnet_uri: String::new(),
        max_ratio: ratio_limit,
        max_seeding_time: seeding_time_limit,
        name,
        num_complete: num_seeds,
        num_incomplete: num_leechs,
        num_leechs,
        num_seeds,
        priority: 0,
        progress,
        ratio,
        ratio_limit,
        save_path: output_folder,
        seeding_time,
        seeding_time_limit,
        seen_complete: if completion_on > 0 {
            i64::try_from(completion_on).unwrap_or(i64::MAX)
        } else {
            -1
        },
        seq_dl: false,
        size: stats.total_bytes,
        state: qbit_state.to_string(),
        super_seeding: false,
        tags: handle
            .shared()
            .tags
            .read()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        time_active,
        total_size: stats.total_bytes,
        tracker,
        trackers_count,
        up_limit: qbit_torrent_limit(limits.upload_bps),
        uploaded: stats.uploaded_bytes,
        uploaded_session: stats.uploaded_bytes,
        upspeed: up_speed,
    }
}

/// Apply qBittorrent `offset`/`limit` pagination. An offset at or past the end
/// yields an empty list (not the whole list); a `limit` of 0 means "no limit".
fn apply_offset_limit<T>(mut items: Vec<T>, offset: usize, limit: Option<usize>) -> Vec<T> {
    if offset >= items.len() {
        return Vec::new();
    }
    if offset > 0 {
        items = items.split_off(offset);
    }
    if let Some(limit) = limit
        && limit > 0
    {
        items.truncate(limit);
    }
    items
}

async fn h_torrents_info(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<TorrentsInfoQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let now = now_unix();

    let hash_filter: Option<Vec<String>> = query
        .hashes
        .as_ref()
        .map(|h| h.split('|').map(|s| s.to_lowercase()).collect());

    let priorities = queue_priorities(api);
    let mut torrents: Vec<QbitTorrentInfo> = api.session().with_torrents(|iter| {
        iter.filter_map(|(_id, handle)| {
            let stats = handle.stats();
            let mut info = build_torrent_info(handle, &stats, now);
            info.priority = priorities.get(&handle.id()).copied().unwrap_or(0);

            // Filter by hash if specified.
            if let Some(ref hashes) = hash_filter
                && !hashes.contains(&info.hash)
            {
                return None;
            }

            // qBittorrent treats an empty category and "uncategorized" as the
            // uncategorized bucket; otherwise category matching is exact.
            if let Some(ref requested) = query.category
                && !matches_category(requested, &info.category)
            {
                return None;
            }

            // An empty tag query selects untagged torrents; otherwise the
            // torrent must carry that tag.
            if let Some(ref tag) = query.tag
                && !matches_tag(tag, &handle.shared().tags.read())
            {
                return None;
            }

            // Apply the state filter.
            if let Some(ref filter) = query.filter
                && !matches_filter(filter, &info.state, &stats)
            {
                return None;
            }

            Some(info)
        })
        .collect()
    });

    // Sort
    if let Some(ref sort_field) = query.sort {
        let reverse = query.reverse.unwrap_or(false);
        torrents.sort_by(|a, b| {
            let cmp = match sort_field.as_str() {
                "name" => a.name.cmp(&b.name),
                "size" | "total_size" => a.total_size.cmp(&b.total_size),
                "progress" => a
                    .progress
                    .partial_cmp(&b.progress)
                    .unwrap_or(std::cmp::Ordering::Equal),
                "dlspeed" => a.dlspeed.cmp(&b.dlspeed),
                "upspeed" => a.upspeed.cmp(&b.upspeed),
                "eta" => a.eta.cmp(&b.eta),
                "state" => a.state.cmp(&b.state),
                "added_on" => a.added_on.cmp(&b.added_on),
                "hash" => a.hash.cmp(&b.hash),
                "downloaded" => a.downloaded.cmp(&b.downloaded),
                "uploaded" => a.uploaded.cmp(&b.uploaded),
                "ratio" => a
                    .ratio
                    .partial_cmp(&b.ratio)
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => std::cmp::Ordering::Equal,
            };
            if reverse { cmp.reverse() } else { cmp }
        });
    }

    let torrents = apply_offset_limit(torrents, query.offset.unwrap_or(0), query.limit);
    axum::Json(torrents)
}

fn matches_tag(requested: &str, tags: &BTreeSet<String>) -> bool {
    if requested.is_empty() {
        tags.is_empty()
    } else {
        tags.contains(requested)
    }
}

fn matches_category(requested: &str, category: &str) -> bool {
    match requested {
        "all" => true,
        "uncategorized" | "" => category.is_empty(),
        value => category == value,
    }
}

#[derive(Deserialize)]
struct HashQuery {
    hash: String,
}

/// Descriptive fields from the top level of a `.torrent` file.
struct TorrentFileMetadata {
    comment: String,
    created_by: String,
    /// Unix seconds; -1 when the file does not say.
    creation_date: i64,
}

impl Default for TorrentFileMetadata {
    fn default() -> Self {
        Self {
            comment: String::new(),
            created_by: String::new(),
            creation_date: -1,
        }
    }
}

fn torrent_file_metadata(torrent_bytes: &[u8]) -> TorrentFileMetadata {
    // Torrents added from a magnet have no outer dictionary to read.
    let Ok(meta) = librtbit_core::torrent_metainfo::torrent_from_bytes(torrent_bytes) else {
        return TorrentFileMetadata::default();
    };
    let text = |buf: Option<buffers::ByteBuf<'_>>| {
        buf.map(|b| String::from_utf8_lossy(b.as_ref()).into_owned())
            .unwrap_or_default()
    };
    TorrentFileMetadata {
        comment: text(meta.comment),
        created_by: text(meta.created_by),
        creation_date: meta
            .creation_date
            .and_then(|date| i64::try_from(date).ok())
            .unwrap_or(-1),
    }
}

async fn h_torrents_properties(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let stats = handle.stats();
    let output_folder = qbit_save_path(&handle);
    let now = now_unix();
    let added_on = handle.shared().added_on;
    let completion_on = handle
        .shared()
        .completion_on
        .load(std::sync::atomic::Ordering::Relaxed);
    let ratio = if stats.progress_bytes == 0 {
        0.0
    } else {
        stats.uploaded_bytes as f64 / stats.progress_bytes as f64
    };

    let dl_speed = stats
        .live
        .as_ref()
        .map(|l| l.download_speed.as_bytes())
        .unwrap_or(0);
    let up_speed = stats
        .live
        .as_ref()
        .map(|l| l.upload_speed.as_bytes())
        .unwrap_or(0);

    let eta = stats
        .total_bytes
        .saturating_sub(stats.progress_bytes)
        .checked_div(dl_speed)
        .map(|seconds| i64::try_from(seconds).unwrap_or(8_640_000))
        .unwrap_or(8_640_000);

    let piece_size = handle
        .with_metadata(|m| m.info.lengths().default_piece_length() as u64)
        .unwrap_or(0);
    let pieces_num = handle
        .with_metadata(|m| m.info.lengths().total_pieces())
        .unwrap_or(0);
    let meta = handle
        .with_metadata(|m| torrent_file_metadata(&m.torrent_bytes))
        .unwrap_or_default();

    axum::Json(QbitTorrentProperties {
        save_path: output_folder,
        creation_date: meta.creation_date,
        piece_size,
        comment: meta.comment,
        total_wasted: 0,
        total_uploaded: stats.uploaded_bytes,
        total_uploaded_session: stats.uploaded_bytes,
        total_downloaded: stats.progress_bytes,
        total_downloaded_session: stats.progress_bytes,
        up_limit: qbit_torrent_limit(handle.rate_limits().upload_bps),
        dl_limit: qbit_torrent_limit(handle.rate_limits().download_bps),
        time_elapsed: now.saturating_sub(added_on),
        seeding_time: if completion_on > 0 {
            now.saturating_sub(completion_on)
        } else {
            0
        },
        nb_connections: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.peer_stats.live)
            .unwrap_or(0),
        nb_connections_limit: -1,
        share_ratio: ratio,
        addition_date: added_on,
        completion_date: if completion_on > 0 {
            i64::try_from(completion_on).unwrap_or(i64::MAX)
        } else {
            -1
        },
        created_by: meta.created_by,
        dl_speed_avg: dl_speed,
        dl_speed,
        eta,
        last_seen: if completion_on > 0 {
            completion_on
        } else {
            added_on
        },
        peers: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_leechers)
            .unwrap_or(0),
        peers_total: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_leechers)
            .unwrap_or(0),
        pieces_have: stats
            .progress_bytes
            .checked_div(piece_size)
            .map(|pieces| u32::try_from(pieces).unwrap_or(u32::MAX))
            .unwrap_or(0),
        pieces_num,
        reannounce: 0,
        seeds: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_seeders)
            .unwrap_or(0),
        seeds_total: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_seeders)
            .unwrap_or(0),
        total_size: stats.total_bytes,
        up_speed_avg: up_speed,
        up_speed,
    })
    .into_response()
}

async fn h_torrents_files(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let details = match api.api_torrent_details(idx) {
        Ok(d) => d,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let stats = handle.stats();
    let is_seed = stats.finished;

    let details_name = details.name.clone();
    let details_files = details.files.unwrap_or_default();
    let qbit_root = handle.shared().options.output_folder_root.is_some();
    let is_multi_file = handle
        .with_metadata(|metadata| metadata.info.info().files.is_some())
        .unwrap_or(false);
    // Source per-file names from file_infos (which reflects renames) when its
    // count lines up with the details file list; otherwise fall back to the
    // metadata names to avoid any index skew (e.g. padding files).
    let rel_names: Vec<String> = handle
        .with_metadata(|metadata| {
            metadata
                .file_infos
                .iter()
                .map(|fi| fi.relative_filename.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let use_rel_names = rel_names.len() == details_files.len();
    let files: Vec<QbitFileInfo> = details_files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let file_progress = stats.file_progress.get(i).copied().unwrap_or(0);
            let progress = if f.length > 0 {
                file_progress as f64 / f.length as f64
            } else {
                1.0
            };
            let base_name = if use_rel_names {
                rel_names[i].as_str()
            } else {
                f.name.as_str()
            };
            let name = qbit_file_name(details_name.as_deref(), base_name, is_multi_file, qbit_root);
            QbitFileInfo {
                index: i,
                name,
                size: f.length,
                progress,
                priority: if f.included { 1 } else { 0 },
                is_seed,
                piece_range: [0, 0],
                availability: if progress >= 1.0 { 1.0 } else { progress },
            }
        })
        .collect();

    axum::Json(files).into_response()
}

#[derive(Serialize)]
struct QbitTrackerInfo {
    url: String,
    /// qBittorrent tracker status: 0 disabled, 1 not contacted, 2 working,
    /// 3 updating, 4 not working.
    status: u8,
    tier: i32,
    num_peers: i64,
    num_seeds: i64,
    num_leeches: i64,
    num_downloaded: i64,
    msg: String,
}

/// Map our tracker announce state to qBittorrent's integer status code.
fn qbit_tracker_status(state: tracker_comms::TrackerAnnounceState) -> u8 {
    use tracker_comms::TrackerAnnounceState::*;
    match state {
        Disabled => 0,
        NotContacted => 1,
        Working => 2,
        Updating => 3,
        Error => 4,
    }
}

async fn h_torrents_trackers(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let response = match api.api_tracker_status(idx) {
        Ok(r) => r,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let trackers: Vec<QbitTrackerInfo> = response
        .trackers
        .into_iter()
        .map(|t| QbitTrackerInfo {
            status: qbit_tracker_status(t.state),
            tier: 0,
            num_peers: t.peers_returned.map_or(-1, i64::from),
            num_seeds: t.seeders.map_or(-1, i64::from),
            num_leeches: t.leechers.map_or(-1, i64::from),
            num_downloaded: -1,
            msg: t.last_error.unwrap_or_default(),
            url: t.url,
        })
        .collect();
    axum::Json(trackers).into_response()
}

async fn h_torrents_piece_states(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let (bf, len) = match api.api_dump_haves(idx) {
        Ok(v) => v,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    // qBittorrent: 0 not downloaded, 1 downloading (requested), 2 downloaded.
    // We only distinguish have/not-have, so emit 0 or 2.
    let states: Vec<u8> = bf
        .iter()
        .take(len as usize)
        .map(|b| if *b { 2 } else { 0 })
        .collect();
    axum::Json(states).into_response()
}

async fn h_torrents_piece_hashes(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let hashes: Option<Vec<String>> = handle
        .with_metadata(|m| {
            let info = m.info.info();
            let total = m.info.lengths().total_pieces();
            (0..total)
                .map(|p| {
                    info.get_hash(p).map(|h| {
                        h.iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    })
                })
                .collect::<Option<Vec<String>>>()
        })
        .ok()
        .flatten();
    match hashes {
        Some(hashes) => axum::Json(hashes).into_response(),
        // Metadata not yet available (magnet still resolving) or v2-only torrent.
        None => axum::Json(Vec::<String>::new()).into_response(),
    }
}

async fn h_torrents_count(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let count = state
        .api_state
        .api
        .session()
        .with_torrents(|iter| iter.count());
    axum::Json(count)
}

/// `torrents/export` — return the raw `.torrent` file bytes for one torrent.
async fn h_torrents_export(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    match handle.with_metadata(|meta| meta.torrent_bytes.clone()) {
        Ok(bytes) => ([("content-type", "application/x-bittorrent")], bytes).into_response(),
        // Metadata not resolved yet (magnet) — no file to export.
        Err(_) => qbit_error(StatusCode::CONFLICT, "Metadata not available"),
    }
}

#[derive(Serialize)]
struct QbitWebSeed {
    url: String,
}

async fn h_torrents_webseeds(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let seeds: Vec<QbitWebSeed> = handle
        .shared()
        .web_seed_urls
        .iter()
        .map(|url| QbitWebSeed { url: url.clone() })
        .collect();
    axum::Json(seeds).into_response()
}

// ---------------------------------------------------------------------------
// Torrent actions (add, pause, resume, delete)
// ---------------------------------------------------------------------------

async fn h_torrents_add(
    State(state): State<Arc<QbitState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut urls: Vec<String> = Vec::new();
    let mut torrent_bytes: Vec<Bytes> = Vec::new();
    let mut savepath: Option<String> = None;
    let mut category: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut ratelimits = LimitsConfig::default();
    let mut paused = false;

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "urls" => {
                if let Ok(text) = field.text().await {
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && urls.len() < 100 {
                            urls.push(trimmed.to_string());
                        }
                    }
                }
            }
            "torrents" => {
                if torrent_bytes.len() >= 100 {
                    continue;
                }
                if let Ok(data) = field.bytes().await
                    && !data.is_empty()
                {
                    torrent_bytes.push(data);
                }
            }
            "category" => {
                if let Ok(text) = field.text().await
                    && !text.is_empty()
                {
                    category = Some(text);
                }
            }
            "tags" => {
                if let Ok(text) = field.text().await {
                    tags = parse_tags(&text);
                }
            }
            // Per-torrent limits in bytes per second; 0 or -1 is unlimited.
            "dlLimit" | "upLimit" => {
                let is_download = field_name == "dlLimit";
                if let Ok(text) = field.text().await
                    && let Ok(limit) = text.trim().parse::<i64>()
                {
                    let bps = parse_torrent_limit(limit);
                    if is_download {
                        ratelimits.download_bps = bps;
                    } else {
                        ratelimits.upload_bps = bps;
                    }
                }
            }
            "savepath" => {
                if let Ok(text) = field.text().await
                    && !text.is_empty()
                {
                    savepath = Some(text);
                }
            }
            // `paused` was renamed to `stopped` in WebAPI 2.11; accept both.
            "paused" | "stopped" => {
                if let Ok(text) = field.text().await
                    && text.eq_ignore_ascii_case("true")
                {
                    paused = true;
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let api = &state.api_state.api;
    let mut had_error = false;

    for url in urls {
        let opts = AddTorrentOptions {
            paused,
            overwrite: true,
            output_folder_root: savepath.clone(),
            category: category.clone(),
            tags: tags.clone(),
            ratelimits,
            ..Default::default()
        };
        if let Err(e) = api
            .api_add_torrent(AddTorrent::Url(url.into()), Some(opts))
            .await
        {
            warn!("qbit compat: error adding torrent URL: {e:#}");
            had_error = true;
        }
    }

    for data in torrent_bytes {
        let opts = AddTorrentOptions {
            paused,
            overwrite: true,
            output_folder_root: savepath.clone(),
            category: category.clone(),
            tags: tags.clone(),
            ratelimits,
            ..Default::default()
        };
        if let Err(e) = api
            .api_add_torrent(AddTorrent::TorrentFileBytes(data), Some(opts))
            .await
        {
            warn!("qbit compat: error adding torrent file: {e:#}");
            had_error = true;
        }
    }

    if had_error {
        (StatusCode::INTERNAL_SERVER_ERROR, "Error adding torrent(s)")
    } else {
        (StatusCode::OK, "Ok.")
    }
}

#[derive(Deserialize, Default)]
struct HashesForm {
    #[serde(default)]
    hashes: String,
}

/// Resolve hash(es) from form body. "all" means all torrents.
fn resolve_hashes(api: &Api, hashes_str: &str) -> Vec<TorrentIdOrHash> {
    if hashes_str == "all" {
        api.session().with_torrents(|iter| {
            iter.map(|(_, handle)| TorrentIdOrHash::Hash(handle.shared().info_hash))
                .collect()
        })
    } else {
        hashes_str
            .split('|')
            .filter_map(|h| {
                let h = h.trim();
                if h.is_empty() {
                    return None;
                }
                TorrentIdOrHash::parse(h).ok()
            })
            .collect()
    }
}

async fn h_torrents_pause(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);

    for idx in hashes {
        if let Err(e) = api.api_torrent_action_pause(idx).await {
            warn!("qbit compat: error pausing torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

async fn h_torrents_resume(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);

    for idx in hashes {
        if let Err(e) = api.api_torrent_action_start(idx).await {
            warn!("qbit compat: error resuming torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

/// `torrents/stop` — the WebAPI 2.11+ name for `torrents/pause`. We advertise
/// 2.11.3, so modern clients (qbittorrent-api, newer *arr) call this.
async fn h_torrents_stop(state: State<Arc<QbitState>>, body: Bytes) -> &'static str {
    h_torrents_pause(state, body).await
}

/// `torrents/start` — the WebAPI 2.11+ name for `torrents/resume`.
async fn h_torrents_start(state: State<Arc<QbitState>>, body: Bytes) -> &'static str {
    h_torrents_resume(state, body).await
}

/// Move the torrents named by `hashes` within the session queue.
async fn move_in_queue(state: &QbitState, body: &[u8], mv: QueueMove) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(body).unwrap_or_default();
    let api = &state.api_state.api;
    let ids: Vec<_> = resolve_hashes(api, &form.hashes)
        .into_iter()
        .filter_map(|idx| api.mgr_handle(idx).ok().map(|h| h.id()))
        .collect();
    api.session().move_in_queue(&ids, mv).await;
    "Ok."
}

async fn h_torrents_top_prio(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    move_in_queue(&state, &body, QueueMove::Top).await
}

async fn h_torrents_bottom_prio(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    move_in_queue(&state, &body, QueueMove::Bottom).await
}

async fn h_torrents_increase_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    move_in_queue(&state, &body, QueueMove::Up).await
}

async fn h_torrents_decrease_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    move_in_queue(&state, &body, QueueMove::Down).await
}

#[derive(Deserialize, Default)]
struct ForceStartForm {
    #[serde(default)]
    hashes: String,
    #[serde(default)]
    value: String,
}

/// `torrents/setForceStart` — `value=true` starts the torrents outside the
/// queue limits; `value=false` hands them back to the queue.
async fn h_torrents_set_force_start(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: ForceStartForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let enabled = form.value.eq_ignore_ascii_case("true");
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        let Ok(handle) = api.mgr_handle(idx) else {
            continue;
        };
        if let Err(error) = api.session().set_force_start(&handle, enabled).await {
            warn!(%error, "qbit compat: error setting force start");
        }
    }
    "Ok."
}

/// qBittorrent's `priority` field: the 1-based queue position among
/// unfinished torrents while queueing is enabled, 0 otherwise. Seeds report 0.
fn queue_priorities(api: &Api) -> HashMap<usize, u32> {
    let session = api.session();
    if !session.queueing_enabled() {
        return HashMap::new();
    }
    session
        .queue_order()
        .into_iter()
        .filter(|&id| session.get(id.into()).is_some_and(|h| !h.stats().finished))
        .zip(1u32..)
        .collect()
}

async fn h_torrents_recheck(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        if let Err(error) = api.api_torrent_action_recheck(idx).await {
            warn!(%error, "qbit compat: error rechecking torrent");
        }
    }
    "Ok."
}

async fn h_torrents_reannounce(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            handle.reannounce();
        }
    }
    "Ok."
}

/// Relative paths of every file in the torrent, indexed by file id.
fn torrent_file_paths(handle: &crate::torrent_state::ManagedTorrentHandle) -> Vec<PathBuf> {
    handle
        .with_metadata(|metadata| {
            metadata
                .file_infos
                .iter()
                .map(|fi| fi.relative_filename.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Map a rename engine error onto qBittorrent's 409 Conflict (used for
/// "torrent must be stopped", path collisions, invalid paths, etc.).
fn rename_conflict(error: anyhow::Error) -> axum::response::Response {
    (StatusCode::CONFLICT, format!("{error:#}")).into_response()
}

#[derive(Deserialize, Default)]
struct RenamePathForm {
    #[serde(default)]
    hash: String,
    #[serde(default, alias = "oldPath")]
    old_path: String,
    #[serde(default, alias = "newPath")]
    new_path: String,
}

async fn h_torrents_rename_file(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: RenamePathForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "hash is required");
    }
    if form.old_path.is_empty() || form.new_path.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "oldPath and newPath are required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let paths = torrent_file_paths(&handle);
    let old = PathBuf::from(&form.old_path);
    let file_id = match paths.iter().position(|p| *p == old) {
        Some(id) => id,
        None => return qbit_error(StatusCode::CONFLICT, "oldPath does not exist"),
    };
    match api
        .session()
        .rename_files(&handle, &[(file_id, PathBuf::from(&form.new_path))])
        .await
    {
        Ok(()) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => rename_conflict(error),
    }
}

async fn h_torrents_rename_folder(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: RenamePathForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "hash is required");
    }
    if form.old_path.is_empty() || form.new_path.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "oldPath and newPath are required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    // Rename every file whose path is under the old folder prefix.
    let old_prefix = PathBuf::from(&form.old_path);
    let new_prefix = PathBuf::from(&form.new_path);
    let mut renames: Vec<(usize, PathBuf)> = Vec::new();
    for (id, path) in torrent_file_paths(&handle).into_iter().enumerate() {
        if let Ok(rest) = path.strip_prefix(&old_prefix) {
            renames.push((id, new_prefix.join(rest)));
        }
    }
    if renames.is_empty() {
        return qbit_error(StatusCode::CONFLICT, "no files under oldPath");
    }
    match api.session().rename_files(&handle, &renames).await {
        Ok(()) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => rename_conflict(error),
    }
}

#[derive(Deserialize, Default)]
struct RenameForm {
    #[serde(default)]
    hash: String,
    #[serde(default)]
    name: String,
}

async fn h_torrents_rename(State(state): State<Arc<QbitState>>, body: Bytes) -> impl IntoResponse {
    let form: RenameForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    if form.name.trim().is_empty() {
        return (StatusCode::CONFLICT, "name must not be empty");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    handle.set_display_name(Some(form.name));
    (StatusCode::OK, "Ok.")
}

#[derive(Deserialize, Default)]
struct DeleteForm {
    #[serde(default)]
    hashes: String,
    #[serde(default, alias = "deleteFiles")]
    delete_files: Option<String>,
}

async fn h_torrents_delete(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: DeleteForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);
    let delete_files = form
        .delete_files
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("true"));

    for idx in hashes {
        let result = if delete_files {
            api.api_torrent_action_delete(idx).await
        } else {
            api.api_torrent_action_forget(idx).await
        };
        if let Err(e) = result {
            warn!("qbit compat: error deleting torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

#[derive(Deserialize, Default)]
struct SetCategoryForm {
    #[serde(default)]
    hashes: String,
    #[serde(default)]
    category: String,
}

async fn h_torrents_set_category(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: SetCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let category = (!form.category.is_empty()).then_some(form.category);
    for idx in resolve_hashes(api, &form.hashes) {
        if let Err(error) = api.api_set_torrent_category(idx, category.clone()).await {
            warn!(%error, "qbit compat: error setting torrent category");
        }
    }
    "Ok."
}

#[derive(Deserialize, Default)]
struct AddTrackersForm {
    #[serde(default)]
    hash: String,
    #[serde(default)]
    urls: String,
}

async fn h_torrents_add_trackers(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: AddTrackersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let trackers: Vec<String> = form
        .urls
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if trackers.is_empty() {
        return (StatusCode::OK, "Ok.");
    }
    match api.api_torrent_action_add_trackers(idx, trackers).await {
        Ok(_) => (StatusCode::OK, "Ok."),
        Err(error) => {
            warn!(%error, "qbit compat: error adding trackers");
            (StatusCode::NOT_FOUND, "Not found")
        }
    }
}

#[derive(Deserialize, Default)]
struct RemoveTrackersForm {
    #[serde(default)]
    hash: String,
    /// Pipe-separated tracker URLs.
    #[serde(default)]
    urls: String,
}

async fn h_torrents_remove_trackers(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: RemoveTrackersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let urls: Vec<String> = form
        .urls
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    match api.session().remove_trackers(&handle, &urls).await {
        Ok(0) if !urls.is_empty() => qbit_error(StatusCode::CONFLICT, "All urls were not found"),
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => {
            warn!(%error, "qbit compat: error removing trackers");
            (StatusCode::OK, "Ok.").into_response()
        }
    }
}

#[derive(Deserialize, Default)]
struct EditTrackerForm {
    #[serde(default)]
    hash: String,
    #[serde(default, alias = "origUrl")]
    orig_url: String,
    #[serde(default, alias = "newUrl")]
    new_url: String,
}

async fn h_torrents_edit_tracker(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: EditTrackerForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let Some(new_url) = url::Url::parse(&form.new_url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https" | "udp"))
    else {
        return qbit_error(StatusCode::BAD_REQUEST, "invalid newUrl");
    };
    {
        let configured = handle.shared().trackers.read();
        let orig_known = url::Url::parse(&form.orig_url).is_ok_and(|u| configured.contains(&u));
        if !orig_known {
            return qbit_error(StatusCode::CONFLICT, "origUrl not found");
        }
        if configured.contains(&new_url) {
            return qbit_error(StatusCode::CONFLICT, "newUrl already exists");
        }
    }
    match api
        .session()
        .edit_tracker(&handle, &form.orig_url, &form.new_url)
        .await
    {
        Ok(()) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => {
            warn!(%error, "qbit compat: error editing tracker");
            qbit_error(StatusCode::CONFLICT, "Failed")
        }
    }
}

#[derive(Deserialize, Default)]
struct AddPeersForm {
    #[serde(default)]
    hashes: String,
    #[serde(default)]
    peers: String,
}

async fn h_torrents_add_peers(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: AddPeersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let peers: Vec<std::net::SocketAddr> = form
        .peers
        .split('|')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    for idx in resolve_hashes(api, &form.hashes) {
        let Ok(handle) = api.mgr_handle(idx) else {
            continue;
        };
        let Some(live) = handle.live() else {
            continue;
        };
        for addr in &peers {
            let _ = live.add_peer_if_not_seen(*addr);
        }
    }
    "Ok."
}

#[derive(Deserialize, Default)]
struct FilePrioForm {
    #[serde(default)]
    hash: String,
    /// Pipe-separated file indices.
    #[serde(default)]
    id: String,
    #[serde(default)]
    priority: u8,
}

async fn h_torrents_file_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: FilePrioForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let num_files = match api.api_torrent_details(idx) {
        Ok(details) => details.files.map(|f| f.len()).unwrap_or(0),
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };

    // qBittorrent priority 0 means "do not download"; anything else downloads.
    // We only model an include/exclude selection, so map onto only_files.
    let mut included: HashSet<usize> = match handle.only_files() {
        Some(files) => files.into_iter().collect(),
        None => (0..num_files).collect(),
    };
    let download = form.priority != 0;
    for id in form
        .id
        .split('|')
        .filter_map(|s| s.trim().parse::<usize>().ok())
    {
        if id >= num_files {
            return (StatusCode::CONFLICT, "Invalid file id");
        }
        if download {
            included.insert(id);
        } else {
            included.remove(&id);
        }
    }
    match api
        .api_torrent_action_update_only_files(idx, &included)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok."),
        Err(error) => {
            warn!(%error, "qbit compat: error setting file priority");
            (StatusCode::CONFLICT, "Failed")
        }
    }
}

// ---------------------------------------------------------------------------
// Per-torrent speed limits
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct HashesQuery {
    #[serde(default)]
    hashes: String,
}

#[derive(Deserialize, Default)]
struct SetTorrentLimitForm {
    #[serde(default)]
    hashes: String,
    /// Bytes/s; <= 0 means unlimited.
    #[serde(default)]
    limit: i64,
}

fn torrent_limit_map(
    api: &Api,
    hashes: &str,
    pick: impl Fn(LimitsConfig) -> Option<NonZeroU32>,
) -> HashMap<String, u64> {
    let mut map = HashMap::new();
    for idx in resolve_hashes(api, hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            let hash = handle.shared().info_hash.as_string();
            map.insert(hash, bps_to_u64(pick(handle.rate_limits())));
        }
    }
    map
}

/// A per-torrent limit as qBittorrent reports it in `torrents/info` and
/// `torrents/properties`: bytes per second, or -1 for unlimited.
fn qbit_torrent_limit(bps: Option<NonZeroU32>) -> i32 {
    bps.map_or(-1, |bps| i32::try_from(bps.get()).unwrap_or(i32::MAX))
}

/// Set one direction of the speed limit on every torrent named by `hashes`,
/// keeping the other direction, and save it with the torrent.
async fn set_torrent_limit(
    api: &Api,
    hashes: &str,
    bps: Option<NonZeroU32>,
    apply: impl Fn(&mut crate::TorrentRateLimits, Option<NonZeroU32>),
) {
    for idx in resolve_hashes(api, hashes) {
        let Ok(mut limits) = api.api_torrent_rate_limits(idx) else {
            continue;
        };
        apply(&mut limits, bps);
        if let Err(error) = api.api_set_torrent_rate_limits(idx, limits).await {
            warn!(%error, "qbit compat: error setting torrent speed limit");
        }
    }
}

fn parse_torrent_limit(limit: i64) -> Option<NonZeroU32> {
    if limit <= 0 {
        None
    } else {
        limit_to_bps(limit as u64)
    }
}

async fn h_torrents_download_limit(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashesQuery>,
) -> impl IntoResponse {
    let map = torrent_limit_map(&state.api_state.api, &query.hashes, |c| c.download_bps);
    axum::Json(map)
}

async fn h_torrents_upload_limit(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashesQuery>,
) -> impl IntoResponse {
    let map = torrent_limit_map(&state.api_state.api, &query.hashes, |c| c.upload_bps);
    axum::Json(map)
}

async fn h_torrents_set_download_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: SetTorrentLimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let bps = parse_torrent_limit(form.limit);
    set_torrent_limit(api, &form.hashes, bps, |l, bps| l.download_bps = bps).await;
    "Ok."
}

async fn h_torrents_set_upload_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: SetTorrentLimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let bps = parse_torrent_limit(form.limit);
    set_torrent_limit(api, &form.hashes, bps, |l, bps| l.upload_bps = bps).await;
    "Ok."
}

fn default_ratio_limit() -> f64 {
    -1.0
}

fn default_seeding_time_limit() -> i64 {
    -1
}

#[derive(Deserialize)]
struct SetShareLimitsForm {
    #[serde(default)]
    hashes: String,
    #[serde(default = "default_ratio_limit", alias = "ratioLimit")]
    ratio_limit: f64,
    #[serde(default = "default_seeding_time_limit", alias = "seedingTimeLimit")]
    seeding_time_limit: i64,
    #[serde(default, alias = "inactiveSeedingTimeLimit")]
    inactive_seeding_time_limit: Option<i64>,
}

/// `torrents/setShareLimits` — set per-torrent ratio, seeding-time and
/// inactive-seeding-time limits. The session's policy pass pauses a seed that
/// reaches any of them, and the limits are persisted with the torrent. An
/// omitted `inactiveSeedingTimeLimit` (older clients) keeps the current value.
async fn h_torrents_set_share_limits(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: SetShareLimitsForm = match serde_urlencoded::from_bytes(&body) {
        Ok(form) => form,
        Err(_) => return "Ok.",
    };
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        let Ok(handle) = api.mgr_handle(idx) else {
            continue;
        };
        let current = api.session().share_limits(&handle);
        let limits = ShareLimits {
            max_ratio: ratio_from_qbit(form.ratio_limit),
            max_seeding_time: minutes_from_qbit(form.seeding_time_limit),
            max_inactive_seeding_time: match form.inactive_seeding_time_limit {
                Some(raw) => minutes_from_qbit(raw),
                None => current.max_inactive_seeding_time,
            },
        };
        api.session().set_share_limits(&handle, limits).await;
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// Tag endpoints (backed by the session's tags)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct TagsForm {
    #[serde(default)]
    hashes: String,
    #[serde(default)]
    tags: String,
}

async fn h_torrents_tags(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    axum::Json(state.api_state.api.api_list_tags())
}

async fn h_torrents_create_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if let Err(error) = state
        .api_state
        .api
        .api_create_tags(&parse_tags(&form.tags))
        .await
    {
        warn!(%error, "qbit compat: error creating tags");
    }
    "Ok."
}

async fn h_torrents_delete_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if let Err(error) = state
        .api_state
        .api
        .api_delete_tags(&parse_tags(&form.tags))
        .await
    {
        warn!(%error, "qbit compat: error deleting tags");
    }
    "Ok."
}

#[derive(Clone, Copy)]
enum TagEdit {
    Add,
    Remove,
    Set,
}

/// Apply a per-torrent tag edit to every torrent named in the form.
async fn edit_torrent_tags(state: &QbitState, body: &[u8], edit: TagEdit) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(body).unwrap_or_default();
    let api = &state.api_state.api;
    let tags = parse_tags(&form.tags);
    // addTags with nothing to add is a no-op rather than an error.
    if matches!(edit, TagEdit::Add) && tags.is_empty() {
        return "Ok.";
    }
    for idx in resolve_hashes(api, &form.hashes) {
        let result = match edit {
            TagEdit::Add => api.api_add_torrent_tags(idx, &tags).await,
            TagEdit::Remove => api.api_remove_torrent_tags(idx, &tags).await,
            TagEdit::Set => api.api_set_torrent_tags(idx, &tags).await,
        };
        if let Err(error) = result {
            warn!(%error, "qbit compat: error editing torrent tags");
        }
    }
    "Ok."
}

async fn h_torrents_add_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    edit_torrent_tags(&state, &body, TagEdit::Add).await
}

async fn h_torrents_remove_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    edit_torrent_tags(&state, &body, TagEdit::Remove).await
}

async fn h_torrents_set_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    edit_torrent_tags(&state, &body, TagEdit::Set).await
}

#[derive(Deserialize, Default)]
struct SetLocationForm {
    // setLocation uses `hashes`/`location`; setSavePath uses `id`/`path`.
    #[serde(default, alias = "id")]
    hashes: String,
    #[serde(default, alias = "path")]
    location: String,
}

async fn h_torrents_set_location(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: SetLocationForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let location = form.location.trim();
    if location.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "location is required");
    }
    let new_root = PathBuf::from(location);
    let mut last_error = None;
    for idx in resolve_hashes(api, &form.hashes) {
        let Ok(handle) = api.mgr_handle(idx) else {
            continue;
        };
        if let Err(error) = api.session().set_location(&handle, new_root.clone()).await {
            last_error = Some(error);
        }
    }
    match last_error {
        Some(error) => rename_conflict(error),
        None => (StatusCode::OK, "Ok.").into_response(),
    }
}

// ---------------------------------------------------------------------------
// Category endpoints
// ---------------------------------------------------------------------------

async fn h_categories(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let map: HashMap<String, QbitCategory> = state
        .api_state
        .api
        .api_list_categories()
        .into_iter()
        .map(|(name, category)| {
            let save_path = category
                .save_path
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name.clone(), QbitCategory { name, save_path })
        })
        .collect();
    axum::Json(serde_json::to_value(&map).unwrap_or_default())
}

#[derive(Deserialize, Default)]
struct CreateCategoryForm {
    #[serde(default)]
    category: String,
    #[serde(default, alias = "savePath")]
    save_path: String,
}

async fn h_create_category(State(state): State<Arc<QbitState>>, body: Bytes) -> impl IntoResponse {
    let form: CreateCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.category.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "category name is required");
    }
    let save_path = (!form.save_path.is_empty()).then(|| form.save_path.into());
    match state
        .api_state
        .api
        .api_create_or_edit_category(form.category, save_path)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn h_edit_category(State(state): State<Arc<QbitState>>, body: Bytes) -> impl IntoResponse {
    let form: CreateCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.category.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "category name is required");
    }
    let save_path = (!form.save_path.is_empty()).then(|| form.save_path.into());
    match state
        .api_state
        .api
        .api_create_or_edit_category(form.category, save_path)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[derive(Deserialize, Default)]
struct RemoveCategoriesForm {
    #[serde(default)]
    categories: String,
}

async fn h_remove_categories(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: RemoveCategoriesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    for name in form.categories.split('\n') {
        let name = name.trim();
        if !name.is_empty()
            && let Err(error) = state.api_state.api.api_remove_category(name).await
        {
            warn!(%error, %name, "qbit compat: error removing category");
        }
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// Sync (main polling endpoint)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct QbitServerState {
    connection_status: &'static str,
    dht_nodes: u64,
    dl_info_data: u64,
    dl_info_speed: u64,
    dl_rate_limit: u64,
    up_info_data: u64,
    up_info_speed: u64,
    up_rate_limit: u64,
    queueing: bool,
    use_alt_speed_limits: bool,
    refresh_interval: u64,
    free_space_on_disk: u64,
    global_ratio: String,
}

#[derive(Serialize)]
struct QbitMainData {
    torrents: HashMap<String, QbitTorrentInfo>,
    categories: HashMap<String, QbitCategory>,
    tags: Vec<String>,
    server_state: QbitServerState,
}

#[derive(Deserialize, Default)]
struct SyncQuery {
    #[serde(default)]
    rid: u64,
}

/// `sync/maindata` — the primary polling endpoint for WebUI frontends and many
/// integrations. `rid=0` (or an unknown/expired rid) returns a `full_update`
/// snapshot; a rid from a previous response returns only what changed since
/// it (see `qbit_sync`).
async fn h_sync_maindata(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<SyncQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let now = now_unix();

    let priorities = queue_priorities(api);
    let torrents: HashMap<String, QbitTorrentInfo> = api.session().with_torrents(|iter| {
        iter.map(|(_id, handle)| {
            let stats = handle.stats();
            let mut info = build_torrent_info(handle, &stats, now);
            info.priority = priorities.get(&handle.id()).copied().unwrap_or(0);
            (info.hash.clone(), info)
        })
        .collect()
    });

    let categories: HashMap<String, QbitCategory> = api
        .api_list_categories()
        .into_iter()
        .map(|(name, category)| {
            let save_path = category
                .save_path
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name.clone(), QbitCategory { name, save_path })
        })
        .collect();

    let session_stats = api.api_session_stats();
    let config = api.session().ratelimits.get_config();
    let downloaded = session_stats.counters.fetched_bytes;
    let uploaded = session_stats.counters.uploaded_bytes;
    let global_ratio = if downloaded == 0 {
        "0.00".to_string()
    } else {
        format!("{:.2}", uploaded as f64 / downloaded as f64)
    };

    let server_state = QbitServerState {
        connection_status: "connected",
        dht_nodes: dht_nodes(&state),
        dl_info_data: downloaded,
        dl_info_speed: session_stats.download_speed.as_bytes(),
        dl_rate_limit: bps_to_u64(config.download_bps),
        up_info_data: uploaded,
        up_info_speed: session_stats.upload_speed.as_bytes(),
        up_rate_limit: bps_to_u64(config.upload_bps),
        queueing: api.session().queueing_enabled(),
        use_alt_speed_limits: api.session().alt_speed_enabled(),
        refresh_interval: 1500,
        free_space_on_disk: 0,
        global_ratio,
    };

    let snapshot = to_json_map(&QbitMainData {
        torrents,
        categories,
        tags: api.api_list_tags(),
        server_state,
    });
    axum::Json(state.sync.respond("maindata", query.rid, snapshot))
}

fn to_json_map(value: &impl Serialize) -> serde_json::Map<String, serde_json::Value> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => unreachable!("sync payloads serialize to JSON objects"),
    }
}

#[derive(Serialize)]
struct QbitPeer {
    client: String,
    connection: &'static str,
    country: String,
    country_code: String,
    dl_speed: u64,
    downloaded: u64,
    files: String,
    flags: String,
    flags_desc: String,
    ip: String,
    port: u16,
    progress: f64,
    relevance: f64,
    up_speed: u64,
    uploaded: u64,
    peer_id_client: String,
}

#[derive(Serialize)]
struct QbitTorrentPeers {
    show_flags: bool,
    peers: HashMap<String, QbitPeer>,
}

#[derive(Deserialize)]
struct SyncPeersQuery {
    hash: String,
    #[serde(default)]
    rid: u64,
}

/// `sync/torrentPeers` — live peers of one torrent, keyed by `ip:port`, with
/// the same rid delta encoding as `sync/maindata`. Only the transfer counters
/// and connection kind are known natively; client name, country, flags,
/// per-peer progress and speeds are reported empty/zero.
async fn h_sync_torrent_peers(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<SyncPeersQuery>,
) -> Response {
    let api = &state.api_state.api;
    let Ok(idx) = TorrentIdOrHash::parse(&query.hash) else {
        return qbit_error(StatusCode::NOT_FOUND, "Not found");
    };
    let Ok(handle) = api.mgr_handle(idx) else {
        return qbit_error(StatusCode::NOT_FOUND, "Not found");
    };
    // A torrent that is not live (paused, initializing) simply has no peers.
    let peers = handle
        .live()
        .map(|live| {
            live.per_peer_stats_snapshot(PeerStatsFilter::default())
                .peers
                .into_iter()
                .filter_map(|(addr, stats)| {
                    let socket: std::net::SocketAddr = addr.parse().ok()?;
                    let connection = match stats.conn_kind {
                        Some(ConnectionKind::Utp) => "μTP",
                        _ => "BT",
                    };
                    let peer = QbitPeer {
                        client: String::new(),
                        connection,
                        country: String::new(),
                        country_code: String::new(),
                        dl_speed: 0,
                        downloaded: stats.counters.fetched_bytes,
                        files: String::new(),
                        flags: String::new(),
                        flags_desc: String::new(),
                        ip: socket.ip().to_string(),
                        port: socket.port(),
                        progress: 0.0,
                        relevance: 0.0,
                        up_speed: 0,
                        uploaded: stats.counters.uploaded_bytes,
                        peer_id_client: String::new(),
                    };
                    Some((addr, peer))
                })
                .collect()
        })
        .unwrap_or_default();

    let snapshot = to_json_map(&QbitTorrentPeers {
        show_flags: true,
        peers,
    });
    let scope = format!("peers:{}", handle.info_hash().as_string());
    axum::Json(state.sync.respond(&scope, query.rid, snapshot)).into_response()
}

// ---------------------------------------------------------------------------
// Torrent creator endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct AddTaskForm {
    #[serde(default, alias = "sourcePath")]
    source_path: String,
    #[serde(default)]
    format: String,
    #[serde(default)]
    trackers: String,
    #[serde(default, alias = "pieceSize")]
    piece_size: u32,
    #[serde(default, alias = "torrentName", alias = "name")]
    name: String,
}

async fn h_torrentcreator_add_task(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    if !state.api_state.opts.allow_create {
        return (
            StatusCode::FORBIDDEN,
            "creating torrents is not enabled on this server",
        )
            .into_response();
    }
    let form: AddTaskForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.source_path.trim().is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "sourcePath is required");
    }

    let format = if form.format.is_empty() {
        "v1".to_string()
    } else {
        form.format.clone()
    };
    let task_id = state.creator.new_task(form.source_path.clone(), format);

    // Run the (synchronous) creation off the request path and record the result.
    let trackers: Vec<String> = form
        .trackers
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let name = (!form.name.trim().is_empty()).then(|| form.name.clone());
    let piece_length = (form.piece_size > 0).then_some(form.piece_size);
    let source = form.source_path.clone();
    let state = state.clone();
    let spawned_id = task_id.clone();
    tokio::spawn(async move {
        let spawner = BlockingSpawner::new(1);
        let options = CreateTorrentOptions {
            name: name.as_deref(),
            trackers,
            piece_length,
        };
        match create_torrent(std::path::Path::new(&source), options, &spawner).await {
            Ok(result) => {
                let bytes = result.as_bytes().ok();
                let info_hash = result.info_hash().as_string();
                state.creator.finish(&spawned_id, bytes, info_hash);
            }
            Err(error) => state.creator.fail(&spawned_id, format!("{error:#}")),
        }
    });

    axum::Json(serde_json::json!({ "taskID": task_id })).into_response()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatorTaskStatus {
    #[serde(rename = "taskID")]
    task_id: String,
    source_path: String,
    format: String,
    status: CreatorStatus,
    error_message: String,
    progress: f64,
}

fn creator_task_status(id: String, task: CreatorTask) -> CreatorTaskStatus {
    CreatorTaskStatus {
        progress: match task.status {
            CreatorStatus::Finished => 1.0,
            _ => 0.0,
        },
        error_message: task.error.unwrap_or_default(),
        task_id: id,
        source_path: task.source_path,
        format: task.format,
        status: task.status,
    }
}

#[derive(Deserialize, Default)]
struct OptionalTaskIdQuery {
    #[serde(default, alias = "taskID")]
    task_id: String,
}

async fn h_torrentcreator_status(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<OptionalTaskIdQuery>,
) -> axum::response::Response {
    if !query.task_id.is_empty() {
        return match state.creator.get(&query.task_id) {
            Some(task) => {
                axum::Json(vec![creator_task_status(query.task_id, task)]).into_response()
            }
            None => qbit_error(StatusCode::NOT_FOUND, "task not found"),
        };
    }
    let all: Vec<CreatorTaskStatus> = state
        .creator
        .all()
        .into_iter()
        .map(|(id, task)| creator_task_status(id, task))
        .collect();
    axum::Json(all).into_response()
}

#[derive(Deserialize)]
struct RequiredTaskIdQuery {
    #[serde(alias = "taskID")]
    task_id: String,
}

async fn h_torrentcreator_torrent_file(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<RequiredTaskIdQuery>,
) -> axum::response::Response {
    match state.creator.get(&query.task_id) {
        Some(task) if matches!(task.status, CreatorStatus::Finished) => match task.torrent_bytes {
            Some(bytes) => ([("content-type", "application/x-bittorrent")], bytes).into_response(),
            None => (StatusCode::INTERNAL_SERVER_ERROR, "no torrent bytes").into_response(),
        },
        Some(_) => qbit_error(StatusCode::CONFLICT, "task has not finished"),
        None => qbit_error(StatusCode::NOT_FOUND, "task not found"),
    }
}

#[derive(Deserialize, Default)]
struct DeleteTaskForm {
    #[serde(default, alias = "taskID")]
    task_id: String,
}

async fn h_torrentcreator_delete_task(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: DeleteTaskForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    state.creator.remove(&form.task_id);
    "Ok."
}

// ---------------------------------------------------------------------------
// Auth middleware helper
// ---------------------------------------------------------------------------

fn extract_sid(headers: &HeaderMap) -> Option<String> {
    headers
        .get(http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix("SID=").map(|s| s.to_string())
            })
        })
}

/// Plain-text error response used by every compat handler, so clients always
/// get the same shape: the status code and a short sentence in the body.
fn qbit_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, message.into()).into_response()
}

/// Rejects cross-site POSTs. Browsers always send `Origin` on cross-origin
/// POSTs, and it must name the same authority as `Host`. Requests without an
/// `Origin` (native *arr clients, curl) are not browser requests and pass.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(http::header::ORIGIN) else {
        return true;
    };
    let Some(origin) = origin.to_str().ok() else {
        return false;
    };
    let Some(host) = headers
        .get(http::header::HOST)
        .and_then(|host| host.to_str().ok())
    else {
        return false;
    };
    // `null` and anything without a scheme separator fail the split.
    origin
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .is_some_and(|authority| authority.eq_ignore_ascii_case(host))
}

async fn reject_cross_origin_posts(request: Request, next: Next) -> Response {
    if request.method() == Method::POST && !origin_allowed(request.headers()) {
        return qbit_error(StatusCode::FORBIDDEN, "cross-origin request rejected");
    }
    next.run(request).await
}

// ---------------------------------------------------------------------------
// Client data (WebUI key/value store)
// ---------------------------------------------------------------------------

type ClientDataMap = serde_json::Map<String, serde_json::Value>;

/// Opaque key/value pairs a WebUI client stores on the server. Kept in memory
/// and, when `path` is set, saved as one small JSON file on every change.
struct ClientDataStore {
    path: Option<PathBuf>,
    data: tokio::sync::Mutex<ClientDataMap>,
}

impl ClientDataStore {
    fn open(path: Option<PathBuf>) -> Self {
        let data = path
            .as_deref()
            .and_then(|path| {
                match std::fs::read(path) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .inspect_err(|error| {
                        warn!(?path, %error, "qbit compat: ignoring unreadable client data")
                    })
                    .ok(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    warn!(?path, %error, "qbit compat: cannot read client data");
                    None
                }
            }
            })
            .unwrap_or_default();
        Self {
            path,
            data: tokio::sync::Mutex::new(data),
        }
    }

    /// The requested keys that are set, or every pair when `keys` is empty.
    async fn load(&self, keys: &[String]) -> ClientDataMap {
        let data = self.data.lock().await;
        if keys.is_empty() {
            return data.clone();
        }
        keys.iter()
            .filter_map(|key| data.get(key).map(|value| (key.clone(), value.clone())))
            .collect()
    }

    /// Merge `changes` in; a `null` value removes its key.
    async fn store(&self, changes: ClientDataMap) -> std::io::Result<()> {
        let mut data = self.data.lock().await;
        for (key, value) in changes {
            if value.is_null() {
                data.remove(&key);
            } else {
                data.insert(key, value);
            }
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(&*data)?;
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    }
}

/// Where client data is saved: beside the session's JSON persistence.
fn clientdata_path() -> Option<PathBuf> {
    // Tests build many routers; keep them off the user's real data directory.
    if cfg!(test) {
        return None;
    }
    SessionPersistenceConfig::default_json_persistence_folder()
        .inspect_err(|error| debug!(%error, "qbit compat: no data directory for client data"))
        .ok()
        .map(|dir| dir.join("qbit-clientdata.json"))
}

#[derive(Deserialize, Default)]
struct ClientDataLoadQuery {
    /// JSON array of key names; absent means every key.
    #[serde(default)]
    keys: Option<String>,
}

async fn h_clientdata_load(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<ClientDataLoadQuery>,
) -> Response {
    let keys: Vec<String> = match query.keys.as_deref() {
        None | Some("") => Vec::new(),
        Some(raw) => match serde_json::from_str(raw) {
            Ok(keys) => keys,
            Err(_) => {
                return qbit_error(
                    StatusCode::BAD_REQUEST,
                    "keys must be a JSON array of strings",
                );
            }
        },
    };
    axum::Json(state.clientdata.load(&keys).await).into_response()
}

#[derive(Deserialize, Default)]
struct ClientDataStoreForm {
    /// JSON object of key/value pairs to set; `null` removes a key.
    #[serde(default)]
    data: Option<String>,
}

async fn h_clientdata_store(State(state): State<Arc<QbitState>>, body: Bytes) -> Response {
    let form: ClientDataStoreForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let Some(raw) = form.data else {
        return qbit_error(StatusCode::BAD_REQUEST, "data is required");
    };
    let Ok(changes) = serde_json::from_str::<ClientDataMap>(&raw) else {
        return qbit_error(StatusCode::BAD_REQUEST, "data must be a JSON object");
    };
    match state.clientdata.store(changes).await {
        Ok(()) => "".into_response(),
        Err(error) => {
            warn!(%error, "qbit compat: error saving client data");
            qbit_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to save client data",
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Log + peer bans
// ---------------------------------------------------------------------------

fn log_filter_default() -> bool {
    true
}

#[derive(Deserialize)]
struct LogMainQuery {
    #[serde(default = "log_filter_default")]
    normal: bool,
    #[serde(default = "log_filter_default")]
    info: bool,
    #[serde(default = "log_filter_default")]
    warning: bool,
    #[serde(default = "log_filter_default")]
    critical: bool,
    #[serde(default = "default_last_known_id")]
    last_known_id: i64,
}

#[derive(Deserialize)]
struct LogPeersQuery {
    #[serde(default = "default_last_known_id")]
    last_known_id: i64,
}

fn default_last_known_id() -> i64 {
    -1
}

/// `-1` (or any negative id) means "everything retained".
fn after_id(last_known_id: i64) -> Option<u64> {
    u64::try_from(last_known_id).ok()
}

async fn h_log_main(Query(q): Query<LogMainQuery>) -> Response {
    use crate::log_ring::LogSeverity;
    let entries = crate::log_ring::global().main_since(after_id(q.last_known_id), |s| match s {
        LogSeverity::Normal => q.normal,
        LogSeverity::Info => q.info,
        LogSeverity::Warning => q.warning,
        LogSeverity::Critical => q.critical,
    });
    axum::Json(entries).into_response()
}

async fn h_log_peers(Query(q): Query<LogPeersQuery>) -> Response {
    axum::Json(crate::log_ring::global().peers_since(after_id(q.last_known_id))).into_response()
}

#[derive(Deserialize, Default)]
struct BanPeersForm {
    /// Pipe-separated `host:port` peers; the port is ignored.
    #[serde(default)]
    peers: String,
}

/// Parse one `transfer/banPeers` entry: `ip:port`, `[v6]:port` or a bare IP.
fn parse_ban_peer(raw: &str) -> Option<std::net::IpAddr> {
    let raw = raw.trim();
    raw.parse::<std::net::SocketAddr>()
        .map(|a| a.ip())
        .or_else(|_| raw.parse::<std::net::IpAddr>())
        .ok()
}

async fn h_transfer_ban_peers(State(state): State<Arc<QbitState>>, body: Bytes) -> Response {
    let form: BanPeersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let ips: Vec<_> = form.peers.split('|').filter_map(parse_ban_peer).collect();
    if ips.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "peers is required");
    }
    state.api_state.api.session().ban_peers(ips);
    "".into_response()
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

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, path::Path, sync::Arc, time::Duration};

    use axum::response::IntoResponse;
    use bytes::Bytes;
    use http::StatusCode;
    use tower::ServiceExt;

    use crate::{
        AddTorrent, AddTorrentOptions, Api, CreateTorrentOptions, ListenerMode, Session,
        SessionOptions, create_torrent,
        http_api::{HttpApi, HttpApiOptions},
        listen::ListenerOptions,
        spawn_utils::BlockingSpawner,
        torrent_state::TorrentStatsState,
    };

    use super::{
        CreatorStatus, QbitSessions, QbitState, QbitTorrentCreator, SyncSnapshots,
        h_app_preferences, h_app_set_preferences, h_app_version, h_torrentcreator_add_task,
        h_torrents_add_tags, h_torrents_create_tags, h_torrents_delete_tags, h_torrents_info,
        h_torrents_recheck, h_torrents_remove_tags, h_torrents_set_tags, matches_category,
        origin_allowed, qbit_file_name,
    };

    async fn qbit_state() -> (Arc<QbitState>, Arc<Session>, tempfile::TempDir) {
        let output = tempfile::TempDir::with_prefix("qbit_preferences").unwrap();
        let session = Session::new_with_opts(
            output.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    mode: ListenerMode::TcpOnly,
                    listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                    announce_port: Some(4241),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = Api::new(
            session.clone(),
            None,
            #[cfg(feature = "tracing-subscriber-utils")]
            None,
        );
        let api_state = Arc::new(HttpApi::new(
            api,
            Some(HttpApiOptions {
                web_ui_port: Some(3031),
                allow_create: true,
                ..Default::default()
            }),
        ));
        (
            Arc::new(QbitState {
                api_state,
                sessions: QbitSessions::new(),
                creator: QbitTorrentCreator::default(),
                sync: SyncSnapshots::default(),
                clientdata: super::ClientDataStore::open(None),
            }),
            session,
            output,
        )
    }

    #[tokio::test]
    async fn preferences_reports_fixed_listener_and_runtime_announce_port() {
        let (state, session, _output) = qbit_state().await;

        let preferences = h_app_preferences(axum::extract::State(state)).await.0;

        assert_eq!(
            preferences.listen_port,
            session.listen_addr().unwrap().port()
        );
        assert_eq!(preferences.announce_port, 4241);
        assert_eq!(preferences.web_ui_port, 3031);
    }

    #[tokio::test]
    async fn set_preferences_updates_port_idempotently_without_rebinding_listener() {
        let (state, session, _output) = qbit_state().await;
        let listen_addr = session.listen_addr().unwrap();
        let body = Bytes::from_static(b"json=%7B%22announce_port%22%3A51234%7D");

        for _ in 0..2 {
            let response = h_app_set_preferences(axum::extract::State(state.clone()), body.clone())
                .await
                .into_response();
            assert_eq!(response.status(), StatusCode::OK);
        }

        assert_eq!(session.announce_port(), Some(51_234));
        assert_eq!(session.listen_addr(), Some(listen_addr));
    }

    #[tokio::test]
    async fn set_preferences_ignores_unsupported_fields_and_missing_port() {
        let (state, session, _output) = qbit_state().await;
        // A read-modify-write client posting its whole preferences object back.
        let body = Bytes::from(
            "json=%7B%22save_path%22%3A%22%2Ftmp%22%2C%22dl_limit%22%3A0%2C%22max_ratio%22%3A1.5%7D",
        );
        let response = h_app_set_preferences(axum::extract::State(state.clone()), body)
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(session.announce_port(), Some(4241));
    }

    #[tokio::test]
    async fn set_preferences_rejects_invalid_announce_port_and_malformed_form() {
        let (state, session, _output) = qbit_state().await;
        let invalid_bodies = [
            "json=%7B%22announce_port%22%3A0%7D",
            "json=%7B%22announce_port%22%3A65536%7D",
            "json=not-json",
            "announce_port=4241",
        ];

        for body in invalid_bodies {
            let response = h_app_set_preferences(
                axum::extract::State(state.clone()),
                Bytes::copy_from_slice(body.as_bytes()),
            )
            .await
            .into_response();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "body={body}");
        }
        assert_eq!(session.announce_port(), Some(4241));
    }

    #[tokio::test]
    async fn app_version_is_on_the_5x_line() {
        assert!(h_app_version().await.starts_with("v5."));
    }

    #[tokio::test]
    async fn preferences_report_seeding_limits_as_disabled() {
        let (state, _session, _output) = qbit_state().await;
        let preferences =
            serde_json::to_value(h_app_preferences(axum::extract::State(state)).await.0).unwrap();
        assert_eq!(preferences["max_ratio_enabled"], false);
        assert_eq!(preferences["max_seeding_time_enabled"], false);
        assert!(preferences["max_ratio"].is_f64());
        assert!(preferences["max_ratio_act"].is_u64());
        assert!(preferences["max_seeding_time"].is_i64());
    }

    #[test]
    fn sessions_slide_forward_on_use_and_prune_idle_entries() {
        let sessions = QbitSessions::with_idle_ttl(Duration::from_millis(300));
        let live = sessions.create_session();
        std::thread::sleep(Duration::from_millis(200));
        // Still inside the idle window, so this use slides the expiry forward.
        assert!(sessions.validate_session(&live));
        std::thread::sleep(Duration::from_millis(200));
        // 400ms since creation, but only 200ms since last use.
        assert!(sessions.validate_session(&live));

        let stale = sessions.create_session();
        std::thread::sleep(Duration::from_millis(400));
        // `live` and `stale` have both been idle longer than the TTL, so creating
        // a new session prunes them and only `fresh` remains.
        let fresh = sessions.create_session();
        assert_eq!(sessions.sessions.read().len(), 1);
        assert!(!sessions.sessions.read().contains_key(&stale));
        assert!(sessions.validate_session(&fresh));
    }

    #[test]
    fn purge_expired_drops_only_idle_sessions() {
        let sessions = QbitSessions::with_idle_ttl(Duration::from_millis(300));
        let stale = sessions.create_session();
        std::thread::sleep(Duration::from_millis(400));
        sessions
            .sessions
            .write()
            .insert("fresh".into(), std::time::Instant::now());
        assert_eq!(sessions.purge_expired(), 1);
        assert!(!sessions.sessions.read().contains_key(&stale));
        assert!(sessions.validate_session("fresh"));
        assert_eq!(sessions.purge_expired(), 0);
    }

    #[tokio::test]
    async fn set_preferences_partially_applies_speed_limits() {
        let (state, session, _output) = qbit_state().await;
        // Unknown keys are ignored; known keys apply.
        let json = r#"{"dl_limit":1048576,"alt_up_limit":2048,"bogus_key":[1,2],"locale":"de"}"#;
        let body = format!("json={}", urlencoding::encode(json));
        let response =
            h_app_set_preferences(axum::extract::State(state.clone()), Bytes::from(body))
                .await
                .into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let prefs = serde_json::to_value(
            h_app_preferences(axum::extract::State(state.clone()))
                .await
                .0,
        )
        .unwrap();
        assert_eq!(prefs["dl_limit"], 1_048_576);
        assert_eq!(prefs["up_limit"], 0);
        assert_eq!(prefs["alt_up_limit"], 2048);
        assert_eq!(prefs["alt_dl_limit"], 0);
        assert_eq!(session.alt_speed_status().config.upload_rate, Some(2048));

        // -1 clears a limit; untouched keys keep their values.
        let body = format!("json={}", urlencoding::encode(r#"{"dl_limit":-1}"#));
        h_app_set_preferences(axum::extract::State(state.clone()), Bytes::from(body)).await;
        let prefs =
            serde_json::to_value(h_app_preferences(axum::extract::State(state)).await.0).unwrap();
        assert_eq!(prefs["dl_limit"], 0);
        assert_eq!(prefs["alt_up_limit"], 2048);
    }

    #[test]
    fn origin_check_allows_same_origin_and_non_browser_clients() {
        let headers = |origin: Option<&str>, host: Option<&str>| {
            let mut map = http::HeaderMap::new();
            if let Some(origin) = origin {
                map.insert(http::header::ORIGIN, origin.parse().unwrap());
            }
            if let Some(host) = host {
                map.insert(http::header::HOST, host.parse().unwrap());
            }
            map
        };
        assert!(origin_allowed(&headers(None, None)));
        assert!(origin_allowed(&headers(None, Some("10.0.0.2:3030"))));
        assert!(origin_allowed(&headers(
            Some("http://10.0.0.2:3030"),
            Some("10.0.0.2:3030")
        )));
        assert!(origin_allowed(&headers(
            Some("HTTP://Example.TEST:3030/path"),
            Some("example.test:3030")
        )));
        assert!(!origin_allowed(&headers(
            Some("https://evil.example"),
            Some("10.0.0.2:3030")
        )));
        assert!(!origin_allowed(&headers(
            Some("null"),
            Some("10.0.0.2:3030")
        )));
        assert!(!origin_allowed(&headers(
            Some("http://10.0.0.2:3030"),
            None
        )));
    }

    #[tokio::test]
    async fn cross_origin_login_post_is_rejected_by_the_router() {
        let (state, _session, _output) = qbit_state().await;
        let router = super::make_qbit_router(state.api_state.clone());
        let request = http::Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("host", "10.0.0.2:3030")
            .header("origin", "https://evil.example")
            .body(axum::body::Body::from("username=a&password=b"))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn map_state_reports_queued_and_forced() {
        use super::map_state;
        let (live, paused) = (TorrentStatsState::Live, TorrentStatsState::Paused);
        assert_eq!(map_state(paused, false, true, false), "queuedDL");
        assert_eq!(map_state(paused, true, true, false), "queuedUP");
        assert_eq!(map_state(paused, false, false, false), "stoppedDL");
        assert_eq!(map_state(live, false, false, true), "forcedDL");
        assert_eq!(map_state(live, true, false, true), "forcedUP");
        assert_eq!(map_state(live, true, false, false), "uploading");
    }

    #[test]
    fn queue_preferences_map_to_limits() {
        use super::{SetPreferences, queue_limits_update};
        let prefs = |json: &str| serde_json::from_str::<SetPreferences>(json).unwrap();

        assert_eq!(queue_limits_update(&prefs("{}"), (None, None)), None);
        // Turning queueing on with no limits uses the defaults.
        assert_eq!(
            queue_limits_update(&prefs(r#"{"queueing_enabled":true}"#), (None, None)),
            Some((Some(3), Some(3)))
        );
        assert_eq!(
            queue_limits_update(
                &prefs(
                    r#"{"queueing_enabled":true,"max_active_downloads":2,"max_active_uploads":-1}"#
                ),
                (None, None)
            ),
            Some((Some(2), None))
        );
        // A limit change while queueing is on keeps the other limit.
        assert_eq!(
            queue_limits_update(&prefs(r#"{"max_active_downloads":5}"#), (Some(1), Some(4))),
            Some((Some(5), Some(4)))
        );
        // Limits sent while queueing is off do not turn it on.
        assert_eq!(
            queue_limits_update(&prefs(r#"{"max_active_downloads":5}"#), (None, None)),
            Some((None, None))
        );
        assert_eq!(
            queue_limits_update(&prefs(r#"{"queueing_enabled":false}"#), (Some(1), Some(1))),
            Some((None, None))
        );
    }

    #[tokio::test]
    async fn prio_endpoints_reorder_queue_and_report_priority() {
        let (state, session, output) = qbit_state().await;
        let mut hashes = Vec::new();
        for byte in [0x41u8, 0x42, 0x43] {
            let dir = output.path().join(format!("src{byte}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("payload.bin"), vec![byte; 32 * 1024]).unwrap();
            let torrent = create_torrent(
                &dir,
                CreateTorrentOptions {
                    piece_length: Some(16_384),
                    ..Default::default()
                },
                &BlockingSpawner::new(1),
            )
            .await
            .unwrap()
            .as_bytes()
            .unwrap()
            .to_vec();
            let handle = session
                .add_torrent(
                    AddTorrent::from_bytes(torrent),
                    Some(AddTorrentOptions {
                        paused: true,
                        overwrite: true,
                        output_folder: Some(
                            output.path().join("dl").to_string_lossy().into_owned(),
                        ),
                        ..Default::default()
                    }),
                )
                .await
                .unwrap()
                .into_handle()
                .unwrap();
            handle.wait_until_initialized().await.unwrap();
            hashes.push(handle.info_hash().as_string());
        }
        let router = super::make_qbit_router(state.api_state.clone());
        let post = |action: &str, body: String| {
            http::Request::builder()
                .method("POST")
                .uri(format!("/torrents/{action}"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(axum::body::Body::from(body))
                .unwrap()
        };
        let priorities = || {
            let router = router.clone();
            let hashes = hashes.clone();
            async move {
                let request = http::Request::builder()
                    .uri("/torrents/info")
                    .body(axum::body::Body::empty())
                    .unwrap();
                let response = router.oneshot(request).await.unwrap();
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let list: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
                hashes
                    .iter()
                    .map(|h| {
                        list.iter().find(|t| t["hash"] == *h).unwrap()["priority"]
                            .as_u64()
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            }
        };

        // Queueing off: every priority is 0.
        assert_eq!(priorities().await, vec![0, 0, 0]);
        session.set_queue_limits(Some(1), None).await.unwrap();
        assert_eq!(priorities().await, vec![1, 2, 3]);

        for (action, hash, expected) in [
            ("topPrio", 2, vec![2, 3, 1]),
            ("increasePrio", 1, vec![3, 2, 1]),
            ("bottomPrio", 2, vec![2, 1, 3]),
            ("decreasePrio", 0, vec![3, 1, 2]),
        ] {
            let response = router
                .clone()
                .oneshot(post(action, format!("hashes={}", hashes[hash])))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{action}");
            assert_eq!(priorities().await, expected, "{action}");
        }

        // setForceStart starts the torrent outside the queue.
        let response = router
            .clone()
            .oneshot(post(
                "setForceStart",
                format!("hashes={}&value=true", hashes[0]),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let handle = state
            .api_state
            .api
            .mgr_handle(crate::api::TorrentIdOrHash::parse(&hashes[0]).unwrap())
            .unwrap();
        assert!(handle.is_force_started());
        assert!(!handle.is_paused());
    }

    #[tokio::test]
    async fn queue_and_force_start_routes_succeed() {
        let (state, _session, _output) = qbit_state().await;
        let router = super::make_qbit_router(state.api_state.clone());
        for action in [
            "topPrio",
            "bottomPrio",
            "increasePrio",
            "decreasePrio",
            "setForceStart",
        ] {
            let request = http::Request::builder()
                .method("POST")
                .uri(format!("/torrents/{action}"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(axum::body::Body::from("hashes=all&value=true"))
                .unwrap();
            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{action}");
        }
    }

    #[tokio::test]
    async fn recheck_endpoint_accepts_qbit_hash_form() {
        let (state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            output.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();

        let body = Bytes::from(format!("hashes={}", handle.info_hash().as_string()));
        assert_eq!(
            h_torrents_recheck(axum::extract::State(state), body).await,
            "Ok."
        );
        assert!(matches!(
            handle.stats().state,
            TorrentStatsState::Initializing
        ));
        handle.wait_until_initialized().await.unwrap();
        assert!(matches!(handle.stats().state, TorrentStatsState::Paused));
    }

    #[tokio::test]
    async fn per_torrent_rate_limits_round_trip_on_handle() {
        use std::num::NonZeroU32;

        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            output.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();

        // Defaults to unlimited.
        assert_eq!(handle.rate_limits().download_bps, None);
        assert_eq!(handle.rate_limits().upload_bps, None);

        // Each direction is set independently and persists on the override
        // (this torrent is paused, so there is no live limiter to update).
        handle.set_download_limit(NonZeroU32::new(4096));
        handle.set_upload_limit(NonZeroU32::new(8192));
        assert_eq!(handle.rate_limits().download_bps, NonZeroU32::new(4096));
        assert_eq!(handle.rate_limits().upload_bps, NonZeroU32::new(8192));

        // Setting one leaves the other untouched.
        handle.set_download_limit(NonZeroU32::new(1024));
        assert_eq!(handle.rate_limits().download_bps, NonZeroU32::new(1024));
        assert_eq!(handle.rate_limits().upload_bps, NonZeroU32::new(8192));

        // None clears the limit.
        handle.set_download_limit(None);
        assert_eq!(handle.rate_limits().download_bps, None);
    }

    #[tokio::test]
    async fn rename_file_moves_on_disk_and_updates_metadata_when_paused() {
        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            output.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();

        // Discover file 0's real on-disk location from the metadata.
        let old_rel = handle
            .with_metadata(|m| m.file_infos[0].relative_filename.clone())
            .unwrap();
        let old_abs = output.path().join(&old_rel);
        assert!(old_abs.exists(), "expected file at {old_abs:?}");

        // A path escaping the root is rejected before anything moves.
        assert!(
            handle
                .rename_files(&[(0, std::path::PathBuf::from("../escape.bin"))])
                .is_err()
        );
        assert!(old_abs.exists(), "rejected rename must not move anything");

        // A valid rename moves the file on disk and updates file_infos.
        let new_rel = std::path::PathBuf::from("renamed_dir/renamed.bin");
        handle.rename_files(&[(0, new_rel.clone())]).unwrap();
        assert_eq!(
            handle
                .with_metadata(|m| m.file_infos[0].relative_filename.clone())
                .unwrap(),
            new_rel
        );
        assert!(!old_abs.exists(), "old path should be gone");
        assert!(
            output.path().join(&new_rel).exists(),
            "new path should exist"
        );

        // The display-name override is independent of file renames.
        assert!(handle.name().is_some());
        handle.set_display_name(Some("Custom Name".to_string()));
        assert_eq!(handle.name().as_deref(), Some("Custom Name"));
        handle.set_display_name(Some("   ".to_string()));
        assert_ne!(
            handle.name().as_deref(),
            Some("   "),
            "blank name clears override"
        );
    }

    #[tokio::test]
    async fn set_location_relocates_torrent_files_when_paused() {
        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            output.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();

        let rel = handle
            .with_metadata(|m| m.file_infos[0].relative_filename.clone())
            .unwrap();
        let old_abs = output.path().join(&rel);
        assert!(old_abs.exists(), "expected file at {old_abs:?}");

        // Relocate to a sibling directory on the same filesystem.
        let new_root = output.path().join("relocated");
        handle.set_location(new_root.clone()).unwrap();

        assert!(!old_abs.exists(), "old location should be gone");
        assert!(
            new_root.join(&rel).exists(),
            "file should now live under the new root"
        );
    }

    async fn add_single_file_torrent(
        session: &Arc<crate::Session>,
        root: &std::path::Path,
        paused: bool,
        file_renames: Vec<(usize, std::path::PathBuf)>,
    ) -> crate::torrent_state::ManagedTorrentHandle {
        let torrent = create_torrent(
            root,
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused,
                    overwrite: true,
                    output_folder: Some(root.to_string_lossy().into_owned()),
                    file_renames,
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();
        handle
    }

    #[tokio::test]
    async fn session_set_location_pauses_live_torrent_and_resumes_it() {
        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let handle = add_single_file_torrent(&session, output.path(), false, Vec::new()).await;
        assert!(handle.live().is_some(), "torrent should be live");
        let rel = handle
            .with_metadata(|m| m.file_infos[0].relative_filename.clone())
            .unwrap();

        // The handle-level move still refuses a live torrent...
        let new_root = output.path().join("moved");
        assert!(handle.set_location(new_root.clone()).is_err());

        // ...while the session pauses it for the move and resumes it.
        session
            .set_location(&handle, new_root.clone())
            .await
            .unwrap();
        assert!(
            new_root.join(&rel).exists(),
            "file should be under new root"
        );
        assert!(
            !output.path().join(&rel).exists(),
            "old file should be gone"
        );
        assert_eq!(handle.output_folder(), new_root);
        assert!(
            !handle.is_paused(),
            "torrent should be resumed after the move"
        );
    }

    #[tokio::test]
    async fn session_rename_files_works_on_live_torrent_and_reports_renames() {
        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let handle = add_single_file_torrent(&session, output.path(), false, Vec::new()).await;
        assert!(handle.renamed_files().is_empty());

        let new_rel = std::path::PathBuf::from("sub/renamed.bin");
        session
            .rename_files(&handle, &[(0, new_rel.clone())])
            .await
            .unwrap();
        assert!(output.path().join(&new_rel).exists());
        assert_eq!(handle.renamed_files(), vec![(0, new_rel)]);
        assert!(
            !handle.is_paused(),
            "torrent should be resumed after the rename"
        );
    }

    #[tokio::test]
    async fn add_torrent_applies_restored_file_renames() {
        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            output.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        // Lay the file out the way an earlier rename would have left it.
        let dest = output.path().join("dest");
        let rel = std::path::PathBuf::from("restored/payload.bin");
        std::fs::create_dir_all(dest.join("restored")).unwrap();
        std::fs::rename(output.path().join("payload.bin"), dest.join(&rel)).unwrap();

        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(dest.to_string_lossy().into_owned()),
                    file_renames: vec![(0, rel.clone())],
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();
        assert_eq!(
            handle
                .with_metadata(|m| m.file_infos[0].relative_filename.clone())
                .unwrap(),
            rel
        );
        assert_eq!(handle.renamed_files(), vec![(0, rel.clone())]);
        // The restored file is recognised as complete data at the renamed path.
        assert!(dest.join(&rel).exists());
        assert!(!dest.join("payload.bin").exists());
    }

    #[tokio::test]
    async fn add_torrent_ignores_invalid_restored_file_renames() {
        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let handle = add_single_file_torrent(
            &session,
            output.path(),
            true,
            vec![(0, std::path::PathBuf::from("../escape.bin"))],
        )
        .await;
        assert!(handle.renamed_files().is_empty());
    }

    #[test]
    fn category_filter_supports_qbit_special_values() {
        assert!(matches_category("all", "Linux ISOs"));
        assert!(matches_category("all", ""));
        assert!(matches_category("uncategorized", ""));
        assert!(matches_category("", ""));
        assert!(!matches_category("uncategorized", "Linux ISOs"));
        assert!(matches_category("Linux ISOs", "Linux ISOs"));
        assert!(!matches_category("linux isos", "Linux ISOs"));
    }

    #[test]
    fn qbit_multi_file_names_include_torrent_root_for_save_path_imports() {
        assert_eq!(
            qbit_file_name(Some("release"), "disc/file.bin", true, true),
            Path::new("release").join("disc/file.bin").to_string_lossy()
        );
        assert_eq!(
            qbit_file_name(Some("release"), "single.bin", false, true),
            "single.bin"
        );
        assert_eq!(
            qbit_file_name(Some("release"), "only.bin", true, true),
            Path::new("release").join("only.bin").to_string_lossy()
        );
        assert_eq!(
            qbit_file_name(Some("release"), "disc/file.bin", true, false),
            "disc/file.bin"
        );
    }

    #[test]
    fn test_eta_overflow_safety() {
        // Simulate: very large remaining bytes / very small download speed
        // This should not panic, and should clamp to a safe fallback.
        let remaining: u64 = u64::MAX;
        let dl_speed: u64 = 1;
        let eta_secs = remaining / dl_speed;
        let eta = i64::try_from(eta_secs).unwrap_or(8640000i64);
        assert_eq!(eta, 8640000i64, "should clamp to fallback on overflow");

        // Large but within i64 range
        let remaining: u64 = 1_000_000_000_000;
        let dl_speed: u64 = 100;
        let eta_secs = remaining / dl_speed;
        let eta = i64::try_from(eta_secs).unwrap_or(8640000i64);
        assert_eq!(
            eta, 10_000_000_000i64,
            "should return exact ETA when it fits in i64"
        );

        // Zero speed: the calling code uses a guard, but verify the fallback path
        let dl_speed: u64 = 0;
        let eta = 1_000_000u64
            .checked_div(dl_speed)
            .map(|seconds| i64::try_from(seconds).unwrap_or(8_640_000))
            .unwrap_or(8_640_000);
        assert_eq!(eta, 8640000i64, "zero speed should return 8640000");
    }

    #[test]
    fn test_timestamp_cast_safety() {
        // Current unix timestamp fits in i64, but test the boundary
        let now: u64 = i64::MAX as u64 + 1;
        let result = i64::try_from(now).unwrap_or(i64::MAX);
        assert_eq!(result, i64::MAX, "should clamp to i64::MAX on overflow");

        // Normal timestamp should pass through
        let now: u64 = 1_700_000_000;
        let result = i64::try_from(now).unwrap_or(i64::MAX);
        assert_eq!(result, 1_700_000_000i64);
    }

    #[test]
    fn parse_tags_trims_splits_and_drops_empties() {
        assert_eq!(
            super::parse_tags("tv, , radarr ,,sonarr"),
            vec!["tv".to_string(), "radarr".to_string(), "sonarr".to_string()]
        );
        assert!(super::parse_tags("").is_empty());
        assert!(super::parse_tags("  , ,").is_empty());
    }

    /// Add a paused torrent over a single file of `byte` repeated, so each
    /// call with a different byte yields a distinct info hash.
    async fn add_paused_torrent(
        session: &Arc<Session>,
        output: &tempfile::TempDir,
        byte: u8,
    ) -> crate::torrent_state::ManagedTorrentHandle {
        let dir = output.path().join(format!("src{byte}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("payload.bin"), vec![byte; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            &dir,
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();
        handle
    }

    fn torrent_tags(handle: &crate::torrent_state::ManagedTorrentHandle) -> Vec<String> {
        handle.shared().tags.read().iter().cloned().collect()
    }

    #[tokio::test]
    async fn tag_endpoints_drive_backend_tags() {
        let (state, session, output) = qbit_state().await;
        let a = add_paused_torrent(&session, &output, 0x11).await;
        let b = add_paused_torrent(&session, &output, 0x22).await;
        let (ha, hb) = (a.info_hash().as_string(), b.info_hash().as_string());
        let st = || axum::extract::State(state.clone());
        let form = |s: String| Bytes::from(s);

        // createTags registers tags without touching any torrent.
        h_torrents_create_tags(st(), form("tags=tv, hd".into())).await;
        assert_eq!(session.tags().list(), vec!["hd", "tv"]);
        assert!(torrent_tags(&a).is_empty());

        // addTags assigns tags (registering new ones) to every listed torrent.
        h_torrents_add_tags(st(), form(format!("hashes={ha}|{hb}&tags=tv,new"))).await;
        assert_eq!(torrent_tags(&a), vec!["new", "tv"]);
        assert_eq!(torrent_tags(&b), vec!["new", "tv"]);
        assert!(session.tags().exists("new"));

        // removeTags with a specific tag only drops it from the given torrent.
        h_torrents_remove_tags(st(), form(format!("hashes={ha}&tags=tv"))).await;
        assert_eq!(torrent_tags(&a), vec!["new"]);
        assert_eq!(torrent_tags(&b), vec!["new", "tv"]);

        // setTags replaces the whole tag set.
        h_torrents_set_tags(st(), form(format!("hashes={hb}&tags=only"))).await;
        assert_eq!(torrent_tags(&b), vec!["only"]);

        // deleteTags unregisters a tag and strips it from every torrent.
        h_torrents_delete_tags(st(), form("tags=new".into())).await;
        assert!(torrent_tags(&a).is_empty());
        assert!(!session.tags().exists("new"));

        // removeTags with an empty list clears every tag on the torrent.
        h_torrents_add_tags(st(), form(format!("hashes={ha}&tags=x,y"))).await;
        h_torrents_remove_tags(st(), form(format!("hashes={ha}&tags="))).await;
        assert!(torrent_tags(&a).is_empty());

        // torrents/info exposes the tags and filters by them; an empty tag
        // selects untagged torrents.
        let info = |tag: &str| {
            let state = state.clone();
            let tag = tag.to_owned();
            async move {
                let query = super::TorrentsInfoQuery {
                    tag: Some(tag),
                    ..Default::default()
                };
                let response =
                    h_torrents_info(axum::extract::State(state), axum::extract::Query(query))
                        .await
                        .into_response();
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                serde_json::from_slice::<Vec<serde_json::Value>>(&body).unwrap()
            }
        };
        let only = info("only").await;
        assert_eq!(only.len(), 1);
        assert_eq!(only[0]["hash"], hb);
        assert_eq!(only[0]["tags"], "only");
        let untagged = info("").await;
        assert_eq!(untagged.len(), 1);
        assert_eq!(untagged[0]["hash"], ha);
        assert!(info("missing").await.is_empty());
    }

    #[tokio::test]
    async fn legacy_tag_store_is_imported_once() {
        let (state, session, output) = qbit_state().await;
        let a = add_paused_torrent(&session, &output, 0x33).await;
        let path = output.path().join("qbit-tags.json");
        let legacy = serde_json::json!({
            "all": ["movies"],
            "per_torrent": {
                a.info_hash().as_string(): ["hd"],
                "f".repeat(40): ["gone"],
            },
        });
        std::fs::write(&path, legacy.to_string()).unwrap();

        super::migrate_legacy_tag_store(&state.api_state.api, &path).await;

        assert_eq!(torrent_tags(&a), vec!["hd"]);
        assert_eq!(session.tags().list(), vec!["hd", "movies"]);
        assert!(!path.exists());
        assert!(path.with_extension("json.migrated").exists());
    }

    #[tokio::test]
    async fn unreadable_legacy_tag_store_is_left_in_place() {
        let (state, session, output) = qbit_state().await;
        let path = output.path().join("qbit-tags.json");
        std::fs::write(&path, b"{ not json").unwrap();
        super::migrate_legacy_tag_store(&state.api_state.api, &path).await;
        assert!(path.exists());
        assert!(session.tags().list().is_empty());
    }

    #[test]
    fn matches_tag_treats_empty_as_untagged() {
        let tags: std::collections::BTreeSet<String> = ["tv".to_owned()].into();
        assert!(super::matches_tag("tv", &tags));
        assert!(!super::matches_tag("hd", &tags));
        assert!(!super::matches_tag("", &tags));
        assert!(super::matches_tag("", &Default::default()));
    }

    #[test]
    fn offset_limit_pagination_matches_qbittorrent_semantics() {
        use super::apply_offset_limit;
        let v = || vec![0, 1, 2, 3, 4];
        // Normal window.
        assert_eq!(apply_offset_limit(v(), 1, Some(2)), vec![1, 2]);
        // Offset past the end -> empty (not the whole list).
        assert_eq!(apply_offset_limit(v(), 5, None), Vec::<i32>::new());
        assert_eq!(apply_offset_limit(v(), 99, Some(3)), Vec::<i32>::new());
        // Offset exactly at the end -> empty.
        assert_eq!(apply_offset_limit(v(), 5, None), Vec::<i32>::new());
        // limit == 0 means no limit.
        assert_eq!(apply_offset_limit(v(), 0, Some(0)), vec![0, 1, 2, 3, 4]);
        // No offset, no limit.
        assert_eq!(apply_offset_limit(v(), 0, None), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn share_limit_wire_encoding_round_trips() {
        use super::{minutes_from_qbit, minutes_to_qbit, ratio_from_qbit, ratio_to_qbit};
        use std::time::Duration;
        assert_eq!(ratio_from_qbit(-1.0), None);
        assert_eq!(ratio_from_qbit(-2.0), None);
        assert_eq!(ratio_from_qbit(0.0), Some(0.0));
        assert_eq!(ratio_to_qbit(ratio_from_qbit(1.5)), 1.5);
        assert_eq!(ratio_to_qbit(None), -1.0);
        assert_eq!(minutes_from_qbit(-1), None);
        assert_eq!(minutes_from_qbit(-2), None);
        assert_eq!(minutes_from_qbit(90), Some(Duration::from_secs(5400)));
        assert_eq!(minutes_to_qbit(minutes_from_qbit(90)), 90);
        assert_eq!(minutes_to_qbit(None), -1);
    }

    #[test]
    fn torrent_creator_task_store_lifecycle() {
        let creator = QbitTorrentCreator::default();
        let id = creator.new_task("/data/movie".to_string(), "v1".to_string());
        let task = creator.get(&id).unwrap();
        assert!(matches!(task.status, CreatorStatus::Running));
        assert_eq!(task.source_path, "/data/movie");
        assert_eq!(creator.all().len(), 1);

        creator.finish(
            &id,
            Some(bytes::Bytes::from_static(b"torrent")),
            "abc".to_string(),
        );
        let task = creator.get(&id).unwrap();
        assert!(matches!(task.status, CreatorStatus::Finished));
        assert_eq!(task.info_hash.as_deref(), Some("abc"));
        assert_eq!(task.torrent_bytes.as_deref(), Some(&b"torrent"[..]));

        let failing = creator.new_task("/data/x".to_string(), "v2".to_string());
        creator.fail(&failing, "boom".to_string());
        let task = creator.get(&failing).unwrap();
        assert!(matches!(task.status, CreatorStatus::Failed));
        assert_eq!(task.error.as_deref(), Some("boom"));

        assert!(creator.remove(&id));
        assert!(!creator.remove(&id));
        assert!(creator.get(&id).is_none());
        assert_eq!(creator.all().len(), 1);
    }

    #[tokio::test]
    async fn torrentcreator_add_task_produces_a_torrent_file() {
        let (state, _session, output) = qbit_state().await;
        std::fs::write(output.path().join("data.bin"), vec![7u8; 4096]).unwrap();

        let body = Bytes::from(format!("sourcePath={}", output.path().to_string_lossy()));
        let response = h_torrentcreator_add_task(axum::extract::State(state.clone()), body)
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);

        // The task is registered synchronously; poll it to completion.
        let all = state.creator.all();
        assert_eq!(all.len(), 1);
        let (id, _) = all.into_iter().next().unwrap();
        for _ in 0..200 {
            if !matches!(
                state.creator.get(&id).unwrap().status,
                CreatorStatus::Running
            ) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let task = state.creator.get(&id).unwrap();
        assert!(
            matches!(task.status, CreatorStatus::Finished),
            "creation did not finish: {:?}",
            task.error
        );
        assert!(
            task.torrent_bytes.is_some(),
            "expected generated torrent bytes"
        );
        assert!(task.info_hash.is_some());
    }

    #[test]
    fn tracker_status_maps_to_qbit_codes() {
        use super::qbit_tracker_status;
        use tracker_comms::TrackerAnnounceState::*;
        assert_eq!(qbit_tracker_status(Disabled), 0);
        assert_eq!(qbit_tracker_status(NotContacted), 1);
        assert_eq!(qbit_tracker_status(Working), 2);
        assert_eq!(qbit_tracker_status(Updating), 3);
        assert_eq!(qbit_tracker_status(Error), 4);
    }

    #[test]
    fn rate_limit_conversions_round_trip_and_treat_zero_as_unlimited() {
        use std::num::NonZeroU32;
        assert_eq!(super::limit_to_bps(0), None);
        assert_eq!(super::bps_to_u64(None), 0);
        assert_eq!(super::limit_to_bps(1024), NonZeroU32::new(1024));
        assert_eq!(super::bps_to_u64(NonZeroU32::new(1024)), 1024);
        // Values beyond u32 saturate rather than overflow.
        assert_eq!(super::limit_to_bps(u64::MAX), NonZeroU32::new(u32::MAX));
    }

    #[test]
    fn per_torrent_limit_treats_non_positive_as_unlimited() {
        use std::num::NonZeroU32;
        assert_eq!(super::parse_torrent_limit(0), None);
        assert_eq!(super::parse_torrent_limit(-1), None);
        assert_eq!(super::parse_torrent_limit(-999), None);
        assert_eq!(super::parse_torrent_limit(2048), NonZeroU32::new(2048));
        assert_eq!(
            super::parse_torrent_limit(i64::MAX),
            NonZeroU32::new(u32::MAX)
        );
    }

    #[tokio::test]
    async fn clientdata_survives_reopen_and_null_removes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("qbit-clientdata.json");
        let store = super::ClientDataStore::open(Some(path.clone()));
        let changes = serde_json::json!({"a": 1, "b": "x"});
        store
            .store(changes.as_object().unwrap().clone())
            .await
            .unwrap();
        let removal = serde_json::json!({"a": null});
        store
            .store(removal.as_object().unwrap().clone())
            .await
            .unwrap();

        let reopened = super::ClientDataStore::open(Some(path.clone()));
        assert_eq!(
            serde_json::Value::Object(reopened.load(&[]).await),
            serde_json::json!({"b": "x"})
        );

        // A corrupt file starts empty instead of failing the router.
        std::fs::write(&path, b"not json").unwrap();
        assert!(
            super::ClientDataStore::open(Some(path))
                .load(&[])
                .await
                .is_empty()
        );
    }
}

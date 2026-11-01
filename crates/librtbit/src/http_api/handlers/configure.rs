use std::{path::PathBuf, time::Duration};

use axum::{Json, extract::Query, extract::State, response::IntoResponse};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use super::ApiState;
use crate::{
    ApiError, WithStatusError,
    api::{EmptyJsonResponse, Result},
    limits::LimitsConfig,
    session_persistence::AdvancedSettings,
};

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/limits",
    request_body(content = LimitsConfig, description = "Rate limits configuration"),
    responses(
        (status = 200, description = "Rate limits updated", body = EmptyJsonResponse)
    )
))]
pub async fn h_update_session_ratelimits(
    State(state): State<ApiState>,
    Json(limits): Json<LimitsConfig>,
) -> axum::response::Response {
    // Validate before applying anything, so a refused body changes nothing.
    // A zero would stall the session; a huge value would exhaust descriptors.
    if let Err(err) = bounded(
        "peer_limit",
        limits.peer_limit.map(|v| v as u64),
        MIN_PEER_LIMIT,
        MAX_PEER_LIMIT,
    )
    .and_then(|_| {
        bounded(
            "concurrent_init_limit",
            limits.concurrent_init_limit.map(|v| v as u64),
            MIN_CONCURRENT_INIT_LIMIT,
            MAX_CONCURRENT_INIT_LIMIT,
        )
    }) {
        return err.into_response();
    }
    // Goes through the alt-speed-aware setter: while alternative limits are
    // active this updates the saved normal limits instead of the live limiter.
    state
        .api
        .session()
        .set_normal_rate_limits(limits.download_bps, limits.upload_bps);
    if let Some(peer_limit) = limits.peer_limit {
        state.api.session().set_peer_limit(peer_limit);
    }
    if let Some(init_limit) = limits.concurrent_init_limit {
        state.api.session().set_concurrent_init_limit(init_limit);
    }
    Json(EmptyJsonResponse {}).into_response()
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/limits",
    responses(
        (status = 200, description = "Current rate limits", body = LimitsConfig)
    )
))]
pub async fn h_get_session_ratelimits(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let mut config = state.api.session().ratelimits.get_config();
    config.peer_limit = Some(state.api.session().get_peer_limit());
    config.concurrent_init_limit = Some(state.api.session().get_concurrent_init_limit());
    Ok(Json(config))
}

// --- Folder management ---

#[derive(Serialize, Deserialize)]
pub struct FoldersConfig {
    pub download_folder: String,
    pub completed_folder: Option<String>,
}

pub async fn h_get_folders(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(FoldersConfig {
        download_folder: state.api.api_output_folder(),
        completed_folder: state.api.api_completed_folder(),
    }))
}

pub async fn h_set_folders(
    State(state): State<ApiState>,
    Json(config): Json<FoldersConfig>,
) -> axum::response::Response {
    match state
        .api
        .api_set_folders(config.download_folder, config.completed_folder)
        .await
    {
        Ok(()) => Json(EmptyJsonResponse {}).into_response(),
        Err(err) => err.into_response(),
    }
}

// --- Downloads settings ---

/// Session-wide download behaviour shown on the settings Downloads tab.
#[derive(Serialize, Deserialize)]
pub struct DownloadsConfig {
    /// Write incomplete files under a `.part` name; applies to torrents added afterwards.
    pub incomplete_file_suffix: bool,
}

pub async fn h_get_downloads(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let session = state.api.session();
    Ok(Json(DownloadsConfig {
        incomplete_file_suffix: session.use_part_files(),
    }))
}

pub async fn h_set_downloads(
    State(state): State<ApiState>,
    Json(config): Json<DownloadsConfig>,
) -> axum::response::Response {
    let session = state.api.session();
    session.set_use_part_files(config.incomplete_file_suffix);
    if let Err(err) = session.persist_session_settings().await {
        return ApiError::from(err).into_response();
    }
    Json(EmptyJsonResponse {}).into_response()
}

// --- Advanced library knobs ---

/// Bounds for the Advanced tab. A value outside its bounds is refused with a
/// 400 that names the field. The caps are defined beside the settings store so
/// a hand-edited settings file is held to the same limits.
pub const MIN_CONNECT_TIMEOUT_SECS: u64 = crate::session_persistence::MIN_CONNECT_TIMEOUT_SECS;
pub const MAX_CONNECT_TIMEOUT_SECS: u64 = crate::session_persistence::MAX_CONNECT_TIMEOUT_SECS;
pub const MIN_READ_WRITE_TIMEOUT_SECS: u64 =
    crate::session_persistence::MIN_READ_WRITE_TIMEOUT_SECS;
pub const MAX_READ_WRITE_TIMEOUT_SECS: u64 =
    crate::session_persistence::MAX_READ_WRITE_TIMEOUT_SECS;
pub const MIN_KEEP_ALIVE_SECS: u64 = crate::session_persistence::MIN_KEEP_ALIVE_SECS;
pub const MAX_KEEP_ALIVE_SECS: u64 = crate::session_persistence::MAX_KEEP_ALIVE_SECS;
pub const MIN_UPLOAD_SLOTS: u64 = crate::session_persistence::MIN_UPLOAD_SLOTS;
pub const MAX_UPLOAD_SLOTS: u64 = crate::session_persistence::MAX_UPLOAD_SLOTS;
pub const MIN_MAGNET_RESOLUTION_TIMEOUT_SECS: u64 =
    crate::session_persistence::MIN_MAGNET_RESOLUTION_TIMEOUT_SECS;
pub const MAX_MAGNET_RESOLUTION_TIMEOUT_SECS: u64 =
    crate::session_persistence::MAX_MAGNET_RESOLUTION_TIMEOUT_SECS;
pub const MIN_MAX_CONCURRENT_HANDSHAKES: u64 =
    crate::session_persistence::MIN_MAX_CONCURRENT_HANDSHAKES;
pub const MAX_MAX_CONCURRENT_HANDSHAKES: u64 =
    crate::session_persistence::MAX_MAX_CONCURRENT_HANDSHAKES;
pub const MIN_FASTRESUME_VALIDATION_DENOM: u64 =
    crate::session_persistence::MIN_FASTRESUME_VALIDATION_DENOM;
pub const MAX_FASTRESUME_VALIDATION_DENOM: u64 =
    crate::session_persistence::MAX_FASTRESUME_VALIDATION_DENOM;
/// `POST /torrents/limits` bounds.
pub const MIN_PEER_LIMIT: u64 = 1;
pub const MAX_PEER_LIMIT: u64 = 5000;
pub const MIN_CONCURRENT_INIT_LIMIT: u64 = 1;
pub const MAX_CONCURRENT_INIT_LIMIT: u64 = 32;

/// Session-wide Advanced knobs. `None` (or a missing field) means the default,
/// so a body that only carries the three original timeouts is still accepted.
///
/// Live knobs apply at once, to what starts next, and are saved when session
/// persistence is on: `connect_timeout_secs`, `read_write_timeout_secs`,
/// `keep_alive_secs` (seconds), `upload_slots` (per torrent started
/// afterwards) and `magnet_resolution_timeout_secs` (session default; a
/// per-add value still wins). Restart knobs are saved for the next start and
/// need persistence: `max_concurrent_handshakes` and
/// `fastresume_validation_denom` (0 skips probabilistic validation).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdvancedConfig {
    #[serde(default)]
    pub connect_timeout_secs: Option<u64>,
    #[serde(default)]
    pub read_write_timeout_secs: Option<u64>,
    #[serde(default)]
    pub keep_alive_secs: Option<u64>,
    #[serde(default)]
    pub upload_slots: Option<u64>,
    #[serde(default)]
    pub magnet_resolution_timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_concurrent_handshakes: Option<u64>,
    #[serde(default)]
    pub fastresume_validation_denom: Option<u64>,
}

/// The restart knobs as saved for the next start.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdvancedRestartKnobs {
    pub max_concurrent_handshakes: Option<u64>,
    pub fastresume_validation_denom: Option<u64>,
}

/// `GET /session/advanced`: the knobs plus whether a restart is pending.
/// The three timeouts are the values in effect (including startup flags); the
/// other knobs are the saved values, null when at their default.
#[derive(Serialize)]
pub struct AdvancedStatus {
    #[serde(flatten)]
    pub config: AdvancedConfig,
    /// True when a saved restart knob differs from the one this process runs.
    pub restart_required: bool,
    /// The saved restart knobs, present only while `restart_required`.
    pub pending_restart: Option<AdvancedRestartKnobs>,
    /// False when session persistence is off, so restart knobs cannot be saved.
    pub can_save: bool,
}

fn secs_of(d: Option<Duration>) -> Option<u64> {
    d.map(|d| d.as_secs())
}

/// Check `value` against `min..=max`. `None` is the default and always passes.
/// The message names the field.
fn bounded(name: &str, value: Option<u64>, min: u64, max: u64) -> Result<Option<u64>> {
    match value {
        Some(v) if !(min..=max).contains(&v) => Err(ApiError::from((
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("{name} must be between {min} and {max}"),
        ))),
        other => Ok(other),
    }
}

/// Check every Advanced knob. Returns what to apply, or the first refusal.
fn validate_advanced(config: &AdvancedConfig) -> Result<AdvancedSettings> {
    Ok(AdvancedSettings {
        connect_timeout_secs: bounded(
            "connect_timeout_secs",
            config.connect_timeout_secs,
            MIN_CONNECT_TIMEOUT_SECS,
            MAX_CONNECT_TIMEOUT_SECS,
        )?,
        read_write_timeout_secs: bounded(
            "read_write_timeout_secs",
            config.read_write_timeout_secs,
            MIN_READ_WRITE_TIMEOUT_SECS,
            MAX_READ_WRITE_TIMEOUT_SECS,
        )?,
        keep_alive_secs: bounded(
            "keep_alive_secs",
            config.keep_alive_secs,
            MIN_KEEP_ALIVE_SECS,
            MAX_KEEP_ALIVE_SECS,
        )?,
        upload_slots: bounded(
            "upload_slots",
            config.upload_slots,
            MIN_UPLOAD_SLOTS,
            MAX_UPLOAD_SLOTS,
        )?,
        magnet_resolution_timeout_secs: bounded(
            "magnet_resolution_timeout_secs",
            config.magnet_resolution_timeout_secs,
            MIN_MAGNET_RESOLUTION_TIMEOUT_SECS,
            MAX_MAGNET_RESOLUTION_TIMEOUT_SECS,
        )?,
        max_concurrent_handshakes: bounded(
            "max_concurrent_handshakes",
            config.max_concurrent_handshakes,
            MIN_MAX_CONCURRENT_HANDSHAKES,
            MAX_MAX_CONCURRENT_HANDSHAKES,
        )?,
        fastresume_validation_denom: bounded(
            "fastresume_validation_denom",
            config.fastresume_validation_denom,
            MIN_FASTRESUME_VALIDATION_DENOM,
            MAX_FASTRESUME_VALIDATION_DENOM,
        )?,
    })
}

fn restart_knobs(settings: &AdvancedSettings) -> AdvancedRestartKnobs {
    AdvancedRestartKnobs {
        max_concurrent_handshakes: settings.max_concurrent_handshakes,
        fastresume_validation_denom: settings.fastresume_validation_denom,
    }
}

pub async fn h_get_advanced(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let session = state.api.session();
    let opts = session.peer_opts();
    let saved = session.saved_advanced();
    let saved_restart = restart_knobs(&saved);
    let restart_required = saved_restart != restart_knobs(&session.startup_advanced());
    Ok(Json(AdvancedStatus {
        config: AdvancedConfig {
            connect_timeout_secs: secs_of(opts.connect_timeout),
            read_write_timeout_secs: secs_of(opts.read_write_timeout),
            keep_alive_secs: secs_of(opts.keep_alive_interval),
            upload_slots: saved.upload_slots,
            magnet_resolution_timeout_secs: saved.magnet_resolution_timeout_secs,
            max_concurrent_handshakes: saved.max_concurrent_handshakes,
            fastresume_validation_denom: saved.fastresume_validation_denom,
        },
        restart_required,
        pending_restart: restart_required.then_some(saved_restart),
        can_save: session.has_persistence(),
    }))
}

/// Replace the Advanced knobs. Every value is checked first; a refused body
/// changes nothing. Live knobs apply at once and are saved when persistence is
/// on. A restart knob needs persistence: without it the live knobs still apply
/// and the answer is 409.
pub async fn h_set_advanced(
    State(state): State<ApiState>,
    Json(config): Json<AdvancedConfig>,
) -> axum::response::Response {
    let mut settings = match validate_advanced(&config) {
        Ok(settings) => settings,
        Err(err) => return err.into_response(),
    };
    let session = state.api.session();
    let persistent = session.has_persistence();
    let has_restart_knob = restart_knobs(&settings)
        != AdvancedRestartKnobs {
            max_concurrent_handshakes: None,
            fastresume_validation_denom: None,
        };
    if !persistent {
        // Nothing can be saved, so a restart knob must not linger in memory.
        settings.max_concurrent_handshakes = None;
        settings.fastresume_validation_denom = None;
    }
    session.set_advanced(settings);
    if !persistent && has_restart_knob {
        return ApiError::from((
            StatusCode::CONFLICT,
            "session persistence is off, so max_concurrent_handshakes and \
             fastresume_validation_denom cannot be saved for the next start; \
             the other settings were applied",
        ))
        .into_response();
    }
    if let Err(err) = session.persist_session_settings().await {
        tracing::warn!("could not save advanced settings: {err:#}");
        return ApiError::from(err).into_response();
    }
    let restart_required = persistent
        && restart_knobs(&session.saved_advanced()) != restart_knobs(&session.startup_advanced());
    Json(serde_json::json!({ "restart_required": restart_required })).into_response()
}

// --- Network settings (startup only) ---

/// Peer and discovery settings as the running process actually has them.
/// `encryption` is always null: this engine has no encryption-mode switch.
/// `pex_enabled` is always true: peer exchange is part of the peer protocol and
/// has no flag. `http_api_listen` is the address this request arrived on, for
/// display only. `pending` holds values saved for the next start.
#[derive(Serialize)]
pub struct NetworkConfig {
    pub listen_ip: Option<String>,
    pub listen_port: Option<u16>,
    pub dht_enabled: bool,
    pub pex_enabled: bool,
    pub lsd_enabled: bool,
    pub upnp_port_forward: bool,
    pub encryption: Option<String>,
    pub http_api_listen: String,
    pub pending: Option<crate::session_persistence::NetworkSettings>,
    /// What this start did with the saved values: `applied`, `ignored`
    /// (`--ignore-saved-network`) or `fell_back` (they could not bind, so the
    /// startup flags were used). Null when nothing was saved at startup.
    pub pending_status: Option<crate::SavedNetworkStatus>,
    /// False when session persistence is off, so nothing can be saved.
    pub can_save: bool,
}

/// A write of the network settings. Deserialized leniently so an unknown or
/// forbidden field is still visible to the validator.
#[derive(Deserialize, Default)]
pub struct NetworkConfigUpdate {
    #[serde(default)]
    pub listen_ip: Option<String>,
    #[serde(default)]
    pub listen_port: Option<u16>,
    #[serde(default)]
    pub dht_enabled: Option<bool>,
    #[serde(default)]
    pub pex_enabled: Option<bool>,
    #[serde(default)]
    pub lsd_enabled: Option<bool>,
    #[serde(default)]
    pub upnp_port_forward: Option<bool>,
    #[serde(default)]
    pub encryption: Option<String>,
    #[serde(default)]
    pub http_api_listen: Option<String>,
    #[serde(default)]
    pub disable_auth: Option<bool>,
    #[serde(default)]
    pub read_only: Option<bool>,
    /// Forget every saved value first, so the next start uses the startup
    /// flags. Other fields in the same body are then saved on top.
    #[serde(default)]
    pub reset: Option<bool>,
}

/// Check a network-settings write. Returns the settings to save for the next
/// start, or why the write was refused.
pub fn validate_network_update(
    update: &NetworkConfigUpdate,
) -> std::result::Result<crate::session_persistence::NetworkSettings, &'static str> {
    if update.http_api_listen.is_some()
        || update.disable_auth.is_some()
        || update.read_only.is_some()
    {
        return Err(
            "refusing to change the HTTP API bind address or authentication; \
                that would expose the web UI. Set --http-api-listen-addr at startup \
                and keep authentication on for any non-loopback bind",
        );
    }
    if update.encryption.is_some() {
        return Err("this engine has no encryption-mode switch");
    }
    if update.pex_enabled.is_some() {
        return Err("peer exchange is always on and cannot be turned off");
    }
    if update.listen_port == Some(0) {
        return Err("listen_port must be between 1 and 65535");
    }
    let listen_ip = match update.listen_ip.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) if raw.contains('%') => {
            return Err(
                "listen_ip cannot carry an IPv6 zone (%interface); give the address \
                 without it, or pick the interface with --bind-device",
            );
        }
        Some(raw) => Some(
            raw.trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .map_err(|_| "listen_ip must be an IPv4 or IPv6 address")?,
        ),
    };
    let settings = crate::session_persistence::NetworkSettings {
        listen_ip,
        listen_port: update.listen_port,
        dht_enabled: update.dht_enabled,
        lsd_enabled: update.lsd_enabled,
        upnp_port_forward: update.upnp_port_forward,
    };
    if settings.is_empty() && update.reset != Some(true) {
        return Err("no network setting was given");
    }
    Ok(settings)
}

pub async fn h_get_network(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let session = state.api.session();
    let listen = session.listen_addr();
    // Display only. The listener itself is not reachable from here, and this
    // handler never rebinds it.
    let http_api_listen = state
        .opts
        .web_ui_port
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Ok(Json(NetworkConfig {
        listen_ip: listen.map(|addr| addr.ip().to_string()),
        listen_port: listen.map(|addr| addr.port()),
        dht_enabled: session.get_dht().is_some(),
        pex_enabled: true,
        lsd_enabled: session.lsd.is_some(),
        upnp_port_forward: session.upnp_port_forward(),
        encryption: None,
        http_api_listen,
        pending: session.pending_network(),
        pending_status: session.saved_network_status(),
        can_save: session.has_persistence(),
    }))
}

pub async fn h_set_network(
    State(state): State<ApiState>,
    Json(update): Json<NetworkConfigUpdate>,
) -> axum::response::Response {
    // A request to move the HTTP API listener, or to turn authentication off,
    // is refused before any other field is looked at. Nothing is applied to
    // the running process: the values are saved for the next start.
    let settings = match validate_network_update(&update) {
        Ok(settings) => settings,
        Err(message) => return ApiError::from((StatusCode::BAD_REQUEST, message)).into_response(),
    };
    let session = state.api.session();
    if !session.has_persistence() {
        return ApiError::from((
            StatusCode::CONFLICT,
            "session persistence is off, so network settings cannot be saved for \
             the next start; set them with the startup flags instead",
        ))
        .into_response();
    }
    if update.reset == Some(true) {
        session.clear_pending_network();
    }
    if !settings.is_empty() {
        session.set_pending_network(settings);
    }
    if let Err(err) = session.persist_session_settings().await {
        tracing::warn!("could not save network settings: {err:#}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "could not save network settings" })),
        )
            .into_response();
    }
    let message = if session.pending_network().is_none() {
        "Saved network settings cleared. The next start uses the startup flags."
    } else {
        "Network settings saved. Restart to apply them."
    };
    Json(serde_json::json!({
        "restart_required": true,
        "message": message,
    }))
    .into_response()
}

// --- Directory browsing ---

#[derive(Deserialize)]
pub struct BrowseQuery {
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Serialize)]
pub struct BrowseResponse {
    pub current: String,
    pub parent: Option<String>,
    pub entries: Vec<DirEntry>,
}

/// The folders the browser may show: the session's download folder and, when set, its
/// completed folder. Folders that do not exist are left out.
fn browse_roots(state: &ApiState) -> Vec<PathBuf> {
    std::iter::once(state.api.api_output_folder())
        .chain(state.api.api_completed_folder())
        .filter_map(|folder| PathBuf::from(folder).canonicalize().ok())
        .collect()
}

/// Resolves a browse request to a canonical directory inside one of `roots`. With no
/// path, the first root is used. Symlinks and `..` are resolved before the check, so
/// neither can step out of a root.
fn resolve_browse_path(requested: Option<PathBuf>, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = match requested {
        Some(path) => path,
        None => roots
            .first()
            .cloned()
            .with_status_error(StatusCode::FORBIDDEN, "no download folder is configured")?,
    };

    let canonical = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("invalid path: {e}"))?;
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(ApiError::from((
            StatusCode::FORBIDDEN,
            "path is outside the download folders",
        )));
    }
    Ok(canonical)
}

pub async fn h_browse_directory(
    State(state): State<ApiState>,
    Query(query): Query<BrowseQuery>,
) -> Result<impl IntoResponse> {
    let roots = browse_roots(&state);
    let canonical = resolve_browse_path(query.path.map(PathBuf::from), &roots)?;

    // The parent is only offered while it is still inside a root.
    let parent = canonical
        .parent()
        .filter(|parent| roots.iter().any(|root| parent.starts_with(root)))
        .map(|p| p.to_string_lossy().into_owned());

    let mut entries = Vec::new();
    let read_dir =
        std::fs::read_dir(&canonical).map_err(|e| anyhow::anyhow!("cannot read directory: {e}"))?;

    for entry in read_dir.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip hidden files/dirs
        if name.starts_with('.') {
            continue;
        }
        let entry_path = entry.path();
        entries.push(DirEntry {
            name,
            path: entry_path.to_string_lossy().into_owned(),
            is_dir: file_type.is_dir(),
        });
    }

    // Sort: directories first, then alphabetically
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(Json(BrowseResponse {
        current: canonical.to_string_lossy().into_owned(),
        parent,
        entries,
    }))
}

// --- Proxy and IP filter (live) ---

#[derive(Serialize, Deserialize)]
pub struct ProxyConfigBody {
    /// `socks5://` or `http://` URL. A password in a POST is stored, never echoed.
    /// GET returns the same URL with the password replaced by `***`, or null.
    pub proxy_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct IpFilterBody {
    pub filter: String,
}

pub async fn h_get_proxy(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(ProxyConfigBody {
        proxy_url: state.api.session().proxy_url_redacted(),
    }))
}

pub async fn h_set_proxy(
    State(state): State<ApiState>,
    Json(body): Json<ProxyConfigBody>,
) -> axum::response::Response {
    let raw = body
        .proxy_url
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    if let Err(err) = state.api.session().set_proxy_url(raw) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response();
    }
    if let Err(err) = state.api.session().persist_session_settings().await {
        tracing::warn!("could not save proxy: {err:#}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "could not save proxy" })),
        )
            .into_response();
    }
    // Peer TCP picks the proxy up live. Trackers, DHT, UDP trackers and LSD
    // keep the previous address until the process restarts. The URL is on disk,
    // so that restart still has it.
    Json(serde_json::json!({
        "restart_required": true,
        "message": "Proxy saved. Restart to apply it to trackers, DHT and local discovery. Peer connections use it immediately."
    })).into_response()
}

pub async fn h_get_ip_filter(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(IpFilterBody {
        filter: state.api.session().ip_filter_text(),
    }))
}

pub async fn h_set_ip_filter(
    State(state): State<ApiState>,
    Json(body): Json<IpFilterBody>,
) -> axum::response::Response {
    match state.api.session().set_ip_filter(&body.filter) {
        Ok(()) => Json(EmptyJsonResponse {}).into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn canonical_roots(dirs: &[&Path]) -> Vec<PathBuf> {
        dirs.iter()
            .map(|dir| dir.canonicalize().expect("canonical test root"))
            .collect()
    }

    fn network_update(patch: impl FnOnce(&mut NetworkConfigUpdate)) -> NetworkConfigUpdate {
        let mut update = NetworkConfigUpdate::default();
        patch(&mut update);
        update
    }

    #[test]
    fn network_update_refuses_a_public_http_bind_without_auth() {
        let expose = network_update(|update| {
            update.http_api_listen = Some("0.0.0.0:3030".to_string());
            update.disable_auth = Some(true);
            update.listen_port = Some(6881);
        });
        let message = validate_network_update(&expose).unwrap_err();
        assert!(message.contains("expose the web UI"), "{message}");

        let bind_only = network_update(|update| {
            update.http_api_listen = Some("0.0.0.0:3030".to_string());
        });
        assert!(
            validate_network_update(&bind_only)
                .unwrap_err()
                .contains("expose the web UI")
        );

        let auth_off = network_update(|update| update.disable_auth = Some(true));
        assert!(
            validate_network_update(&auth_off)
                .unwrap_err()
                .contains("expose the web UI")
        );
    }

    #[test]
    fn network_update_refuses_unsupported_and_invalid_values() {
        let encryption = network_update(|update| update.encryption = Some("forced".to_string()));
        assert!(
            validate_network_update(&encryption)
                .unwrap_err()
                .contains("encryption-mode")
        );
        let pex = network_update(|update| update.pex_enabled = Some(false));
        assert!(
            validate_network_update(&pex)
                .unwrap_err()
                .contains("peer exchange")
        );
        let port = network_update(|update| update.listen_port = Some(0));
        assert!(
            validate_network_update(&port)
                .unwrap_err()
                .contains("listen_port")
        );
        let ip = network_update(|update| update.listen_ip = Some("not-an-ip".to_string()));
        assert!(
            validate_network_update(&ip)
                .unwrap_err()
                .contains("listen_ip")
        );
        assert!(validate_network_update(&NetworkConfigUpdate::default()).is_err());
        let zone = network_update(|update| update.listen_ip = Some("fe80::1%eth0".to_string()));
        assert!(
            validate_network_update(&zone)
                .unwrap_err()
                .contains("IPv6 zone")
        );
    }

    async fn network_state(
        persist: Option<&Path>,
    ) -> (ApiState, std::sync::Arc<crate::Session>, tempfile::TempDir) {
        let output = tempfile::TempDir::new().unwrap();
        let session = crate::Session::new_with_opts(
            output.path().to_owned(),
            crate::SessionOptions {
                disable_dht: true,
                disable_dht_persistence: true,
                disable_local_service_discovery: true,
                persistence: persist.map(|folder| crate::SessionPersistenceConfig::Json {
                    folder: Some(folder.to_owned()),
                }),
                listen: Some(crate::ListenerOptions {
                    listen_addr: (std::net::Ipv4Addr::LOCALHOST, 0).into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = crate::Api::new(
            session.clone(),
            None,
            #[cfg(feature = "tracing-subscriber-utils")]
            None,
        );
        let state = std::sync::Arc::new(crate::http_api::HttpApi::new(api, None));
        (state, session, output)
    }

    #[tokio::test]
    async fn set_network_without_persistence_is_a_conflict() {
        let (state, session, _output) = network_state(None).await;
        let update = network_update(|update| update.listen_port = Some(6881));
        let resp = h_set_network(State(state), Json(update)).await;
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("persistence is off"));
        assert_eq!(session.pending_network(), None);
        session.stop().await;
    }

    #[tokio::test]
    async fn set_network_reset_clears_saved_values() {
        let persist = tempfile::TempDir::new().unwrap();
        let (state, session, _output) = network_state(Some(persist.path())).await;
        let save = network_update(|update| {
            update.listen_port = Some(6881);
            update.dht_enabled = Some(false);
        });
        let resp = h_set_network(State(state.clone()), Json(save)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(session.pending_network().unwrap().listen_port, Some(6881));

        let reset = network_update(|update| update.reset = Some(true));
        let resp = h_set_network(State(state), Json(reset)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("cleared"));
        assert_eq!(session.pending_network(), None);
        session.stop().await;
    }

    #[test]
    fn network_update_reset_alone_is_accepted() {
        let reset = network_update(|update| update.reset = Some(true));
        assert!(validate_network_update(&reset).unwrap().is_empty());
        let bracketed = network_update(|update| update.listen_ip = Some("[::1]".to_string()));
        assert_eq!(
            validate_network_update(&bracketed).unwrap().listen_ip,
            Some("::1".parse().unwrap())
        );
    }

    #[test]
    fn network_update_accepts_startup_settings() {
        let update = network_update(|update| {
            update.listen_ip = Some(" ::1 ".to_string());
            update.listen_port = Some(6881);
            update.dht_enabled = Some(false);
            update.lsd_enabled = Some(true);
            update.upnp_port_forward = Some(false);
        });
        let settings = validate_network_update(&update).unwrap();
        assert_eq!(settings.listen_ip, Some("::1".parse().unwrap()));
        assert_eq!(settings.listen_port, Some(6881));
        assert_eq!(settings.dht_enabled, Some(false));
        assert_eq!(settings.lsd_enabled, Some(true));
        assert_eq!(settings.upnp_port_forward, Some(false));
    }

    #[test]
    fn browse_defaults_to_first_root_and_allows_subdirectories() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let downloads = temp.path().join("downloads");
        std::fs::create_dir_all(downloads.join("movies")).expect("create folders");
        let roots = canonical_roots(&[&downloads]);

        assert_eq!(
            resolve_browse_path(None, &roots).expect("default"),
            roots[0]
        );
        let movies = resolve_browse_path(Some(downloads.join("movies")), &roots).expect("subdir");
        assert!(movies.ends_with("movies"));
    }

    #[test]
    fn browse_refuses_paths_outside_the_roots() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let downloads = temp.path().join("downloads");
        // Shares a string prefix with the root but is a different folder.
        let lookalike = temp.path().join("downloads-old");
        let secret = temp.path().join("secret");
        for dir in [&downloads, &lookalike, &secret] {
            std::fs::create_dir_all(dir).expect("create folders");
        }
        let roots = canonical_roots(&[&downloads]);

        assert!(resolve_browse_path(Some(PathBuf::from("/")), &roots).is_err());
        assert!(resolve_browse_path(Some(secret), &roots).is_err());
        assert!(resolve_browse_path(Some(lookalike), &roots).is_err());
        assert!(resolve_browse_path(Some(downloads.join("..")), &roots).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn browse_refuses_symlinks_that_leave_the_roots() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let downloads = temp.path().join("downloads");
        let secret = temp.path().join("secret");
        std::fs::create_dir_all(&downloads).expect("create downloads");
        std::fs::create_dir_all(&secret).expect("create secret");
        std::os::unix::fs::symlink(&secret, downloads.join("escape")).expect("symlink");
        let roots = canonical_roots(&[&downloads]);

        assert!(resolve_browse_path(Some(downloads.join("escape")), &roots).is_err());
    }

    fn advanced(patch: impl FnOnce(&mut AdvancedConfig)) -> AdvancedConfig {
        let mut config = AdvancedConfig::default();
        patch(&mut config);
        config
    }

    /// Every knob with its bounds, as a setter and a getter on the body.
    type Knob = (&'static str, u64, u64, fn(&mut AdvancedConfig, Option<u64>));

    fn knobs() -> [Knob; 7] {
        [
            (
                "connect_timeout_secs",
                MIN_CONNECT_TIMEOUT_SECS,
                MAX_CONNECT_TIMEOUT_SECS,
                |c, v| c.connect_timeout_secs = v,
            ),
            (
                "read_write_timeout_secs",
                MIN_READ_WRITE_TIMEOUT_SECS,
                MAX_READ_WRITE_TIMEOUT_SECS,
                |c, v| c.read_write_timeout_secs = v,
            ),
            (
                "keep_alive_secs",
                MIN_KEEP_ALIVE_SECS,
                MAX_KEEP_ALIVE_SECS,
                |c, v| c.keep_alive_secs = v,
            ),
            (
                "upload_slots",
                MIN_UPLOAD_SLOTS,
                MAX_UPLOAD_SLOTS,
                |c, v| c.upload_slots = v,
            ),
            (
                "magnet_resolution_timeout_secs",
                MIN_MAGNET_RESOLUTION_TIMEOUT_SECS,
                MAX_MAGNET_RESOLUTION_TIMEOUT_SECS,
                |c, v| c.magnet_resolution_timeout_secs = v,
            ),
            (
                "max_concurrent_handshakes",
                MIN_MAX_CONCURRENT_HANDSHAKES,
                MAX_MAX_CONCURRENT_HANDSHAKES,
                |c, v| c.max_concurrent_handshakes = v,
            ),
            (
                "fastresume_validation_denom",
                MIN_FASTRESUME_VALIDATION_DENOM,
                MAX_FASTRESUME_VALIDATION_DENOM,
                |c, v| c.fastresume_validation_denom = v,
            ),
        ]
    }

    #[test]
    fn advanced_bounds_are_the_documented_ones() {
        let got: Vec<_> = knobs().iter().map(|k| (k.0, k.1, k.2)).collect();
        assert_eq!(
            got,
            vec![
                ("connect_timeout_secs", 1, 300),
                ("read_write_timeout_secs", 1, 600),
                ("keep_alive_secs", 1, 600),
                ("upload_slots", 1, 64),
                ("magnet_resolution_timeout_secs", 10, 900),
                ("max_concurrent_handshakes", 8, 512),
                ("fastresume_validation_denom", 0, 1000),
            ]
        );
        assert_eq!((MIN_PEER_LIMIT, MAX_PEER_LIMIT), (1, 5000));
        assert_eq!(
            (MIN_CONCURRENT_INIT_LIMIT, MAX_CONCURRENT_INIT_LIMIT),
            (1, 32)
        );
    }

    #[test]
    fn advanced_validation_accepts_min_max_null_and_refuses_outside() {
        for (name, min, max, set) in knobs() {
            // Null is the default and always passes.
            assert!(validate_advanced(&advanced(|c| set(c, None))).is_ok());
            // Both bounds are allowed.
            for ok in [min, max] {
                let settings = validate_advanced(&advanced(|c| set(c, Some(ok)))).unwrap();
                assert_eq!(
                    serde_json::to_value(settings).unwrap()[name],
                    serde_json::json!(ok),
                    "{name}={ok}"
                );
            }
            // One over the cap is refused with a 400 that names the field.
            let err = validate_advanced(&advanced(|c| set(c, Some(max + 1)))).unwrap_err();
            assert_eq!(err.status(), StatusCode::BAD_REQUEST, "{name}");
            assert!(format!("{err}").contains(name), "{name}: {err}");
            // One under the minimum, where there is room below it.
            if min > 0 {
                let err = validate_advanced(&advanced(|c| set(c, Some(min - 1)))).unwrap_err();
                assert_eq!(err.status(), StatusCode::BAD_REQUEST, "{name}");
                assert!(format!("{err}").contains(name), "{name}: {err}");
            }
        }
    }

    #[test]
    fn advanced_body_decodes_old_three_field_and_empty_bodies() {
        let old: AdvancedConfig = serde_json::from_str(
            r#"{"connect_timeout_secs":10,"read_write_timeout_secs":null,"keep_alive_secs":120}"#,
        )
        .unwrap();
        assert_eq!(old.connect_timeout_secs, Some(10));
        assert_eq!(old.read_write_timeout_secs, None);
        assert_eq!(old.keep_alive_secs, Some(120));
        assert_eq!(old.upload_slots, None);
        assert_eq!(old.fastresume_validation_denom, None);
        assert!(validate_advanced(&old).is_ok());
        let empty: AdvancedConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, AdvancedConfig::default());
        let nulls: AdvancedConfig = serde_json::from_str(
            r#"{"upload_slots":null,"magnet_resolution_timeout_secs":null,
                "max_concurrent_handshakes":null,"fastresume_validation_denom":null}"#,
        )
        .unwrap();
        assert_eq!(nulls, AdvancedConfig::default());
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)
    }

    #[tokio::test]
    async fn set_advanced_applies_live_knobs_and_old_body_still_works() {
        let (state, session, _output) = network_state(None).await;
        // Old 3-field body.
        let resp = h_set_advanced(
            State(state.clone()),
            Json(advanced(|c| {
                c.connect_timeout_secs = Some(15);
                c.read_write_timeout_secs = Some(20);
                c.keep_alive_secs = Some(90);
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["restart_required"], false);
        assert_eq!(
            session.peer_opts().connect_timeout,
            Some(Duration::from_secs(15))
        );
        assert_eq!(session.upload_slots(), 4);
        assert_eq!(session.magnet_resolution_timeout_secs(), 120);

        // The new live knobs.
        let resp = h_set_advanced(
            State(state.clone()),
            Json(advanced(|c| {
                c.upload_slots = Some(64);
                c.magnet_resolution_timeout_secs = Some(10);
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(session.upload_slots(), 64);
        assert_eq!(session.magnet_resolution_timeout_secs(), 10);
        // A body without the timeouts clears them back to the engine default.
        assert_eq!(session.peer_opts().connect_timeout, None);

        // Null puts a knob back to its default.
        let resp = h_set_advanced(State(state.clone()), Json(advanced(|_| {}))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(session.upload_slots(), 4);
        assert_eq!(session.magnet_resolution_timeout_secs(), 120);

        // GET reports the live values.
        let resp = h_get_advanced(State(state)).await.unwrap().into_response();
        let got = body_json(resp).await;
        assert_eq!(got["restart_required"], false);
        assert_eq!(got["can_save"], false);
        assert!(got["pending_restart"].is_null());
        session.stop().await;
    }

    #[tokio::test]
    async fn set_advanced_refusal_changes_nothing_and_names_the_field() {
        let (state, session, _output) = network_state(None).await;
        let resp = h_set_advanced(
            State(state.clone()),
            Json(advanced(|c| c.upload_slots = Some(8))),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        // A good field next to a bad one is not half-applied.
        let resp = h_set_advanced(
            State(state),
            Json(advanced(|c| {
                c.upload_slots = Some(2);
                c.magnet_resolution_timeout_secs = Some(9);
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert!(
            body["human_readable"]
                .as_str()
                .unwrap()
                .contains("magnet_resolution_timeout_secs")
        );
        assert_eq!(session.upload_slots(), 8);
        session.stop().await;
    }

    #[tokio::test]
    async fn set_advanced_restart_knob_without_persistence_is_a_conflict() {
        let (state, session, _output) = network_state(None).await;
        for patch in [
            (|c: &mut AdvancedConfig| c.max_concurrent_handshakes = Some(128))
                as fn(&mut AdvancedConfig),
            |c| c.fastresume_validation_denom = Some(0),
        ] {
            let resp = h_set_advanced(
                State(state.clone()),
                Json(advanced(|c| {
                    c.upload_slots = Some(16);
                    patch(c);
                })),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::CONFLICT);
            let body = body_json(resp).await;
            assert!(
                body["human_readable"]
                    .as_str()
                    .unwrap()
                    .contains("persistence is off")
            );
            // The live knob in the same body still applied.
            assert_eq!(session.upload_slots(), 16);
            assert_eq!(session.saved_advanced().max_concurrent_handshakes, None);
            assert_eq!(session.saved_advanced().fastresume_validation_denom, None);
        }
        session.stop().await;
    }

    #[tokio::test]
    async fn advanced_settings_round_trip_across_a_session_reload() {
        let persist = tempfile::TempDir::new().unwrap();
        let (state, session, _output) = network_state(Some(persist.path())).await;
        let resp = h_set_advanced(
            State(state.clone()),
            Json(AdvancedConfig {
                connect_timeout_secs: Some(31),
                read_write_timeout_secs: Some(32),
                keep_alive_secs: Some(33),
                upload_slots: Some(12),
                magnet_resolution_timeout_secs: Some(240),
                max_concurrent_handshakes: Some(200),
                fastresume_validation_denom: Some(0),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["restart_required"], true);
        // Saved restart knobs are pending, not running.
        assert_eq!(session.max_concurrent_handshakes(), 64);
        let got = body_json(
            h_get_advanced(State(state.clone()))
                .await
                .unwrap()
                .into_response(),
        )
        .await;
        assert_eq!(got["restart_required"], true);
        assert_eq!(got["pending_restart"]["max_concurrent_handshakes"], 200);
        assert_eq!(got["pending_restart"]["fastresume_validation_denom"], 0);
        assert_eq!(got["can_save"], true);
        session.stop().await;

        // A new session over the same persistence folder.
        let output = tempfile::TempDir::new().unwrap();
        let session = crate::Session::new_with_opts(
            output.path().to_owned(),
            crate::SessionOptions {
                disable_dht: true,
                disable_dht_persistence: true,
                disable_local_service_discovery: true,
                persistence: Some(crate::SessionPersistenceConfig::Json {
                    folder: Some(persist.path().to_owned()),
                }),
                listen: Some(crate::ListenerOptions {
                    listen_addr: (std::net::Ipv4Addr::LOCALHOST, 0).into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let opts = session.peer_opts();
        assert_eq!(opts.connect_timeout, Some(Duration::from_secs(31)));
        assert_eq!(opts.read_write_timeout, Some(Duration::from_secs(32)));
        assert_eq!(opts.keep_alive_interval, Some(Duration::from_secs(33)));
        assert_eq!(session.upload_slots(), 12);
        assert_eq!(session.magnet_resolution_timeout_secs(), 240);
        // The restart knobs are now the running ones.
        assert_eq!(session.max_concurrent_handshakes(), 200);
        assert_eq!(session.fastresume_validation_denom, Some(0));
        assert_eq!(
            session.saved_advanced(),
            session.startup_advanced(),
            "nothing is pending after the restart"
        );
        session.stop().await;
    }

    #[tokio::test]
    async fn update_limits_enforces_minimum_and_caps() {
        let (state, session, _output) = network_state(None).await;
        let limits = |peer: Option<usize>, init: Option<usize>| LimitsConfig {
            peer_limit: peer,
            concurrent_init_limit: init,
            ..Default::default()
        };
        // (peer_limit, concurrent_init_limit, status, field named in the refusal)
        #[allow(clippy::type_complexity)]
        let cases: [(Option<usize>, Option<usize>, StatusCode, &str); 9] = [
            (Some(1), Some(1), StatusCode::OK, ""),
            (Some(5000), Some(32), StatusCode::OK, ""),
            (None, None, StatusCode::OK, ""),
            (Some(0), None, StatusCode::BAD_REQUEST, "peer_limit"),
            (Some(5001), None, StatusCode::BAD_REQUEST, "peer_limit"),
            (
                None,
                Some(0),
                StatusCode::BAD_REQUEST,
                "concurrent_init_limit",
            ),
            (
                None,
                Some(33),
                StatusCode::BAD_REQUEST,
                "concurrent_init_limit",
            ),
            (
                Some(10),
                Some(33),
                StatusCode::BAD_REQUEST,
                "concurrent_init_limit",
            ),
            (Some(0), Some(5), StatusCode::BAD_REQUEST, "peer_limit"),
        ];
        for (peer, init, status, field) in cases {
            let before = (
                session.get_peer_limit(),
                session.get_concurrent_init_limit(),
            );
            let resp =
                h_update_session_ratelimits(State(state.clone()), Json(limits(peer, init))).await;
            assert_eq!(resp.status(), status, "{peer:?} {init:?}");
            if status == StatusCode::OK {
                assert_eq!(
                    session.get_peer_limit(),
                    peer.unwrap_or(before.0),
                    "{peer:?}"
                );
                assert_eq!(
                    session.get_concurrent_init_limit(),
                    init.unwrap_or(before.1),
                    "{init:?}"
                );
            } else {
                let body = body_json(resp).await;
                assert!(
                    body["human_readable"].as_str().unwrap().contains(field),
                    "{body}"
                );
                // A refusal changes nothing.
                assert_eq!(
                    (
                        session.get_peer_limit(),
                        session.get_concurrent_init_limit()
                    ),
                    before
                );
            }
        }
        session.stop().await;
    }

    #[test]
    fn browse_refuses_everything_when_no_root_exists() {
        let temp = tempfile::tempdir().expect("temporary directory");
        assert!(resolve_browse_path(None, &[]).is_err());
        assert!(resolve_browse_path(Some(temp.path().to_path_buf()), &[]).is_err());
    }

    /// The daemon's `AdvancedConfig` and the client's share one field set,
    /// pinned by the client's `advanced.json` fixture. A field the client sends
    /// that the daemon lacks would be dropped silently on save.
    #[test]
    fn advanced_config_field_set_matches_the_client_fixture() {
        use std::collections::BTreeSet;

        let keys = |value: serde_json::Value| -> BTreeSet<String> {
            value.as_object().unwrap().keys().cloned().collect()
        };
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../rtbit-client/tests/fixtures/advanced.json");
        let text = std::fs::read_to_string(path).expect("client fixture");
        let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();

        let daemon = keys(serde_json::to_value(AdvancedConfig::default()).unwrap());
        assert_eq!(daemon, keys(fixture.clone()), "daemon vs client fixture");
        assert!(!daemon.contains("peer_limit") && !daemon.contains("concurrent_init_limit"));

        // Every fixture key lands in a daemon field (none are dropped).
        let parsed: AdvancedConfig = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(keys(serde_json::to_value(parsed).unwrap()), keys(fixture));
    }
}

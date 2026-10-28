use std::{path::PathBuf, time::Duration};

use axum::{Json, extract::Query, extract::State, response::IntoResponse};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use super::ApiState;
use crate::{
    ApiError, WithStatusError,
    api::{EmptyJsonResponse, Result},
    limits::LimitsConfig,
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

/// Hard caps for the Advanced tab. A value above its cap is refused.
pub const MAX_CONNECT_TIMEOUT_SECS: u64 = 300;
pub const MAX_READ_WRITE_TIMEOUT_SECS: u64 = 600;
pub const MAX_KEEP_ALIVE_SECS: u64 = 600;

/// Session-wide peer connection timeouts, in seconds. `None` means the engine default.
#[derive(Serialize, Deserialize)]
pub struct AdvancedConfig {
    pub connect_timeout_secs: Option<u64>,
    pub read_write_timeout_secs: Option<u64>,
    pub keep_alive_secs: Option<u64>,
}

fn secs_of(d: Option<Duration>) -> Option<u64> {
    d.map(|d| d.as_secs())
}

fn reject(msg: &'static str) -> crate::ApiError {
    ApiError::from((StatusCode::BAD_REQUEST, msg))
}

fn capped(value: Option<u64>, cap: u64) -> Result<Option<Duration>> {
    match value {
        None => Ok(None),
        Some(0) => Err(reject("timeout must be at least 1 second")),
        Some(v) if v > cap => Err(reject("timeout exceeds its cap")),
        Some(v) => Ok(Some(Duration::from_secs(v))),
    }
}

pub async fn h_get_advanced(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let opts = state.api.session().peer_opts();
    Ok(Json(AdvancedConfig {
        connect_timeout_secs: secs_of(opts.connect_timeout),
        read_write_timeout_secs: secs_of(opts.read_write_timeout),
        keep_alive_secs: secs_of(opts.keep_alive_interval),
    }))
}

pub async fn h_set_advanced(
    State(state): State<ApiState>,
    Json(config): Json<AdvancedConfig>,
) -> axum::response::Response {
    let connect_timeout = match capped(config.connect_timeout_secs, MAX_CONNECT_TIMEOUT_SECS) {
        Ok(v) => v,
        Err(err) => return err.into_response(),
    };
    let read_write_timeout =
        match capped(config.read_write_timeout_secs, MAX_READ_WRITE_TIMEOUT_SECS) {
            Ok(v) => v,
            Err(err) => return err.into_response(),
        };
    let keep_alive_interval = match capped(config.keep_alive_secs, MAX_KEEP_ALIVE_SECS) {
        Ok(v) => v,
        Err(err) => return err.into_response(),
    };
    let opts = crate::PeerConnectionOptions {
        connect_timeout,
        read_write_timeout,
        keep_alive_interval,
    };
    state.api.session().set_peer_opts(opts);
    Json(EmptyJsonResponse {}).into_response()
}

// --- Network settings (startup only) ---

/// Peer and discovery settings as the running process actually has them.
/// Every field is fixed at startup. `encryption` is always null: this engine
/// has no encryption-mode switch. `pex_enabled` is always true: peer exchange
/// is part of the peer protocol and has no flag. `http_api_listen` is the
/// address this request arrived on, for display only.
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
}

/// Why a network-settings write was refused. `None` means the body asked for
/// nothing, which is still not applied: there is no runtime setter.
pub fn reject_network_update(update: &NetworkConfigUpdate) -> &'static str {
    if update.http_api_listen.is_some() || update.disable_auth == Some(true) {
        return "refusing to change the HTTP API bind address or authentication; \
                that would expose the web UI. Set --http-api-listen-addr at startup \
                and keep authentication on for any non-loopback bind";
    }
    if update.listen_ip.is_some()
        || update.listen_port.is_some()
        || update.dht_enabled.is_some()
        || update.pex_enabled.is_some()
        || update.lsd_enabled.is_some()
        || update.upnp_port_forward.is_some()
        || update.encryption.is_some()
        || update.read_only.is_some()
        || update.disable_auth.is_some()
    {
        return "these settings are fixed at startup and need a restart; \
                set them with --listen-port, --listen-ip, --disable-dht, \
                --disable-lsd or --disable-upnp-port-forward. Peer exchange is \
                always on, and this engine has no encryption-mode switch";
    }
    "no network setting was changed; these values are fixed at startup"
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
    }))
}

pub async fn h_set_network(Json(update): Json<NetworkConfigUpdate>) -> axum::response::Response {
    // Nothing here is applied. In particular a request to move the HTTP API
    // listener, or to turn authentication off, is refused before any other
    // field is looked at, and there is no setter that could honour it.
    let message = reject_network_update(&update);
    ApiError::from((StatusCode::BAD_REQUEST, message)).into_response()
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
        });
        let message = reject_network_update(&expose);
        assert!(message.contains("expose the web UI"), "{message}");

        let bind_only = network_update(|update| {
            update.http_api_listen = Some("0.0.0.0:3030".to_string());
        });
        assert!(reject_network_update(&bind_only).contains("expose the web UI"));

        let auth_off = network_update(|update| update.disable_auth = Some(true));
        assert!(reject_network_update(&auth_off).contains("expose the web UI"));
    }

    #[test]
    fn network_update_refuses_startup_settings_and_empty_bodies() {
        let port = network_update(|update| update.listen_port = Some(6881));
        let message = reject_network_update(&port);
        assert!(message.contains("restart"), "{message}");
        let encryption = network_update(|update| update.encryption = Some("forced".to_string()));
        assert!(reject_network_update(&encryption).contains("encryption-mode"));
        assert!(
            reject_network_update(&NetworkConfigUpdate::default()).contains("fixed at startup")
        );
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

    #[test]
    fn advanced_caps_reject_zero_and_over_cap_and_keep_empty() {
        assert!(capped(None, MAX_CONNECT_TIMEOUT_SECS).unwrap().is_none());
        assert_eq!(
            capped(Some(30), MAX_CONNECT_TIMEOUT_SECS)
                .unwrap()
                .unwrap()
                .as_secs(),
            30
        );
        assert!(capped(Some(0), MAX_CONNECT_TIMEOUT_SECS).is_err());
        assert!(capped(Some(MAX_CONNECT_TIMEOUT_SECS + 1), MAX_CONNECT_TIMEOUT_SECS).is_err());
        assert!(
            capped(
                Some(MAX_READ_WRITE_TIMEOUT_SECS + 1),
                MAX_READ_WRITE_TIMEOUT_SECS
            )
            .is_err()
        );
        assert!(capped(Some(MAX_KEEP_ALIVE_SECS + 1), MAX_KEEP_ALIVE_SECS).is_err());
        // The bound is the refusal: the largest allowed value is kept.
        assert_eq!(
            capped(Some(MAX_CONNECT_TIMEOUT_SECS), MAX_CONNECT_TIMEOUT_SECS)
                .unwrap()
                .unwrap()
                .as_secs(),
            MAX_CONNECT_TIMEOUT_SECS
        );
    }

    #[test]
    fn browse_refuses_everything_when_no_root_exists() {
        let temp = tempfile::tempdir().expect("temporary directory");
        assert!(resolve_browse_path(None, &[]).is_err());
        assert!(resolve_browse_path(Some(temp.path().to_path_buf()), &[]).is_err());
    }
}

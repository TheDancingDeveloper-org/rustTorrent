//! App controller (`app/*`): version, build info and preferences.

use super::*;

// ---------------------------------------------------------------------------
// App info endpoints
// ---------------------------------------------------------------------------

/// Advertised application version. *arr apps gate features on this string, so
/// it stays on the 5.x line that matches the advertised WebAPI version below.
pub(super) const COMPAT_APP_VERSION: &str = "v5.0.0";

pub(super) async fn h_app_version() -> &'static str {
    COMPAT_APP_VERSION
}

pub(super) async fn h_app_webapi_version() -> &'static str {
    "2.11.3"
}

pub(super) async fn h_app_default_save_path(State(state): State<Arc<QbitState>>) -> String {
    state.api_state.api.api_output_folder()
}

/// Configured download roots: the session output folder plus every category
/// save path.
fn save_roots(api: &crate::Api) -> Vec<std::path::PathBuf> {
    let mut roots = vec![std::path::PathBuf::from(api.api_output_folder())];
    roots.extend(
        api.api_list_categories()
            .into_values()
            .filter_map(|category| category.save_path),
    );
    roots
}

/// Whether `path` is a root or lies beneath one. Compared by path component,
/// and a `..` component never qualifies, so it cannot climb out of a root.
fn path_under_roots(path: &std::path::Path, roots: &[std::path::PathBuf]) -> bool {
    !path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
        && roots.iter().any(|root| path.starts_with(root))
}

/// Free bytes on the volume holding the default save path; 0 when the
/// platform cannot report it.
pub(super) fn free_space_on_save_volume(api: &crate::Api) -> u64 {
    crate::session::scheduler::free_disk_space(std::path::Path::new(&api.api_output_folder()))
        .unwrap_or(0)
}

#[derive(Deserialize, Default)]
pub(super) struct FreeSpaceQuery {
    #[serde(default)]
    pub(super) path: String,
}

/// `app/getFreeSpaceAtPath?path=` — bytes free on the volume holding `path`,
/// as plain text. Only paths at or under a configured download root (the
/// default save path or a category save path) are answered; anything else is
/// 400, so the endpoint cannot probe the rest of the filesystem.
pub(super) async fn h_app_get_free_space_at_path(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<FreeSpaceQuery>,
) -> axum::response::Response {
    let api = &state.api_state.api;
    let path = std::path::Path::new(&query.path);
    if query.path.is_empty() || !path_under_roots(path, &save_roots(api)) {
        return qbit_error(
            StatusCode::BAD_REQUEST,
            "path must be under a configured download folder",
        );
    }
    match crate::session::scheduler::free_disk_space(path) {
        Some(free) => (StatusCode::OK, free.to_string()).into_response(),
        None => qbit_error(
            StatusCode::BAD_REQUEST,
            "free space is not available for this path",
        ),
    }
}

pub(super) async fn h_app_build_info() -> impl IntoResponse {
    axum::Json(QbitBuildInfo {
        qt: "N/A",
        libtorrent: "N/A",
        boost: "N/A",
        openssl: "N/A",
        bitness: 64,
    })
}

pub(super) async fn h_app_preferences(
    State(state): State<Arc<QbitState>>,
) -> axum::Json<QbitPreferences> {
    let save_path = state.api_state.api.api_output_folder();
    let session = state.api_state.api.session();
    let (download, upload) = session.normal_rate_limits();
    let alt = session.alt_speed_status().config;
    let seed = session.seed_limits();
    let net = session.pending_network().unwrap_or_default();

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
        // A saved value applies at the next start; report it so a client that
        // just set it reads it back.
        dht: net
            .dht_enabled
            .unwrap_or_else(|| session.get_dht().is_some()),
        // Peer exchange is part of the peer protocol and has no switch.
        pex: true,
        lsd: net.lsd_enabled.unwrap_or_else(|| session.lsd.is_some()),
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
        max_ratio_enabled: seed.max_ratio.is_some(),
        max_ratio: seed.max_ratio.unwrap_or(2.0),
        // Reaching a limit always pauses the torrent.
        max_ratio_act: 0,
        max_seeding_time_enabled: seed.max_seeding_time.is_some(),
        max_seeding_time: minutes_or(seed.max_seeding_time, 1440),
        max_inactive_seeding_time_enabled: seed.max_inactive_seeding_time.is_some(),
        max_inactive_seeding_time: minutes_or(seed.max_inactive_seeding_time, 1440),
    })
}

/// Minutes on the wire for a global limit, or `default` when it is off.
fn minutes_or(limit: Option<std::time::Duration>, default: i64) -> i64 {
    limit.map_or(default, |d| i64::from(minutes_to_qbit(Some(d))))
}

/// Apply an enabled flag and value pair to one global share limit. A flag
/// alone keeps the current value, or `default` when there is none.
fn merge_limit<T: Copy>(
    current: Option<T>,
    enabled: Option<bool>,
    value: Option<T>,
    default: T,
) -> Option<T> {
    match enabled {
        Some(false) => None,
        Some(true) => value.or(current).or(Some(default)),
        None => current.map(|c| value.unwrap_or(c)),
    }
}

/// The global share limits after a preferences update.
fn share_limits_update(preferences: &SetPreferences, current: ShareLimits) -> ShareLimits {
    let minutes = |m: Option<i64>| m.and_then(minutes_from_qbit);
    ShareLimits {
        max_ratio: merge_limit(
            current.max_ratio,
            preferences.max_ratio_enabled,
            preferences.max_ratio.filter(|r| *r >= 0.0),
            2.0,
        ),
        max_seeding_time: merge_limit(
            current.max_seeding_time,
            preferences.max_seeding_time_enabled,
            minutes(preferences.max_seeding_time),
            std::time::Duration::from_secs(1440 * 60),
        ),
        max_inactive_seeding_time: merge_limit(
            current.max_inactive_seeding_time,
            preferences.max_inactive_seeding_time_enabled,
            minutes(preferences.max_inactive_seeding_time),
            std::time::Duration::from_secs(1440 * 60),
        ),
    }
}

/// `app/setPreferences`. `announce_port`, the global share limits
/// (`max_ratio*`, `max_seeding_time*`, `max_inactive_seeding_time*`), `dht` and
/// `lsd` (saved for the next start; `pex` is always on), the queue settings
/// (`queueing_enabled`, `max_active_downloads`, `max_active_uploads`) and the
/// normal and alternative speed limits (`dl_limit`, `up_limit`,
/// `alt_dl_limit`, `alt_up_limit`, bytes/s, 0 or -1 unlimited) are applied;
/// other keys are ignored. `announce_port`, when present, must be a
/// valid port. Absent keys leave their setting alone, so
/// read-modify-write clients that post their whole preferences object work.
pub(super) async fn h_app_set_preferences(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
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
    let current = session.seed_limits();
    let limits = share_limits_update(&preferences, current);
    if limits != current
        && let Err(error) = session.set_seed_limits(limits).await
    {
        warn!(%error, "qbit compat: error saving global share limits");
    }
    if preferences.dht.is_some() || preferences.lsd.is_some() {
        // Discovery is chosen at startup, so these are saved for the next start.
        if session.has_persistence() {
            session.set_pending_network(crate::session_persistence::NetworkSettings {
                dht_enabled: preferences.dht,
                lsd_enabled: preferences.lsd,
                ..Default::default()
            });
            if let Err(error) = session.persist_session_settings().await {
                warn!(%error, "qbit compat: error saving network settings");
            }
        }
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

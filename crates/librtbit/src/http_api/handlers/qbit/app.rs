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
        max_inactive_seeding_time_enabled: false,
        max_inactive_seeding_time: 1440,
    })
}

/// `app/setPreferences`. `announce_port`, the queue settings
/// (`queueing_enabled`, `max_active_downloads`, `max_active_uploads`) and the
/// normal and alternative speed limits (`dl_limit`, `up_limit`,
/// `alt_dl_limit`, `alt_up_limit`, bytes/s, 0 or -1 unlimited) are applied;
/// other keys are ignored. `announce_port`, when present, must be a
/// valid port. Absent keys leave their setting alone, so
/// read-modify-write clients that post their whole preferences object work.
pub(super) async fn h_app_set_preferences(
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

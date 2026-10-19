//! Transfer controller (`transfer/*`): global stats and session speed limits.

use super::*;

// ---------------------------------------------------------------------------
// Transfer info
// ---------------------------------------------------------------------------

pub(super) async fn h_transfer_info(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
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
pub(super) fn dht_nodes(state: &QbitState) -> u64 {
    state.api_state.api.api_dht_stats().map_or(0, |s| {
        (s.routing_table_size + s.routing_table_size_v6) as u64
    })
}

pub(super) fn bps_to_u64(bps: Option<NonZeroU32>) -> u64 {
    bps.map_or(0, |v| u64::from(v.get()))
}

/// Parse a qBittorrent byte-rate limit (0 means unlimited).
pub(super) fn limit_to_bps(limit: u64) -> Option<NonZeroU32> {
    NonZeroU32::new(u32::try_from(limit).unwrap_or(u32::MAX))
}

#[derive(Deserialize, Default)]
pub(super) struct LimitForm {
    #[serde(default)]
    pub(super) limit: u64,
}

#[derive(Deserialize, Default)]
pub(super) struct ModeForm {
    #[serde(default)]
    pub(super) mode: u8,
}

pub(super) async fn h_transfer_download_limit(
    State(state): State<Arc<QbitState>>,
) -> impl IntoResponse {
    let config = state.api_state.api.session().ratelimits.get_config();
    axum::Json(bps_to_u64(config.download_bps))
}

pub(super) async fn h_transfer_upload_limit(
    State(state): State<Arc<QbitState>>,
) -> impl IntoResponse {
    let config = state.api_state.api.session().ratelimits.get_config();
    axum::Json(bps_to_u64(config.upload_bps))
}

pub(super) async fn h_transfer_set_download_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: LimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let session = state.api_state.api.session();
    let (_, upload) = session.normal_rate_limits();
    session.set_normal_rate_limits(limit_to_bps(form.limit), upload);
    "Ok."
}

pub(super) async fn h_transfer_set_upload_limit(
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
pub(super) struct QbitSpeedLimits {
    pub(super) up_limit: u64,
    pub(super) dl_limit: u64,
    pub(super) alt_up_limit: u64,
    pub(super) alt_dl_limit: u64,
}

#[derive(Deserialize)]
pub(super) struct SpeedLimitsForm {
    pub(super) up_limit: u64,
    pub(super) dl_limit: u64,
    pub(super) alt_up_limit: u64,
    pub(super) alt_dl_limit: u64,
}

/// `transfer/getSpeedLimits` — normal and alternative session limits in
/// bytes/s (0 = unlimited), over the native limiter and `/speed/alt` config.
pub(super) async fn h_transfer_get_speed_limits(
    State(state): State<Arc<QbitState>>,
) -> impl IntoResponse {
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
pub(super) async fn h_transfer_set_speed_limits(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> Response {
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
pub(super) async fn h_transfer_speed_limits_mode(
    State(state): State<Arc<QbitState>>,
) -> &'static str {
    if state.api_state.api.session().alt_speed_enabled() {
        "1"
    } else {
        "0"
    }
}

pub(super) async fn h_transfer_toggle_speed_limits_mode(
    State(state): State<Arc<QbitState>>,
) -> &'static str {
    let session = state.api_state.api.session();
    session.set_alt_speed_enabled(!session.alt_speed_enabled());
    "Ok."
}

pub(super) async fn h_transfer_set_speed_limits_mode(
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
pub(super) async fn h_transfer_pause_session(State(state): State<Arc<QbitState>>) -> &'static str {
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, "all") {
        if let Err(error) = api.api_torrent_action_pause(idx).await {
            warn!(%error, "qbit compat: error pausing session torrent");
        }
    }
    "Ok."
}

/// `transfer/resumeSession` — resume every torrent (see pauseSession).
pub(super) async fn h_transfer_resume_session(State(state): State<Arc<QbitState>>) -> &'static str {
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, "all") {
        if let Err(error) = api.api_torrent_action_start(idx).await {
            warn!(%error, "qbit compat: error resuming session torrent");
        }
    }
    "Ok."
}

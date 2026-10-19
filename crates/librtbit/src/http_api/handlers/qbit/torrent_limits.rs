//! Torrents controller (`torrents/*`): per-torrent speed and share limits.

use super::*;

// ---------------------------------------------------------------------------
// Per-torrent speed limits
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub(super) struct HashesQuery {
    #[serde(default)]
    pub(super) hashes: String,
}

#[derive(Deserialize, Default)]
pub(super) struct SetTorrentLimitForm {
    #[serde(default)]
    pub(super) hashes: String,
    /// Bytes/s; <= 0 means unlimited.
    #[serde(default)]
    pub(super) limit: i64,
}

pub(super) fn torrent_limit_map(
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
pub(super) fn qbit_torrent_limit(bps: Option<NonZeroU32>) -> i32 {
    bps.map_or(-1, |bps| i32::try_from(bps.get()).unwrap_or(i32::MAX))
}

/// Set one direction of the speed limit on every torrent named by `hashes`,
/// keeping the other direction, and save it with the torrent.
pub(super) async fn set_torrent_limit(
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

pub(super) fn parse_torrent_limit(limit: i64) -> Option<NonZeroU32> {
    if limit <= 0 {
        None
    } else {
        limit_to_bps(limit as u64)
    }
}

pub(super) async fn h_torrents_download_limit(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashesQuery>,
) -> impl IntoResponse {
    let map = torrent_limit_map(&state.api_state.api, &query.hashes, |c| c.download_bps);
    axum::Json(map)
}

pub(super) async fn h_torrents_upload_limit(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashesQuery>,
) -> impl IntoResponse {
    let map = torrent_limit_map(&state.api_state.api, &query.hashes, |c| c.upload_bps);
    axum::Json(map)
}

pub(super) async fn h_torrents_set_download_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: SetTorrentLimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let bps = parse_torrent_limit(form.limit);
    set_torrent_limit(api, &form.hashes, bps, |l, bps| l.download_bps = bps).await;
    "Ok."
}

pub(super) async fn h_torrents_set_upload_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: SetTorrentLimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let bps = parse_torrent_limit(form.limit);
    set_torrent_limit(api, &form.hashes, bps, |l, bps| l.upload_bps = bps).await;
    "Ok."
}

pub(super) fn default_ratio_limit() -> f64 {
    -1.0
}

pub(super) fn default_seeding_time_limit() -> i64 {
    -1
}

#[derive(Deserialize)]
pub(super) struct SetShareLimitsForm {
    #[serde(default)]
    pub(super) hashes: String,
    #[serde(default = "default_ratio_limit", alias = "ratioLimit")]
    pub(super) ratio_limit: f64,
    #[serde(default = "default_seeding_time_limit", alias = "seedingTimeLimit")]
    pub(super) seeding_time_limit: i64,
    #[serde(default, alias = "inactiveSeedingTimeLimit")]
    pub(super) inactive_seeding_time_limit: Option<i64>,
}

/// `torrents/setShareLimits` — set per-torrent ratio, seeding-time and
/// inactive-seeding-time limits. The session's policy pass pauses a seed that
/// reaches any of them, and the limits are persisted with the torrent. An
/// omitted `inactiveSeedingTimeLimit` (older clients) keeps the current value.
pub(super) async fn h_torrents_set_share_limits(
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

//! Log controller (`log/*`) and peer bans.

use super::*;

// ---------------------------------------------------------------------------
// Log + peer bans
// ---------------------------------------------------------------------------

pub(super) fn log_filter_default() -> bool {
    true
}

#[derive(Deserialize)]
pub(super) struct LogMainQuery {
    #[serde(default = "log_filter_default")]
    pub(super) normal: bool,
    #[serde(default = "log_filter_default")]
    pub(super) info: bool,
    #[serde(default = "log_filter_default")]
    pub(super) warning: bool,
    #[serde(default = "log_filter_default")]
    pub(super) critical: bool,
    #[serde(default = "default_last_known_id")]
    pub(super) last_known_id: i64,
}

#[derive(Deserialize)]
pub(super) struct LogPeersQuery {
    #[serde(default = "default_last_known_id")]
    pub(super) last_known_id: i64,
}

pub(super) fn default_last_known_id() -> i64 {
    -1
}

/// `-1` (or any negative id) means "everything retained".
pub(super) fn after_id(last_known_id: i64) -> Option<u64> {
    u64::try_from(last_known_id).ok()
}

pub(super) async fn h_log_main(Query(q): Query<LogMainQuery>) -> Response {
    use crate::log_ring::LogSeverity;
    let entries = crate::log_ring::global().main_since(after_id(q.last_known_id), |s| match s {
        LogSeverity::Normal => q.normal,
        LogSeverity::Info => q.info,
        LogSeverity::Warning => q.warning,
        LogSeverity::Critical => q.critical,
    });
    axum::Json(entries).into_response()
}

pub(super) async fn h_log_peers(Query(q): Query<LogPeersQuery>) -> Response {
    axum::Json(crate::log_ring::global().peers_since(after_id(q.last_known_id))).into_response()
}

#[derive(Deserialize, Default)]
pub(super) struct BanPeersForm {
    /// Pipe-separated `host:port` peers; the port is ignored.
    #[serde(default)]
    pub(super) peers: String,
}

/// Parse one `transfer/banPeers` entry: `ip:port`, `[v6]:port` or a bare IP.
pub(super) fn parse_ban_peer(raw: &str) -> Option<std::net::IpAddr> {
    let raw = raw.trim();
    raw.parse::<std::net::SocketAddr>()
        .map(|a| a.ip())
        .or_else(|_| raw.parse::<std::net::IpAddr>())
        .ok()
}

pub(super) async fn h_transfer_ban_peers(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> Response {
    let form: BanPeersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let ips: Vec<_> = form.peers.split('|').filter_map(parse_ban_peer).collect();
    if ips.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "peers is required");
    }
    state.api_state.api.session().ban_peers(ips);
    "".into_response()
}

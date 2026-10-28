use serde::{Deserialize, Serialize};

/// What a caller may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    ReadOnly,
}

/// The token pair `POST /auth/login` and `POST /auth/refresh` return.
/// Debug masks both tokens; the role and expiry stay visible.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: u64,
    pub role: Role,
}

impl std::fmt::Debug for TokenPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenPair")
            .field("access_token", &"[redacted]")
            .field("refresh_token", &"[redacted]")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field("role", &self.role)
            .finish()
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct LoginBody<'a> {
    pub username: &'a str,
    pub password: &'a str,
}

impl std::fmt::Debug for LoginBody<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginBody")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct RefreshBody<'a> {
    pub refresh_token: &'a str,
}

impl std::fmt::Debug for RefreshBody<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshBody")
            .field("refresh_token", &"[redacted]")
            .finish()
    }
}

/// `POST /auth/setup`. The setup token and password are secrets.
#[derive(Clone, Serialize)]
pub(crate) struct SetupBody<'a> {
    pub username: &'a str,
    pub password: &'a str,
    pub setup_token: &'a str,
}

impl std::fmt::Debug for SetupBody<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupBody")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .field("setup_token", &"[redacted]")
            .finish()
    }
}

/// `GET /auth/status`.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthStatus {
    #[serde(default)]
    pub auth_enabled: bool,
    #[serde(default)]
    pub setup_required: bool,
}

#[cfg(test)]
mod secret_debug {
    use super::{LoginBody, RefreshBody, Role, SetupBody, TokenPair};

    #[test]
    fn debug_masks_login_and_refresh_secrets() {
        let login = LoginBody {
            username: "ada",
            password: "s3cret-pass",
        };
        let refresh = RefreshBody {
            refresh_token: "refresh-secret",
        };
        let setup = SetupBody {
            username: "ada",
            password: "s3cret-pass",
            setup_token: "setup-secret",
        };
        let shown = format!("{login:?} {refresh:?} {setup:?}");
        assert!(shown.contains("ada"));
        assert!(shown.contains("[redacted]"));
        assert!(!shown.contains("s3cret-pass"));
        assert!(!shown.contains("refresh-secret"));
        assert!(!shown.contains("setup-secret"));
    }

    #[test]
    fn debug_masks_token_pair_secrets() {
        let pair = TokenPair {
            access_token: "access-secret".into(),
            refresh_token: "refresh-secret".into(),
            token_type: "Bearer".into(),
            expires_in: 3600,
            role: Role::Admin,
        };
        let shown = format!("{pair:?}");
        assert!(shown.contains("[redacted]"));
        assert!(shown.contains("Bearer"));
        assert!(shown.contains("3600"));
        assert!(shown.contains("Admin"));
        assert!(!shown.contains("access-secret"));
        assert!(!shown.contains("refresh-secret"));
    }
}

/// Wire value of a torrent's `stats.state`. These strings are what the daemon
/// sends; the client does not rename them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TorrentState {
    Initializing,
    Live,
    Paused,
    Error,
    /// A state string this client does not know yet.
    #[serde(other)]
    Other,
}

impl TorrentState {
    /// The label the main screen shows. Known values stay the daemon's words.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Live => "live",
            Self::Paused => "paused",
            Self::Error => "error",
            Self::Other => "other",
        }
    }
}

/// Per-torrent speeds inside `stats.live`. Mbps, as the daemon reports them.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct LiveSpeeds {
    #[serde(default)]
    pub download_speed: Speed,
    #[serde(default)]
    pub upload_speed: Speed,
}

/// The `stats` object on a torrent row. Absent when the list was fetched
/// without `with_stats`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct TorrentStats {
    #[serde(default)]
    pub state: Option<TorrentState>,
    #[serde(default)]
    pub progress_bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub live: Option<LiveSpeeds>,
}

/// One row of `GET /torrents`. Extra daemon fields are ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Torrent {
    pub id: Option<u64>,
    pub info_hash: String,
    pub name: Option<String>,
    pub output_folder: String,
    #[serde(default)]
    pub total_pieces: u32,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Announce URLs. Used only to build the tracker filter.
    #[serde(default)]
    pub trackers: Option<Vec<String>>,
    #[serde(default)]
    pub stats: Option<TorrentStats>,
    /// Present on `GET /torrents/{id}`. Absent on the list.
    #[serde(default)]
    pub files: Option<Vec<TorrentFile>>,
}

/// One file inside `GET /torrents/{id}`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TorrentFile {
    pub name: String,
    #[serde(default)]
    pub components: Vec<String>,
    pub length: u64,
    /// Whether the file is in the download selection.
    pub included: bool,
}

/// Announce state of one tracker. The strings are what the daemon sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerState {
    NotContacted,
    Updating,
    Working,
    Error,
    Disabled,
    #[serde(other)]
    Other,
}

impl TrackerState {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NotContacted => "not contacted",
            Self::Updating => "updating",
            Self::Working => "working",
            Self::Error => "error",
            Self::Disabled => "disabled",
            Self::Other => "other",
        }
    }
}

/// One row of `GET /torrents/{id}/trackers`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TrackerStatus {
    pub url: String,
    pub state: TrackerState,
    #[serde(default)]
    pub seeders: Option<u32>,
    #[serde(default)]
    pub leechers: Option<u32>,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// `GET /torrents/{id}/trackers`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TrackerList {
    pub trackers: Vec<TrackerStatus>,
}

/// One peer inside `GET /torrents/{id}/peer_stats`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PeerView {
    pub addr: String,
    pub state: String,
    #[serde(default)]
    pub fetched_bytes: u64,
    #[serde(default)]
    pub uploaded_bytes: u64,
}

/// `GET /torrents/{id}/peer_stats`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct PeerList {
    #[serde(default)]
    pub peers: Vec<PeerView>,
}

impl PeerList {
    /// The daemon returns a map of address to counters. Flatten it.
    pub fn from_value(value: &serde_json::Value) -> Self {
        let Some(map) = value.get("peers").and_then(|p| p.as_object()) else {
            return Self::default();
        };
        let mut peers = Vec::with_capacity(map.len());
        for (addr, peer) in map {
            peers.push(PeerView {
                addr: addr.clone(),
                state: peer
                    .get("state")
                    .and_then(|s| s.as_str())
                    .unwrap_or("other")
                    .to_owned(),
                fetched_bytes: peer
                    .pointer("/counters/fetched_bytes")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0),
                uploaded_bytes: peer
                    .pointer("/counters/uploaded_bytes")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0),
            });
        }
        peers.sort_by(|a, b| a.addr.cmp(&b.addr));
        Self { peers }
    }
}

/// Pieces the daemon has. `have` is true when the piece is complete.
#[derive(Debug, Clone, PartialEq)]
pub struct PieceProgress {
    pub have: Vec<bool>,
}

impl PieceProgress {
    /// Decode the binary bitfield from `GET /torrents/{id}/haves`.
    /// `len` is the `x-bitfield-len` header. Bits past `len` are ignored.
    #[must_use]
    pub fn from_bytes(bytes: &[u8], len: u32) -> Self {
        let mut have = Vec::with_capacity(len as usize);
        for index in 0..len {
            let byte = bytes.get((index / 8) as usize).copied().unwrap_or(0);
            let bit = 7 - (index % 8);
            have.push((byte >> bit) & 1 == 1);
        }
        Self { have }
    }

    #[must_use]
    pub fn have_count(&self) -> u32 {
        self.have.iter().filter(|bit| **bit).count() as u32
    }
}

/// `GET /torrents`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TorrentList {
    pub torrents: Vec<Torrent>,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Speed {
    pub mbps: f64,
    #[serde(default)]
    pub human_readable: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionCounters {
    pub fetched_bytes: u64,
    pub uploaded_bytes: u64,
    #[serde(default)]
    pub blocked_incoming: u64,
    #[serde(default)]
    pub blocked_outgoing: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PeerCounts {
    #[serde(default)]
    pub queued: u32,
    #[serde(default)]
    pub connecting: u32,
    #[serde(default)]
    pub live: u32,
    #[serde(default)]
    pub seen: u32,
    #[serde(default)]
    pub dead: u32,
    #[serde(default)]
    pub not_needed: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct ConnectionFamily {
    #[serde(default)]
    pub attempts: u64,
    #[serde(default)]
    pub successes: u64,
    #[serde(default)]
    pub errors: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct ConnectionKindStats {
    #[serde(default)]
    pub v4: ConnectionFamily,
    #[serde(default)]
    pub v6: ConnectionFamily,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Connections {
    #[serde(default)]
    pub tcp: ConnectionKindStats,
    #[serde(default)]
    pub utp: ConnectionKindStats,
    #[serde(default)]
    pub socks: ConnectionKindStats,
}

/// `GET /stats`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionStats {
    pub counters: SessionCounters,
    pub peers: PeerCounts,
    #[serde(default)]
    pub connections: Connections,
    pub download_speed: Speed,
    pub upload_speed: Speed,
    pub uptime_seconds: u64,
}

/// `GET /session/downloads`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct DownloadsConfig {
    pub incomplete_file_suffix: bool,
}

/// `GET /session/advanced`. Seconds; null is the engine default.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct AdvancedConfig {
    pub connect_timeout_secs: Option<u64>,
    pub read_write_timeout_secs: Option<u64>,
    pub keep_alive_secs: Option<u64>,
}

/// `GET /session/network`. Writes are refused by the daemon: these are fixed at startup.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct NetworkConfig {
    pub listen_ip: Option<String>,
    pub listen_port: Option<u16>,
    pub dht_enabled: bool,
    pub pex_enabled: bool,
    pub lsd_enabled: bool,
    pub upnp_port_forward: bool,
    pub encryption: Option<String>,
    #[serde(default)]
    pub http_api_listen: String,
}

/// `GET /session/proxy`. The password, if any, is already masked by the daemon.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct ProxyConfig {
    pub proxy_url: Option<String>,
}

/// `GET /session/ip-filter`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct IpFilterConfig {
    pub filter: String,
}

/// `GET /session/folders`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct FoldersConfig {
    pub download_folder: String,
    pub completed_folder: Option<String>,
}

/// `GET /torrents/limits`. `None` is unlimited.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct RateLimits {
    pub upload_bps: Option<u32>,
    pub download_bps: Option<u32>,
    pub peer_limit: Option<u64>,
    pub concurrent_init_limit: Option<u64>,
}

/// `GET /speed/alt`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct AltSpeedStatus {
    pub enabled: bool,
    pub config: AltSpeedConfig,
    #[serde(default)]
    pub schedule: Option<AltSpeedSchedule>,
}

/// Alternative rates in bytes per second. `None` is unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Default)]
pub struct AltSpeedConfig {
    pub download_rate: Option<u64>,
    pub upload_rate: Option<u64>,
}

/// `GET /speed/schedule`. Minutes from local midnight; days is a bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Default)]
pub struct AltSpeedSchedule {
    pub enabled: bool,
    #[serde(default)]
    pub start_minutes: u32,
    #[serde(default)]
    pub end_minutes: u32,
    #[serde(default)]
    pub days: u8,
}

/// `GET /torrents/seed_limits`. `None` disables that limit.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Default)]
pub struct SeedLimits {
    pub ratio_limit: Option<f64>,
    pub time_limit_secs: Option<u64>,
}

/// Everything the five settings tabs read, in one round trip.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SettingsSnapshot {
    pub downloads: DownloadsConfig,
    pub advanced: AdvancedConfig,
    pub network: NetworkConfig,
    pub proxy: ProxyConfig,
    pub ip_filter: IpFilterConfig,
    pub folders: FoldersConfig,
    pub limits: RateLimits,
    pub alt: AltSpeedStatus,
    pub schedule: AltSpeedSchedule,
    pub seed_limits: SeedLimits,
}

/// `GET /rss/feeds`. Unknown fields are ignored.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct RssFeed {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub enabled: bool,
}

/// `GET /rss/items`. A missing download URL cannot be fetched.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct RssItem {
    pub id: String,
    #[serde(default)]
    pub feed_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub downloaded: bool,
}

/// Feeds and items the RSS screen shows. Rules are not part of v1.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RssSnapshot {
    pub feeds: Vec<RssFeed>,
    pub items: Vec<RssItem>,
}

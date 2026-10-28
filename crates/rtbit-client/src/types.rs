use serde::{Deserialize, Serialize};

/// What a caller may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    ReadOnly,
}

/// The token pair `POST /auth/login` and `POST /auth/refresh` return.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: u64,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LoginBody<'a> {
    pub username: &'a str,
    pub password: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RefreshBody<'a> {
    pub refresh_token: &'a str,
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

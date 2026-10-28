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

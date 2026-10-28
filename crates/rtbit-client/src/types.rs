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
}

/// `GET /torrents`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TorrentList {
    pub torrents: Vec<Torrent>,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
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

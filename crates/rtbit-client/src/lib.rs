//! HTTP client for the rustTorrent daemon API.
//!
//! This crate talks only to the daemon's own routes (`/auth`, `/torrents`,
//! `/stats`). It does not call the WebUI API v2 compatibility paths, and it
//! does not depend on the daemon library or a UI toolkit.

mod client;
mod error;
mod types;

pub use client::Client;
pub use error::Error;
pub use http::StatusCode;
pub use types::{
    AdvancedConfig, AdvancedSaved, AltSpeedConfig, AltSpeedSchedule, AltSpeedStatus, AuthStatus,
    ConnectionFamily, ConnectionKindStats, Connections, DownloadsConfig, FoldersConfig,
    IpFilterConfig, LiveSpeeds, NetworkConfig, NetworkSettings, PeerCounts, PeerList, PeerView,
    PieceProgress, ProxyConfig, RateLimits, Role, RssFeed, RssItem, RssRule, RssRuleBody,
    RssRuleEpisode, RssSnapshot, SavedNetworkStatus, SeedLimits, SessionCounters, SessionStats,
    SettingsSnapshot, Speed, TokenPair, Torrent, TorrentFile, TorrentList, TorrentState,
    TorrentStats, TrackerList, TrackerState, TrackerStatus,
};

/// Caps for `AdvancedConfig`, matching the daemon's limits.
pub use types::{
    CONCURRENT_INIT_LIMIT_MAX, CONCURRENT_INIT_LIMIT_MIN, CONNECT_TIMEOUT_SECS_MAX,
    CONNECT_TIMEOUT_SECS_MIN, FASTRESUME_VALIDATION_DENOM_MAX, FASTRESUME_VALIDATION_DENOM_MIN,
    KEEP_ALIVE_SECS_MAX, KEEP_ALIVE_SECS_MIN, MAGNET_RESOLUTION_TIMEOUT_SECS_MAX,
    MAGNET_RESOLUTION_TIMEOUT_SECS_MIN, MAX_CONCURRENT_HANDSHAKES_MAX,
    MAX_CONCURRENT_HANDSHAKES_MIN, PEER_LIMIT_MAX, PEER_LIMIT_MIN, READ_WRITE_TIMEOUT_SECS_MAX,
    READ_WRITE_TIMEOUT_SECS_MIN, UPLOAD_SLOTS_MAX, UPLOAD_SLOTS_MIN,
};

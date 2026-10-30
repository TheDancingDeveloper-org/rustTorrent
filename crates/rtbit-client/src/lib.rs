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
    AdvancedConfig, AltSpeedConfig, AltSpeedSchedule, AltSpeedStatus, AuthStatus, ConnectionFamily,
    ConnectionKindStats, Connections, DownloadsConfig, FoldersConfig, IpFilterConfig, LiveSpeeds,
    NetworkConfig, NetworkSettings, PeerCounts, PeerList, PeerView, PieceProgress, ProxyConfig,
    RateLimits, Role, RssFeed, RssItem, RssSnapshot, SavedNetworkStatus, SeedLimits,
    SessionCounters, SessionStats, SettingsSnapshot, Speed, TokenPair, Torrent, TorrentFile,
    TorrentList, TorrentState, TorrentStats, TrackerList, TrackerState, TrackerStatus,
};

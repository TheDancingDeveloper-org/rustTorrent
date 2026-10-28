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
pub use types::{
    ConnectionFamily, ConnectionKindStats, Connections, LiveSpeeds, PeerCounts, Role,
    SessionCounters, SessionStats, Speed, TokenPair, Torrent, TorrentList, TorrentState,
    TorrentStats,
};

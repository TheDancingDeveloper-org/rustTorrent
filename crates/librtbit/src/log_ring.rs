//! Bounded in-memory history of log messages and peer-filter events.
//!
//! The tracing subscriber feeds the process-wide [`global`] ring so the HTTP
//! API can serve recent history (WebUI API v2 `log/main` and `log/peers`)
//! instead of only a live stream. Ids increase monotonically for the life of
//! the process, so a client polls with the last id it has seen.

use std::{
    collections::VecDeque,
    net::IpAddr,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde::Serialize;

/// Entries kept per ring; the oldest are dropped first.
pub const DEFAULT_CAPACITY: usize = 1000;

/// Message severity, using the WebUI API v2 bit values so a client's type
/// filter maps straight onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(into = "u8")]
pub enum LogSeverity {
    Normal = 1,
    Info = 2,
    Warning = 4,
    Critical = 8,
}

impl From<LogSeverity> for u8 {
    fn from(value: LogSeverity) -> Self {
        value as u8
    }
}

impl From<tracing::Level> for LogSeverity {
    fn from(level: tracing::Level) -> Self {
        match level {
            tracing::Level::ERROR => LogSeverity::Critical,
            tracing::Level::WARN => LogSeverity::Warning,
            tracing::Level::INFO => LogSeverity::Normal,
            _ => LogSeverity::Info,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub message: String,
    /// Milliseconds since the Unix epoch.
    pub timestamp: u64,
    #[serde(rename = "type")]
    pub severity: LogSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeerLogEntry {
    pub id: u64,
    pub ip: String,
    /// Milliseconds since the Unix epoch.
    pub timestamp: u64,
    pub blocked: bool,
    pub reason: String,
}

struct Ring<T> {
    entries: VecDeque<T>,
    next_id: u64,
    capacity: usize,
}

impl<T> Ring<T> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 0,
            capacity: capacity.max(1),
        }
    }

    fn push(&mut self, make: impl FnOnce(u64) -> T) {
        let id = self.next_id;
        self.next_id += 1;
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(make(id));
    }
}

pub struct LogRing {
    main: Mutex<Ring<LogEntry>>,
    peers: Mutex<Ring<PeerLogEntry>>,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            main: Mutex::new(Ring::new(capacity)),
            peers: Mutex::new(Ring::new(capacity)),
        }
    }

    pub fn push(&self, severity: LogSeverity, message: String) {
        let timestamp = now_millis();
        self.main.lock().push(|id| LogEntry {
            id,
            message,
            timestamp,
            severity,
        });
    }

    pub fn push_peer(&self, ip: IpAddr, blocked: bool, reason: impl Into<String>) {
        let timestamp = now_millis();
        let reason = reason.into();
        self.peers.lock().push(|id| PeerLogEntry {
            id,
            ip: ip.to_string(),
            timestamp,
            blocked,
            reason,
        });
    }

    /// Main-log entries newer than `after` (all when `None`) whose severity
    /// passes `include`.
    pub fn main_since(
        &self,
        after: Option<u64>,
        include: impl Fn(LogSeverity) -> bool,
    ) -> Vec<LogEntry> {
        self.main
            .lock()
            .entries
            .iter()
            .filter(|e| after.is_none_or(|after| e.id > after))
            .filter(|e| include(e.severity))
            .cloned()
            .collect()
    }

    /// Peer-log entries newer than `after` (all when `None`).
    pub fn peers_since(&self, after: Option<u64>) -> Vec<PeerLogEntry> {
        self.peers
            .lock()
            .entries
            .iter()
            .filter(|e| after.is_none_or(|after| e.id > after))
            .cloned()
            .collect()
    }
}

/// The process-wide ring fed by the tracing subscriber.
pub fn global() -> &'static LogRing {
    static RING: OnceLock<LogRing> = OnceLock::new();
    RING.get_or_init(|| LogRing::new(DEFAULT_CAPACITY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_increase_and_oldest_entries_are_dropped() {
        let ring = LogRing::new(3);
        for i in 0..5 {
            ring.push(LogSeverity::Normal, format!("m{i}"));
        }
        let all = ring.main_since(None, |_| true);
        let ids: Vec<u64> = all.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![2, 3, 4]);
        assert_eq!(all[0].message, "m2");
        assert_eq!(ring.main_since(Some(3), |_| true).len(), 1);
        assert!(ring.main_since(Some(4), |_| true).is_empty());
    }

    #[test]
    fn severity_filter_and_mapping() {
        let ring = LogRing::new(10);
        ring.push(tracing::Level::ERROR.into(), "e".into());
        ring.push(tracing::Level::WARN.into(), "w".into());
        ring.push(tracing::Level::INFO.into(), "i".into());
        ring.push(tracing::Level::DEBUG.into(), "d".into());
        let warn_only = ring.main_since(None, |s| s == LogSeverity::Warning);
        assert_eq!(warn_only.len(), 1);
        assert_eq!(warn_only[0].message, "w");
        let json = serde_json::to_value(ring.main_since(None, |_| true)).unwrap();
        let types: Vec<u64> = json
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["type"].as_u64().unwrap())
            .collect();
        assert_eq!(types, vec![8, 4, 1, 2]);
    }

    #[test]
    fn peer_ring_is_separate() {
        let ring = LogRing::new(10);
        ring.push(LogSeverity::Normal, "x".into());
        ring.push_peer("10.0.0.1".parse().unwrap(), true, "banned");
        let peers = ring.peers_since(None);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, 0);
        assert_eq!(peers[0].ip, "10.0.0.1");
        assert!(peers[0].blocked);
    }
}

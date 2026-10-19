use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde_derive::Serialize;
use tokio::sync::watch;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Announce state of a single tracker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerAnnounceState {
    /// Tracker configured but no announce attempted yet.
    NotContacted,
    /// An announce is in flight (or being retried).
    Updating,
    /// Last announce succeeded.
    Working,
    /// Last announce failed.
    Error,
    /// Tracker skipped (unsupported scheme, proxy mode, ...).
    Disabled,
}

/// Live status of a single tracker for one torrent.
#[derive(Clone, Debug, Serialize)]
pub struct TrackerStatus {
    pub url: String,
    pub state: TrackerAnnounceState,
    /// Seeder count ("complete") reported by the last successful announce.
    pub seeders: Option<u32>,
    /// Leecher count ("incomplete") reported by the last successful announce.
    pub leechers: Option<u32>,
    /// Number of peer addresses returned by the last successful announce.
    pub peers_returned: Option<u32>,
    /// Unix timestamp of the last successful announce.
    pub last_announce_unix: Option<u64>,
    /// Announce interval (seconds) the tracker asked for.
    pub interval_secs: Option<u64>,
    pub last_error: Option<String>,
}

impl TrackerStatus {
    fn new(url: String) -> Self {
        Self {
            url,
            state: TrackerAnnounceState::NotContacted,
            seeders: None,
            leechers: None,
            peers_returned: None,
            last_announce_unix: None,
            interval_secs: None,
            last_error: None,
        }
    }
}

/// Per-torrent registry of tracker announce statuses, updated by the
/// announce loops and readable from the HTTP API.
///
/// The registry is also the control channel for live announce loops: each
/// entry carries a generation, and a loop only keeps announcing while the
/// generation it started with is still current. Removing a tracker drops its
/// entry, so its loop stops; [`Self::request_reannounce`] wakes every sleeping
/// loop for an immediate announce.
pub struct TrackerStatusRegistry {
    inner: RwLock<HashMap<String, Entry>>,
    next_generation: AtomicU64,
    kick: watch::Sender<u64>,
}

struct Entry {
    status: TrackerStatus,
    generation: u64,
}

impl Default for TrackerStatusRegistry {
    fn default() -> Self {
        Self {
            inner: Default::default(),
            next_generation: AtomicU64::new(1),
            kick: watch::Sender::new(0),
        }
    }
}

impl TrackerStatusRegistry {
    fn insert_or_get<'a>(&self, g: &'a mut HashMap<String, Entry>, url: &str) -> &'a mut Entry {
        g.entry(url.to_owned()).or_insert_with(|| Entry {
            status: TrackerStatus::new(url.to_owned()),
            generation: self.next_generation.fetch_add(1, Ordering::Relaxed),
        })
    }

    /// Update an existing entry. Updates for trackers that were removed are
    /// dropped so a stopping announce loop can't resurrect them.
    fn with_existing(&self, url: &str, f: impl FnOnce(&mut TrackerStatus)) {
        if let Some(e) = self.inner.write().get_mut(url) {
            f(&mut e.status);
        }
    }

    /// Register a tracker without changing its state. Returns the entry's
    /// generation, which an announce loop passes to [`Self::is_current`].
    pub fn ensure(&self, url: &str) -> u64 {
        let mut g = self.inner.write();
        self.insert_or_get(&mut g, url).generation
    }

    /// Whether a loop started at `generation` for `url` should keep announcing.
    pub fn is_current(&self, url: &str, generation: u64) -> bool {
        self.inner
            .read()
            .get(url)
            .is_some_and(|e| e.generation == generation)
    }

    /// Forget a tracker. Its announce loops stop at their next wake-up, which
    /// this triggers immediately. Returns whether the tracker was known.
    pub fn remove(&self, url: &str) -> bool {
        let removed = self.inner.write().remove(url).is_some();
        if removed {
            self.kick();
        }
        removed
    }

    /// Wake every announce loop so it announces now instead of waiting out
    /// its interval.
    pub fn request_reannounce(&self) {
        self.kick();
    }

    /// Receiver that changes on every reannounce request or removal.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.kick.subscribe()
    }

    fn kick(&self) {
        self.kick.send_modify(|v| *v = v.wrapping_add(1));
    }

    pub fn record_updating(&self, url: &str) {
        self.with_existing(url, |e| {
            if e.state == TrackerAnnounceState::NotContacted {
                e.state = TrackerAnnounceState::Updating;
            }
        });
    }

    pub fn record_success(
        &self,
        url: &str,
        seeders: Option<u32>,
        leechers: Option<u32>,
        peers_returned: u32,
        interval_secs: u64,
    ) {
        self.with_existing(url, |e| {
            e.state = TrackerAnnounceState::Working;
            e.seeders = seeders.or(e.seeders);
            e.leechers = leechers.or(e.leechers);
            e.peers_returned = Some(peers_returned);
            e.last_announce_unix = Some(now_unix());
            e.interval_secs = Some(interval_secs);
            e.last_error = None;
        });
    }

    pub fn record_error(&self, url: &str, error: &str) {
        self.with_existing(url, |e| {
            e.state = TrackerAnnounceState::Error;
            e.last_error = Some(error.to_owned());
        });
    }

    pub fn record_disabled(&self, url: &str, reason: &str) {
        let mut g = self.inner.write();
        let e = &mut self.insert_or_get(&mut g, url).status;
        e.state = TrackerAnnounceState::Disabled;
        e.last_error = Some(reason.to_owned());
    }

    /// Sorted snapshot of all known trackers.
    pub fn snapshot(&self) -> Vec<TrackerStatus> {
        let mut v: Vec<TrackerStatus> = self
            .inner
            .read()
            .values()
            .map(|e| e.status.clone())
            .collect();
        v.sort_by(|a, b| a.url.cmp(&b.url));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://tracker.example/announce";

    #[test]
    fn remove_invalidates_generation_and_drops_late_updates() {
        let r = TrackerStatusRegistry::default();
        let generation = r.ensure(URL);
        assert_eq!(r.ensure(URL), generation);
        assert!(r.is_current(URL, generation));

        assert!(r.remove(URL));
        assert!(!r.is_current(URL, generation));
        assert!(!r.remove(URL));

        // A stopping loop's late report must not resurrect the entry.
        r.record_success(URL, Some(1), Some(1), 1, 60);
        r.record_error(URL, "boom");
        assert!(r.snapshot().is_empty());

        // Re-adding gives a new generation, so the old loop stays stopped.
        let readded = r.ensure(URL);
        assert_ne!(readded, generation);
        assert!(!r.is_current(URL, generation));
        assert!(r.is_current(URL, readded));
    }

    #[test]
    fn reannounce_and_remove_wake_subscribers() {
        let r = TrackerStatusRegistry::default();
        r.ensure(URL);
        let mut rx = r.subscribe();
        assert!(!rx.has_changed().unwrap());
        r.request_reannounce();
        assert!(rx.has_changed().unwrap());
        rx.mark_unchanged();
        r.remove(URL);
        assert!(rx.has_changed().unwrap());
    }
}

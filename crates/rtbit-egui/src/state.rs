//! Torrent list cache and the sidebar filters.

use rtbit_client::{SessionStats, Torrent, TorrentState};

/// Sidebar status filter. `All` shows every row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    All,
    Live,
    Paused,
    Error,
    Initializing,
}

impl StatusFilter {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Live,
        Self::Paused,
        Self::Error,
        Self::Initializing,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Live => "Live",
            Self::Paused => "Paused",
            Self::Error => "Error",
            Self::Initializing => "Initializing",
        }
    }

    fn matches(self, torrent: &Torrent) -> bool {
        let state = torrent.stats.as_ref().and_then(|stats| stats.state);
        match self {
            Self::All => true,
            Self::Live => state == Some(TorrentState::Live),
            Self::Paused => state == Some(TorrentState::Paused),
            Self::Error => state == Some(TorrentState::Error),
            Self::Initializing => state == Some(TorrentState::Initializing),
        }
    }
}

/// Sentinel for "every category" and "every tracker".
pub const ANY: &str = "All";

/// What the main screen shows, filled by the poll.
#[derive(Clone, Debug, Default)]
pub struct ListCache {
    pub torrents: Vec<Torrent>,
    pub stats: Option<SessionStats>,
    /// Set when the last poll failed. The previous list stays on screen.
    pub error: Option<String>,
}

impl ListCache {
    /// Rows that pass the status, category and tracker filters, in daemon order.
    #[must_use]
    pub fn filtered<'a>(
        &'a self,
        status: StatusFilter,
        category: &str,
        tracker: &str,
    ) -> Vec<&'a Torrent> {
        self.torrents
            .iter()
            .filter(|torrent| status.matches(torrent))
            .filter(|torrent| category == ANY || torrent.category.as_deref() == Some(category))
            .filter(|torrent| tracker == ANY || has_tracker(torrent, tracker))
            .collect()
    }

    /// Category names present in the list, sorted, with `All` first.
    #[must_use]
    pub fn categories(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .torrents
            .iter()
            .filter_map(|torrent| torrent.category.clone())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        names.dedup();
        names.insert(0, ANY.to_owned());
        names
    }

    /// Tracker hosts present in the list, sorted, with `All` first.
    #[must_use]
    pub fn trackers(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self
            .torrents
            .iter()
            .flat_map(|torrent| torrent.trackers.iter().flatten())
            .filter_map(|url| tracker_host(url))
            .collect();
        hosts.sort();
        hosts.dedup();
        hosts.insert(0, ANY.to_owned());
        hosts
    }
}

fn has_tracker(torrent: &Torrent, host: &str) -> bool {
    torrent
        .trackers
        .iter()
        .flatten()
        .any(|url| tracker_host(url).as_deref() == Some(host))
}

/// `http://host:port/announce` -> `host`. A URL without a host is skipped.
fn tracker_host(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', ':']).next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(host.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtbit_client::TorrentStats;

    fn row(name: &str, state: Option<TorrentState>, category: Option<&str>) -> Torrent {
        Torrent {
            id: Some(1),
            info_hash: "h".into(),
            name: Some(name.into()),
            output_folder: "/d".into(),
            total_pieces: 1,
            category: category.map(str::to_owned),
            tags: None,
            trackers: Some(vec!["http://tracker.example:80/announce".into()]),
            files: None,
            stats: state.map(|state| TorrentStats {
                state: Some(state),
                ..TorrentStats::default()
            }),
        }
    }

    #[test]
    fn status_and_category_filter() {
        let cache = ListCache {
            torrents: vec![
                row("a", Some(TorrentState::Live), Some("movies")),
                row("b", Some(TorrentState::Paused), Some("music")),
            ],
            ..ListCache::default()
        };
        let live = cache.filtered(StatusFilter::Live, ANY, ANY);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name.as_deref(), Some("a"));
        let movies = cache.filtered(StatusFilter::All, "movies", "tracker.example");
        assert_eq!(movies.len(), 1);
        assert!(cache.filtered(StatusFilter::All, ANY, "other").is_empty());
    }
}

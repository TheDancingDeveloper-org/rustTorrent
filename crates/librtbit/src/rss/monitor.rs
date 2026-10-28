use std::{path::PathBuf, sync::Arc};

use chrono::Utc;
use parking_lot::Mutex;
use tracing::{info, warn};

use super::config::RssFeedConfig;
use super::db::RssDatabase;
use super::fetch::{self, MAX_FEED_BYTES};
use super::models::RssItem;
use super::rules::{self, RuleDecision};
use crate::api::Api;
use crate::session::AddTorrent;

/// Background RSS feed monitor that polls configured feeds for torrent links,
/// persists all discovered items to the database, and automatically enqueues
/// items that match download rules.
pub struct RssMonitor {
    db: Arc<Mutex<RssDatabase>>,
    api: Api,
    rss_history_limit: Option<usize>,
}

impl RssMonitor {
    pub fn new(db: Arc<Mutex<RssDatabase>>, api: Api, rss_history_limit: Option<usize>) -> Self {
        Self {
            db,
            api,
            rss_history_limit,
        }
    }

    /// Run the monitor loop forever, polling feeds at their configured intervals.
    pub async fn run(self) {
        info!("Starting RSS monitor");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        loop {
            // Load feeds from the database each iteration so API changes are picked up
            let feeds = {
                let db = self.db.lock();
                db.feed_list().unwrap_or_default()
            };

            if feeds.is_empty() {
                // No feeds configured, sleep and check again
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                continue;
            }

            for feed in &feeds {
                if !feed.enabled {
                    continue;
                }

                if let Err(e) = self.check_feed(&client, feed).await {
                    warn!(feed = %feed.name, error = %e, "RSS feed check failed");
                }
            }

            // Prune old items based on config
            self.prune_items();

            // Use the minimum poll interval across all enabled feeds, defaulting to 15 min
            let interval = feeds
                .iter()
                .filter(|f| f.enabled)
                .map(|f| f.poll_interval_secs)
                .min()
                .unwrap_or(900);

            tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;
        }
    }

    fn prune_items(&self) {
        let limit = self.rss_history_limit.unwrap_or(500);
        let db = self.db.lock();
        if let Ok(count) = db.rss_item_count()
            && count > limit
            && let Ok(pruned) = db.rss_items_prune(limit)
            && pruned > 0
        {
            info!(pruned, "Pruned old RSS items");
        }
    }

    async fn read_feed_body(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
        // file:// is only a test seam. A feed URL has already been refused
        // unless it is http(s), so this branch never runs for a stored feed.
        if url.starts_with("file://") {
            let path = file_url_to_path(url)?;
            let bytes = tokio::fs::read(&path).await.map_err(|e| {
                anyhow::anyhow!("error reading RSS feed file {}: {e}", path.display())
            })?;
            return Ok(fetch::cap_feed_body(&bytes));
        }

        let parsed = reqwest::Url::parse(url)?;
        if let Some(host) = parsed.host_str()
            && host.parse::<std::net::IpAddr>().is_err()
        {
            let addresses = tokio::net::lookup_host((host, parsed.port().unwrap_or(80)))
                .await
                .map_err(|e| anyhow::anyhow!("could not resolve feed host {host}: {e}"))?
                .map(|addr| addr.ip())
                .collect::<Vec<_>>();
            fetch::assert_public_addresses(&addresses).map_err(|e| anyhow::anyhow!("{e}"))?;
        }

        let response = client.get(url).send().await?;
        let body = response.bytes().await?;
        Ok(fetch::cap_feed_body(&body))
    }

    async fn check_feed(
        &self,
        client: &reqwest::Client,
        feed: &RssFeedConfig,
    ) -> anyhow::Result<()> {
        info!(feed = %feed.name, url = %feed.url, "Checking RSS feed");

        fetch::check_feed_url(&feed.url).map_err(|e| anyhow::anyhow!("{e}"))?;
        let body = Self::read_feed_body(client, &feed.url).await?;
        if body.len() > MAX_FEED_BYTES {
            warn!(feed = %feed.name, bytes = body.len(), "RSS feed body exceeds the size cap");
        }
        let body = fetch::cap_feed_body(&body);
        let parsed = feed_rs::parser::parse(&body[..])?;

        // Compile the feed filter if one is set. An oversized or invalid
        // pattern filters everything out rather than running unbounded.
        let filter = feed
            .filter_regex
            .as_ref()
            .and_then(|r| rules::BoundedPattern::compile(r).ok());

        // Load download rules for this feed
        let rules = {
            let db = self.db.lock();
            db.rss_rule_list()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.enabled && r.feed_names.iter().any(|n| n == &feed.name))
                .collect::<Vec<_>>()
        };

        let mut new_items = 0;

        for entry in &parsed.entries {
            let title = entry
                .title
                .as_ref()
                .map(|t| t.content.clone())
                .unwrap_or_default();
            let entry_id = entry.id.clone();

            // Find torrent URL from links or media content
            let torrent_url = Self::extract_torrent_url(entry);

            // Extract size from enclosure/media if available
            let size_bytes = entry
                .media
                .iter()
                .flat_map(|m| &m.content)
                .filter_map(|c| c.size)
                .next()
                .unwrap_or(0);

            // Extract published date
            let published_at = entry.published.or(entry.updated);

            // Persist ALL items to DB (regardless of filter)
            let item = RssItem {
                id: entry_id.clone(),
                feed_name: feed.name.clone(),
                title: title.clone(),
                url: torrent_url.clone(),
                published_at,
                first_seen_at: Utc::now(),
                downloaded: false,
                downloaded_at: None,
                category: feed.category.clone(),
                size_bytes,
            };

            // Check if already in DB (dedup)
            let already_exists = {
                let db = self.db.lock();
                db.rss_item_exists(&entry_id).unwrap_or(false)
            };

            if !already_exists {
                let db = self.db.lock();
                let _ = db.rss_item_upsert(&item);
                new_items += 1;
            } else {
                continue; // Already processed
            }

            let Some(ref url) = torrent_url else {
                continue;
            };

            // Check if this item should be auto-downloaded:
            // 1. Feed-level filter must pass (if set)
            let passes_filter = match filter {
                Some(ref re) => re.is_match(&title),
                None => true,
            };

            if !passes_filter {
                continue;
            }

            // 2. Check per-feed rules, skipping episodes already downloaded.
            let tracker = {
                let db = self.db.lock();
                db.episodes_load().unwrap_or_default()
            };
            let decision = rules::decide(&rules, &title, &tracker);

            let (should_download, category) = match decision {
                RuleDecision::Download { category } => {
                    (true, category.or_else(|| feed.category.clone()))
                }
                RuleDecision::Skip if feed.auto_download && feed.filter_regex.is_none() => {
                    (true, feed.category.clone())
                }
                RuleDecision::Skip => (false, None),
            };

            if !should_download {
                continue;
            }

            info!(feed = %feed.name, title = %title, url = %url, "Auto-downloading RSS item");

            match self.add_torrent(url, category.as_deref()).await {
                Ok(()) => {
                    let db = self.db.lock();
                    let _ = db.rss_item_mark_downloaded(&entry_id, category.as_deref());
                    if let Some(episode) = rules::parse_episode(&title) {
                        let _ = db.episode_record(&rules::series_key(&title), episode);
                    }
                    info!(title = %title, "RSS torrent added successfully");
                }
                Err(e) => {
                    warn!(title = %title, error = %e, "Failed to add RSS torrent");
                }
            }
        }

        if new_items > 0 {
            info!(feed = %feed.name, new_items, "RSS feed check complete");
        }

        Ok(())
    }

    /// Extract torrent URL from a feed entry's links or media content.
    /// Looks for .torrent files, magnet links, or application/x-bittorrent media types.
    fn extract_torrent_url(entry: &feed_rs::model::Entry) -> Option<String> {
        // Check for magnet links first
        entry
            .links
            .iter()
            .find(|l| l.href.starts_with("magnet:"))
            .map(|l| l.href.clone())
            .or_else(|| {
                // Check for .torrent file links
                entry
                    .links
                    .iter()
                    .find(|l| {
                        l.href.ends_with(".torrent")
                            || l.media_type
                                .as_deref()
                                .is_some_and(|mt| mt == "application/x-bittorrent")
                    })
                    .map(|l| l.href.clone())
            })
            .or_else(|| {
                // Check media content for torrent URLs
                entry
                    .media
                    .iter()
                    .flat_map(|m| &m.content)
                    .find(|c| {
                        c.url.as_ref().is_some_and(|u| {
                            let s = u.as_str();
                            s.ends_with(".torrent") || s.starts_with("magnet:")
                        })
                    })
                    .and_then(|c| c.url.as_ref().map(|u| u.to_string()))
            })
            .or_else(|| {
                // Check enclosure-style links (RSS 2.0 enclosures often appear as links)
                entry
                    .links
                    .iter()
                    .find(|l| {
                        l.media_type
                            .as_deref()
                            .is_some_and(|mt| mt.contains("torrent") || mt.contains("magnet"))
                    })
                    .map(|l| l.href.clone())
            })
            .or_else(|| {
                // Fall back to first link as last resort
                entry.links.first().map(|l| l.href.clone())
            })
    }

    /// Add a torrent to the session from a URL (magnet or .torrent URL).
    async fn add_torrent(&self, url: &str, category: Option<&str>) -> anyhow::Result<()> {
        use std::borrow::Cow;

        let opts = crate::session::AddTorrentOptions {
            overwrite: true,
            category: category.map(|s| s.to_string()),
            ..Default::default()
        };

        let add = AddTorrent::Url(Cow::Owned(url.to_string()));

        self.api
            .session()
            .add_torrent(add, Some(opts))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to add torrent: {e:#}"))?;

        Ok(())
    }
}

fn file_url_to_path(url: &str) -> anyhow::Result<PathBuf> {
    let parsed =
        url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid file URL {url:?}: {e}"))?;
    if parsed.scheme() != "file" {
        anyhow::bail!("URL is not a file URL: {url:?}");
    }
    parsed
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("file URL cannot be converted to a local path: {url:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_feed_body_supports_file_urls() {
        let dir = tempfile::tempdir().unwrap();
        let feed_path = dir.path().join("feed.xml");
        tokio::fs::write(&feed_path, b"<rss version=\"2.0\"></rss>")
            .await
            .unwrap();
        let url = url::Url::from_file_path(&feed_path).unwrap().to_string();
        let client = reqwest::Client::new();

        let body = RssMonitor::read_feed_body(&client, &url).await.unwrap();

        assert_eq!(body, b"<rss version=\"2.0\"></rss>");
    }
}

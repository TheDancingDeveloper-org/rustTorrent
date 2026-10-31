use std::{path::PathBuf, sync::Arc};

use chrono::Utc;
use parking_lot::Mutex;
use tracing::{info, warn};

use super::config::RssFeedConfig;
use super::db::RssDatabase;
use super::fetch;
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

        let client = pinned_client().expect("Failed to create HTTP client");

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
        let response = send_following_checked_redirects(client, parsed).await?;
        read_capped_body(response).await
    }

    async fn check_feed(
        &self,
        client: &reqwest::Client,
        feed: &RssFeedConfig,
    ) -> anyhow::Result<()> {
        info!(feed = %feed.name, url = %feed.url, "Checking RSS feed");

        fetch::check_feed_url(&feed.url).map_err(|e| anyhow::anyhow!("{e}"))?;
        let body = Self::read_feed_body(client, &feed.url).await?;
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

            // The rule that matched, and whether it tracks episodes. A feed-level
            // auto-download keeps the old behaviour: it always tracks.
            let (should_download, category, matched_rule) = match decision {
                RuleDecision::Download {
                    category,
                    rule_id,
                    track_episodes,
                } => (
                    true,
                    category.or_else(|| feed.category.clone()),
                    Some((rule_id, track_episodes)),
                ),
                RuleDecision::Skip if feed.auto_download && feed.filter_regex.is_none() => {
                    (true, feed.category.clone(), None)
                }
                RuleDecision::Skip => (false, None, None),
            };

            if !should_download {
                continue;
            }

            info!(feed = %feed.name, title = %title, url = %url, "Auto-downloading RSS item");

            match self.add_torrent(url, category.as_deref()).await {
                Ok(torrent_id) => {
                    let db = self.db.lock();
                    let _ = db.rss_item_mark_downloaded(&entry_id, category.as_deref());
                    if let Some(episode) = rules::parse_episode(&title) {
                        let series = rules::series_key(&title);
                        let track = matched_rule.as_ref().is_none_or(|(_, track)| *track);
                        if track {
                            let _ = db.episode_record(&series, episode);
                        }
                        if let Some((rule_id, _)) = &matched_rule {
                            let _ = db.rule_episode_record(
                                rule_id,
                                &super::models::RssRuleEpisode {
                                    show: series,
                                    season: episode.season,
                                    episode: episode.episode,
                                    title: title.clone(),
                                    matched_at: Utc::now(),
                                    torrent_id,
                                },
                            );
                        }
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
    /// Add the torrent; returns its id when the session reports one.
    async fn add_torrent(
        &self,
        url: &str,
        category: Option<&str>,
    ) -> anyhow::Result<Option<usize>> {
        use std::borrow::Cow;

        let opts = crate::session::AddTorrentOptions {
            overwrite: true,
            category: category.map(|s| s.to_string()),
            ..Default::default()
        };

        let add = AddTorrent::Url(Cow::Owned(url.to_string()));

        let response = self
            .api
            .session()
            .add_torrent(add, Some(opts))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to add torrent: {e:#}"))?;

        Ok(match response {
            crate::session::AddTorrentResponse::Added(id, _)
            | crate::session::AddTorrentResponse::AlreadyManaged(id, _) => Some(id),
            crate::session::AddTorrentResponse::ListOnly(_) => None,
        })
    }
}

const MAX_REDIRECTS: usize = 3;

/// Shared client. Redirects are not followed here: each hop is checked and
/// pinned by [`client_pinned_to_checked_address`] before the next request.
fn pinned_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

/// Resolve `url`, refuse any non-public address, and return a client that
/// connects to one of the addresses that passed the check. A later lookup
/// cannot substitute a private address (DNS rebinding).
async fn client_pinned_to_checked_address(
    base: &reqwest::Client,
    url: &reqwest::Url,
) -> anyhow::Result<reqwest::Client> {
    let _ = base;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("feed URL has no host"))?;
    let port = url.port_or_known_default().unwrap_or(80);
    let vetted = vetted_addresses(host, port).await?;
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none());
    for addr in vetted {
        builder = builder.resolve(host, std::net::SocketAddr::new(addr, port));
    }
    Ok(builder.build()?)
}

async fn vetted_addresses(host: &str, port: u16) -> anyhow::Result<Vec<std::net::IpAddr>> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        fetch::assert_public_addresses(&[ip]).map_err(|e| anyhow::anyhow!("{e}"))?;
        return Ok(vec![ip]);
    }
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| anyhow::anyhow!("could not resolve feed host {host}: {e}"))?
        .map(|addr| addr.ip())
        .collect::<Vec<_>>();
    fetch::assert_public_addresses(&addresses).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(addresses)
}

/// Refuse a declared Content-Length over the cap, then stream the body and
/// stop once the running total exceeds it. The whole response is never buffered
/// first.
async fn read_capped_body(response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    // content_length() is the body size hint, which a chunked or hostile
    // response can omit or understate. The declared header is checked on its own.
    let declared = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if let Some(len) = declared
        && len > fetch::MAX_FEED_BYTES as u64
    {
        return Err(anyhow::anyhow!(
            "feed Content-Length {len} exceeds the {max} byte cap",
            max = fetch::MAX_FEED_BYTES
        ));
    }
    let mut body = Vec::new();
    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > fetch::MAX_FEED_BYTES {
            return Err(anyhow::anyhow!(
                "feed body exceeds the {max} byte cap",
                max = fetch::MAX_FEED_BYTES
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Follow at most [`MAX_REDIRECTS`] hops. Every hop is address-checked and
/// pinned the same way as the first request.
async fn send_following_checked_redirects(
    client: &reqwest::Client,
    url: reqwest::Url,
) -> anyhow::Result<reqwest::Response> {
    let mut current = url;
    for _ in 0..=MAX_REDIRECTS {
        let pinned = client_pinned_to_checked_address(client, &current).await?;
        let response = pinned.get(current.clone()).send().await?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow::anyhow!("redirect with no Location header"))?;
            current = current
                .join(location)
                .map_err(|_| anyhow::anyhow!("redirect Location is not a URL"))?;
            fetch::check_feed_url(current.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
            continue;
        }
        return Ok(response);
    }
    Err(anyhow::anyhow!(
        "feed redirected more than {MAX_REDIRECTS} times"
    ))
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

    #[test]
    fn a_redirect_target_on_loopback_is_the_same_refusal_as_the_first_hop() {
        // Every hop runs check_feed_url before it is resolved or connected.
        // A 30x Location of a loopback (or any other forbidden) address is
        // the error send_following_checked_redirects returns instead of
        // issuing the next request.
        for location in [
            "http://127.0.0.1/secret",
            "http://10.0.0.1/secret",
            "http://0.1.2.3/secret",
            "http://[::1]/secret",
            "http://[64:ff9b::8.8.8.8]/secret",
        ] {
            assert_eq!(
                fetch::check_feed_url(location),
                Err(fetch::FeedUrlError::ForbiddenAddress),
                "{location}"
            );
        }
    }

    #[tokio::test]
    async fn an_oversized_content_length_is_refused_without_reading_the_body() {
        let mut response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(reqwest::Body::from("not-read"))
                .unwrap(),
        );
        response.headers_mut().insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_str(&(fetch::MAX_FEED_BYTES as u64 + 1).to_string()).unwrap(),
        );
        let err = read_capped_body(response).await.unwrap_err();
        assert!(err.to_string().contains("Content-Length"), "{err}");
    }

    #[tokio::test]
    async fn a_body_that_grows_past_the_cap_is_refused() {
        let chunk = vec![b'a'; 64 * 1024];
        let parts = std::iter::repeat_n(chunk, 40).map(Ok::<_, std::io::Error>);
        let response = http::Response::builder()
            .status(200)
            .body(reqwest::Body::wrap_stream(futures::stream::iter(parts)))
            .unwrap();
        let response = reqwest::Response::from(response);
        let err = read_capped_body(response).await.unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{err}");
    }

    #[tokio::test]
    async fn a_small_body_is_returned_whole() {
        let response = http::Response::builder()
            .status(200)
            .body(reqwest::Body::from("<rss/>"))
            .unwrap();
        let response = reqwest::Response::from(response);
        let body = read_capped_body(response).await.unwrap();
        assert_eq!(body, b"<rss/>");
    }
}

use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, params};
use tracing::info;

use super::models::{RssItem, RssRule, RssRuleEpisode};

/// Matched-episode history kept per rule; older entries are pruned on insert.
pub const RULE_EPISODES_KEEP: usize = 200;

/// Database handle for RSS feed item and rule persistence.
pub struct RssDatabase {
    conn: Connection,
}

impl RssDatabase {
    /// Open (or create) the RSS database at the given path.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );",
        )?;

        let version: u32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if version < 1 {
            info!("Applying RSS database migration v1");
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS rss_items (
                    id TEXT PRIMARY KEY,
                    feed_name TEXT NOT NULL,
                    title TEXT NOT NULL,
                    url TEXT,
                    published_at TEXT,
                    first_seen_at TEXT NOT NULL,
                    downloaded INTEGER NOT NULL DEFAULT 0,
                    downloaded_at TEXT,
                    category TEXT,
                    size_bytes INTEGER DEFAULT 0
                );

                CREATE INDEX IF NOT EXISTS idx_rss_items_feed ON rss_items(feed_name);
                CREATE INDEX IF NOT EXISTS idx_rss_items_seen ON rss_items(first_seen_at DESC);

                CREATE TABLE IF NOT EXISTS rss_rules (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    feed_name TEXT NOT NULL,
                    category TEXT,
                    priority INTEGER NOT NULL DEFAULT 1,
                    match_regex TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1
                );

                INSERT INTO schema_version (version) VALUES (1);
                ",
            )?;
            tx.commit()?;
        }

        // Store RSS feed configs in the database so they persist across restarts
        // without needing a config file.
        if version < 2 {
            info!("Applying RSS database migration v2");
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS rss_feeds (
                    name TEXT PRIMARY KEY,
                    url TEXT NOT NULL,
                    poll_interval_secs INTEGER NOT NULL DEFAULT 900,
                    category TEXT,
                    filter_regex TEXT,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    auto_download INTEGER NOT NULL DEFAULT 0
                );

                UPDATE schema_version SET version = 2;
                ",
            )?;
            tx.commit()?;
        }

        if version < 3 {
            info!("Applying RSS database migration v3");
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS rss_episodes (
                    series TEXT NOT NULL,
                    season INTEGER NOT NULL,
                    episode INTEGER NOT NULL,
                    PRIMARY KEY (series, season, episode)
                );

                UPDATE schema_version SET version = 3;
                ",
            )?;
            tx.commit()?;
        }

        // Per-rule smart-episode setting, and the episodes each rule matched.
        if version < 4 {
            info!("Applying RSS database migration v4");
            let tx = self.conn.unchecked_transaction()?;
            // Idempotent: a v4 run interrupted before the version bump may
            // already have added the column.
            let has_track_episodes: bool = tx.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('rss_rules') WHERE name = 'track_episodes'",
                [],
                |row| row.get::<_, i64>(0).map(|n| n > 0),
            )?;
            if !has_track_episodes {
                tx.execute_batch(
                    "ALTER TABLE rss_rules ADD COLUMN track_episodes INTEGER NOT NULL DEFAULT 1;",
                )?;
            }
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS rss_rule_episodes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    rule_id TEXT NOT NULL,
                    show TEXT NOT NULL,
                    season INTEGER NOT NULL,
                    episode INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    matched_at TEXT NOT NULL,
                    torrent_id INTEGER
                );

                CREATE INDEX IF NOT EXISTS idx_rss_rule_episodes_rule
                    ON rss_rule_episodes(rule_id, id DESC);

                UPDATE schema_version SET version = 4;
                ",
            )?;
            tx.commit()?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // RSS feed config operations (stored in DB for API-managed feeds)
    // -----------------------------------------------------------------------

    /// List all RSS feed configs from the database.
    pub fn feed_list(&self) -> anyhow::Result<Vec<super::config::RssFeedConfig>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, url, poll_interval_secs, category, filter_regex, enabled, auto_download
             FROM rss_feeds ORDER BY name ASC",
        )?;
        let feeds = stmt
            .query_map([], |row| {
                Ok(super::config::RssFeedConfig {
                    name: row.get(0)?,
                    url: row.get(1)?,
                    poll_interval_secs: row.get::<_, i64>(2)? as u64,
                    category: row.get(3)?,
                    filter_regex: row.get(4)?,
                    enabled: row.get::<_, i32>(5)? != 0,
                    auto_download: row.get::<_, i32>(6)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(feeds)
    }

    /// Insert or replace an RSS feed config.
    pub fn feed_upsert(&self, feed: &super::config::RssFeedConfig) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO rss_feeds (name, url, poll_interval_secs, category, filter_regex, enabled, auto_download)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                feed.name,
                feed.url,
                feed.poll_interval_secs as i64,
                feed.category,
                feed.filter_regex,
                feed.enabled as i32,
                feed.auto_download as i32,
            ],
        )?;
        Ok(())
    }

    /// Delete an RSS feed config by name, including persisted items and rule references.
    pub fn feed_delete(&self, name: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM rss_feeds WHERE name = ?1", params![name])?;
        self.conn
            .execute("DELETE FROM rss_items WHERE feed_name = ?1", params![name])?;

        for mut rule in self.rss_rule_list()? {
            let original_len = rule.feed_names.len();
            rule.feed_names.retain(|feed_name| feed_name != name);
            if rule.feed_names.len() == original_len {
                continue;
            }

            if rule.feed_names.is_empty() {
                self.rss_rule_delete(&rule.id)?;
            } else {
                self.rss_rule_update(&rule)?;
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // RSS item operations
    // -----------------------------------------------------------------------

    /// Upsert an RSS feed item (insert or ignore if already exists).
    pub fn rss_item_upsert(&self, item: &RssItem) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO rss_items (id, feed_name, title, url, published_at,
             first_seen_at, downloaded, downloaded_at, category, size_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                item.id,
                item.feed_name,
                item.title,
                item.url,
                item.published_at.map(|d| d.to_rfc3339()),
                item.first_seen_at.to_rfc3339(),
                item.downloaded as i32,
                item.downloaded_at.map(|d| d.to_rfc3339()),
                item.category,
                item.size_bytes as i64,
            ],
        )?;
        Ok(())
    }

    /// Check if an RSS item ID already exists.
    pub fn rss_item_exists(&self, id: &str) -> anyhow::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM rss_items WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// List RSS items, optionally filtered by feed name.
    pub fn rss_items_list(
        &self,
        feed_name: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<RssItem>> {
        if let Some(name) = feed_name {
            let mut stmt = self.conn.prepare(
                "SELECT id, feed_name, title, url, published_at, first_seen_at,
                 downloaded, downloaded_at, category, size_bytes
                 FROM rss_items WHERE feed_name = ?1
                 ORDER BY first_seen_at DESC LIMIT ?2",
            )?;
            let items = stmt
                .query_map(params![name, limit as i64], |row| self.map_rss_item(row))?
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(items);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, feed_name, title, url, published_at, first_seen_at,
             downloaded, downloaded_at, category, size_bytes
             FROM rss_items ORDER BY first_seen_at DESC LIMIT ?1",
        )?;
        let items = stmt
            .query_map(params![limit as i64], |row| self.map_rss_item(row))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    /// Get a single RSS item by ID.
    pub fn rss_item_get(&self, id: &str) -> anyhow::Result<Option<RssItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, feed_name, title, url, published_at, first_seen_at,
             downloaded, downloaded_at, category, size_bytes
             FROM rss_items WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| self.map_rss_item(row));
        match result {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Mark an RSS item as downloaded.
    pub fn rss_item_mark_downloaded(&self, id: &str, category: Option<&str>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE rss_items SET downloaded = 1, downloaded_at = ?2, category = ?3 WHERE id = ?1",
            params![id, Utc::now().to_rfc3339(), category],
        )?;
        Ok(())
    }

    /// Count total RSS items.
    pub fn rss_item_count(&self) -> anyhow::Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM rss_items", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Prune RSS items to keep only the N most recent.
    pub fn rss_items_prune(&self, keep: usize) -> anyhow::Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM rss_items WHERE id NOT IN (
                SELECT id FROM rss_items ORDER BY first_seen_at DESC LIMIT ?1
            )",
            params![keep as i64],
        )?;
        Ok(deleted)
    }

    fn map_rss_item(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<RssItem> {
        Ok(RssItem {
            id: row.get(0)?,
            feed_name: row.get(1)?,
            title: row.get(2)?,
            url: row.get(3)?,
            published_at: row.get::<_, Option<String>>(4)?.map(|s| parse_datetime(&s)),
            first_seen_at: parse_datetime(&row.get::<_, String>(5)?),
            downloaded: row.get::<_, i32>(6)? != 0,
            downloaded_at: row.get::<_, Option<String>>(7)?.map(|s| parse_datetime(&s)),
            category: row.get(8)?,
            size_bytes: row.get::<_, i64>(9)? as u64,
        })
    }

    // -----------------------------------------------------------------------
    // RSS rule operations
    // -----------------------------------------------------------------------

    /// Insert a new RSS download rule.
    pub fn rss_rule_insert(&self, rule: &RssRule) -> anyhow::Result<()> {
        let feed_names_str = rule.feed_names.join(",");
        self.conn.execute(
            "INSERT INTO rss_rules (id, name, feed_name, category, priority, match_regex, enabled,
             track_episodes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                rule.id,
                rule.name,
                feed_names_str,
                rule.category,
                rule.priority,
                rule.match_regex,
                rule.enabled as i32,
                rule.track_episodes as i32,
            ],
        )?;
        Ok(())
    }

    /// List all RSS download rules.
    pub fn rss_rule_list(&self) -> anyhow::Result<Vec<RssRule>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, feed_name, category, priority, match_regex, enabled, track_episodes
             FROM rss_rules ORDER BY name ASC",
        )?;
        let rules = stmt
            .query_map([], |row| {
                let feed_names_str: String = row.get(2)?;
                let feed_names: Vec<String> = feed_names_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                Ok(RssRule {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    feed_names,
                    category: row.get(3)?,
                    priority: row.get(4)?,
                    match_regex: row.get(5)?,
                    enabled: row.get::<_, i32>(6)? != 0,
                    track_episodes: row.get::<_, i32>(7)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rules)
    }

    /// Update an RSS download rule.
    pub fn rss_rule_update(&self, rule: &RssRule) -> anyhow::Result<()> {
        let feed_names_str = rule.feed_names.join(",");
        self.conn.execute(
            "UPDATE rss_rules SET name=?2, feed_name=?3, category=?4, priority=?5,
             match_regex=?6, enabled=?7, track_episodes=?8 WHERE id=?1",
            params![
                rule.id,
                rule.name,
                feed_names_str,
                rule.category,
                rule.priority,
                rule.match_regex,
                rule.enabled as i32,
                rule.track_episodes as i32,
            ],
        )?;
        Ok(())
    }

    /// Delete an RSS download rule.
    pub fn rss_rule_delete(&self, id: &str) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM rss_rules WHERE id=?1", params![id])?;
        tx.execute(
            "DELETE FROM rss_rule_episodes WHERE rule_id=?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Episode tracking
    // -----------------------------------------------------------------------

    /// Episodes already downloaded, keyed by the series name.
    pub fn episodes_load(&self) -> anyhow::Result<super::rules::EpisodeTracker> {
        let mut tracker = super::rules::EpisodeTracker::new();
        let mut stmt = self
            .conn
            .prepare("SELECT series, season, episode FROM rss_episodes")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                u32::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                u32::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
            ))
        })?;
        for row in rows {
            let (series, season, episode) = row?;
            tracker.record(&series, super::rules::Episode { season, episode });
        }
        Ok(tracker)
    }

    /// Record one downloaded episode. A repeat is stored once.
    pub fn episode_record(
        &self,
        series: &str,
        episode: super::rules::Episode,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO rss_episodes (series, season, episode) VALUES (?1, ?2, ?3)",
            params![series, episode.season as i64, episode.episode as i64],
        )?;
        Ok(())
    }

    /// Record that a rule matched and downloaded an episode, keeping only the
    /// newest [`RULE_EPISODES_KEEP`] entries for the rule.
    pub fn rule_episode_record(&self, rule_id: &str, entry: &RssRuleEpisode) -> anyhow::Result<()> {
        self.rule_episode_record_keep(rule_id, entry, RULE_EPISODES_KEEP)
    }

    fn rule_episode_record_keep(
        &self,
        rule_id: &str,
        entry: &RssRuleEpisode,
        keep: usize,
    ) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO rss_rule_episodes
             (rule_id, show, season, episode, title, matched_at, torrent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rule_id,
                entry.show,
                entry.season as i64,
                entry.episode as i64,
                entry.title,
                entry.matched_at.to_rfc3339(),
                entry.torrent_id.and_then(|id| i64::try_from(id).ok()),
            ],
        )?;
        tx.execute(
            "DELETE FROM rss_rule_episodes WHERE rule_id = ?1 AND id NOT IN (
                SELECT id FROM rss_rule_episodes WHERE rule_id = ?1 ORDER BY id DESC LIMIT ?2
            )",
            params![rule_id, i64::try_from(keep).unwrap_or(i64::MAX)],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Episodes a rule matched, newest first, at most `limit`.
    pub fn rule_episodes_list(
        &self,
        rule_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<RssRuleEpisode>> {
        let mut stmt = self.conn.prepare(
            "SELECT show, season, episode, title, matched_at, torrent_id
             FROM rss_rule_episodes WHERE rule_id = ?1
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![rule_id, limit as i64], |row| {
                Ok(RssRuleEpisode {
                    show: row.get(0)?,
                    season: u32::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                    episode: u32::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                    title: row.get(3)?,
                    matched_at: parse_datetime(&row.get::<_, String>(4)?),
                    torrent_id: row
                        .get::<_, Option<i64>>(5)?
                        .and_then(|id| usize::try_from(id).ok()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Whether a rule with this id exists.
    pub fn rss_rule_exists(&self, id: &str) -> anyhow::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM rss_rules WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

fn parse_datetime(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::rss::config::RssFeedConfig;

    fn test_db() -> (tempfile::TempDir, RssDatabase) {
        let dir = tempfile::tempdir().unwrap();
        let db = RssDatabase::open(&dir.path().join("rss.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn feed_delete_removes_items_and_rule_references() {
        let (_dir, db) = test_db();
        db.feed_upsert(&RssFeedConfig {
            name: "feed-a".into(),
            url: "https://example.invalid/a.xml".into(),
            poll_interval_secs: 15,
            category: None,
            filter_regex: None,
            enabled: true,
            auto_download: false,
        })
        .unwrap();
        db.feed_upsert(&RssFeedConfig {
            name: "feed-b".into(),
            url: "https://example.invalid/b.xml".into(),
            poll_interval_secs: 15,
            category: None,
            filter_regex: None,
            enabled: true,
            auto_download: false,
        })
        .unwrap();
        db.rss_item_upsert(&RssItem {
            id: "item-a".into(),
            feed_name: "feed-a".into(),
            title: "Item A".into(),
            url: None,
            published_at: None,
            first_seen_at: Utc::now(),
            downloaded: false,
            downloaded_at: None,
            category: None,
            size_bytes: 0,
        })
        .unwrap();
        db.rss_item_upsert(&RssItem {
            id: "item-b".into(),
            feed_name: "feed-b".into(),
            title: "Item B".into(),
            url: None,
            published_at: None,
            first_seen_at: Utc::now(),
            downloaded: false,
            downloaded_at: None,
            category: None,
            size_bytes: 0,
        })
        .unwrap();
        db.rss_rule_insert(&RssRule {
            id: "rule-shared".into(),
            name: "shared".into(),
            feed_names: vec!["feed-a".into(), "feed-b".into()],
            category: None,
            priority: 1,
            match_regex: ".*".into(),
            enabled: true,
            track_episodes: true,
        })
        .unwrap();
        db.rss_rule_insert(&RssRule {
            id: "rule-a".into(),
            name: "a only".into(),
            feed_names: vec!["feed-a".into()],
            category: None,
            priority: 1,
            match_regex: ".*".into(),
            enabled: true,
            track_episodes: true,
        })
        .unwrap();

        db.feed_delete("feed-a").unwrap();

        assert!(
            db.feed_list()
                .unwrap()
                .iter()
                .all(|feed| feed.name != "feed-a")
        );
        assert!(db.rss_items_list(Some("feed-a"), 10).unwrap().is_empty());
        assert_eq!(db.rss_items_list(Some("feed-b"), 10).unwrap().len(), 1);

        let rules = db.rss_rule_list().unwrap();
        assert!(rules.iter().all(|rule| rule.id != "rule-a"));
        let shared = rules.iter().find(|rule| rule.id == "rule-shared").unwrap();
        assert_eq!(shared.feed_names, vec!["feed-b".to_string()]);
    }

    #[test]
    fn migrates_a_v3_database_keeping_rules_with_episode_tracking_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rss.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE schema_version (version INTEGER NOT NULL);
                 INSERT INTO schema_version (version) VALUES (3);
                 CREATE TABLE rss_items (id TEXT PRIMARY KEY, feed_name TEXT NOT NULL,
                    title TEXT NOT NULL, url TEXT, published_at TEXT,
                    first_seen_at TEXT NOT NULL, downloaded INTEGER NOT NULL DEFAULT 0,
                    downloaded_at TEXT, category TEXT, size_bytes INTEGER DEFAULT 0);
                 CREATE TABLE rss_rules (id TEXT PRIMARY KEY, name TEXT NOT NULL,
                    feed_name TEXT NOT NULL, category TEXT,
                    priority INTEGER NOT NULL DEFAULT 1, match_regex TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1);
                 CREATE TABLE rss_feeds (name TEXT PRIMARY KEY, url TEXT NOT NULL,
                    poll_interval_secs INTEGER NOT NULL DEFAULT 900, category TEXT,
                    filter_regex TEXT, enabled INTEGER NOT NULL DEFAULT 1,
                    auto_download INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE rss_episodes (series TEXT NOT NULL, season INTEGER NOT NULL,
                    episode INTEGER NOT NULL, PRIMARY KEY (series, season, episode));
                 INSERT INTO rss_rules (id, name, feed_name, match_regex)
                    VALUES ('old', 'Old rule', 'feed', 'S01');",
            )
            .unwrap();
        }

        let db = RssDatabase::open(&path).unwrap();
        let rules = db.rss_rule_list().unwrap();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].track_episodes, "existing rules keep tracking on");
        assert!(db.rule_episodes_list("old", 10).unwrap().is_empty());

        // Opening again does not re-run the migration.
        drop(db);
        let db = RssDatabase::open(&path).unwrap();
        assert!(db.rss_rule_list().unwrap()[0].track_episodes);
    }

    #[test]
    fn rule_episodes_are_listed_newest_first_capped_and_dropped_with_the_rule() {
        let (_dir, db) = test_db();
        let mut rule = RssRule {
            id: "r".into(),
            name: "r".into(),
            feed_names: vec!["f".into()],
            category: None,
            priority: 1,
            match_regex: ".*".into(),
            enabled: true,
            track_episodes: false,
        };
        db.rss_rule_insert(&rule).unwrap();
        assert!(!db.rss_rule_list().unwrap()[0].track_episodes);
        rule.track_episodes = true;
        db.rss_rule_update(&rule).unwrap();
        assert!(db.rss_rule_list().unwrap()[0].track_episodes);

        for episode in 1..=3 {
            db.rule_episode_record(
                "r",
                &RssRuleEpisode {
                    show: "show".into(),
                    season: 1,
                    episode,
                    title: format!("Show S01E0{episode}"),
                    matched_at: Utc::now(),
                    torrent_id: (episode != 2).then_some(episode as usize),
                },
            )
            .unwrap();
        }
        let listed = db.rule_episodes_list("r", 2).unwrap();
        assert_eq!(listed.iter().map(|e| e.episode).collect::<Vec<_>>(), [3, 2]);
        assert_eq!(listed[1].torrent_id, None);

        db.rss_rule_delete("r").unwrap();
        assert!(db.rule_episodes_list("r", 10).unwrap().is_empty());
    }

    #[test]
    fn reopens_after_a_v4_migration_interrupted_before_the_version_bump() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rss.db");
        drop(RssDatabase::open(&path).unwrap());
        {
            // Column added, but the run died before recording version 4.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "DROP TABLE rss_rule_episodes; UPDATE schema_version SET version = 3;",
            )
            .unwrap();
        }

        let db = RssDatabase::open(&path).unwrap();
        let version: u32 = db
            .conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 4);
        assert!(db.rule_episodes_list("any", 10).unwrap().is_empty());
        drop(db);
        RssDatabase::open(&path).unwrap();
    }

    #[test]
    fn rule_episodes_are_capped_to_the_newest_per_rule() {
        let (_dir, db) = test_db();
        let entry = |episode: u32| RssRuleEpisode {
            show: "show".into(),
            season: 1,
            episode,
            title: format!("E{episode}"),
            matched_at: Utc::now(),
            torrent_id: None,
        };
        for episode in 1..=5 {
            db.rule_episode_record_keep("a", &entry(episode), 3)
                .unwrap();
        }
        db.rule_episode_record_keep("b", &entry(1), 3).unwrap();

        let listed = db.rule_episodes_list("a", 10).unwrap();
        assert_eq!(
            listed.iter().map(|e| e.episode).collect::<Vec<_>>(),
            [5, 4, 3]
        );
        assert_eq!(db.rule_episodes_list("b", 10).unwrap().len(), 1);
    }
}

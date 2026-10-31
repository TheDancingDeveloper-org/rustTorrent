use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use super::ApiState;
use crate::{
    ApiError,
    api::Result,
    rss::{
        config::RssFeedConfig,
        fetch::{self, FeedUrlError},
        models::RssRule,
        rules::{self, PatternError},
    },
};

fn feed_url_error(err: FeedUrlError) -> ApiError {
    ApiError::from((
        axum::http::StatusCode::BAD_REQUEST,
        anyhow::anyhow!("invalid feed URL: {err}"),
    ))
}

fn pattern_error(err: PatternError) -> ApiError {
    ApiError::from((
        axum::http::StatusCode::BAD_REQUEST,
        anyhow::anyhow!("invalid filter pattern: {err}"),
    ))
}

/// Refuse a feed whose URL is not http(s) or names a private address, and a
/// filter pattern that is empty, oversized or will not compile.
fn validate_feed(feed: &RssFeedConfig) -> Result<()> {
    fetch::check_feed_url(&feed.url).map_err(feed_url_error)?;
    if let Some(pattern) = &feed.filter_regex {
        rules::BoundedPattern::compile(pattern).map_err(pattern_error)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rss_db(
    state: &ApiState,
) -> Result<&std::sync::Arc<parking_lot::Mutex<crate::rss::db::RssDatabase>>> {
    state
        .opts
        .rss_db
        .as_ref()
        .ok_or_else(|| ApiError::from(anyhow::anyhow!("RSS is not enabled")))
}

// ---------------------------------------------------------------------------
// RSS feed config handlers
// ---------------------------------------------------------------------------

/// GET /rss/feeds -- List RSS feeds.
pub async fn h_rss_feeds_list(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let db = rss_db(&state)?;
    let feeds = db.lock().feed_list().map_err(ApiError::from)?;
    Ok(Json(feeds))
}

/// POST /rss/feeds -- Add an RSS feed.
pub async fn h_rss_feed_add(
    State(state): State<ApiState>,
    Json(feed): Json<RssFeedConfig>,
) -> Result<impl IntoResponse> {
    let db = rss_db(&state)?;
    let db = db.lock();
    let existing = db.feed_list().map_err(ApiError::from)?;
    if existing.iter().any(|f| f.name == feed.name) {
        return Err(ApiError::from(anyhow::anyhow!(
            "Feed '{}' already exists",
            feed.name
        )));
    }
    validate_feed(&feed)?;
    db.feed_upsert(&feed).map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({"status": true})))
}

/// PUT /rss/feeds/{name} -- Update an RSS feed.
pub async fn h_rss_feed_update(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    Json(feed): Json<RssFeedConfig>,
) -> Result<impl IntoResponse> {
    let db = rss_db(&state)?;
    let db = db.lock();
    // If the name changed, delete the old one
    if feed.name != name {
        db.feed_delete(&name).map_err(ApiError::from)?;
    }
    validate_feed(&feed)?;
    db.feed_upsert(&feed).map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({"status": true})))
}

/// DELETE /rss/feeds/{name} -- Delete an RSS feed.
pub async fn h_rss_feed_delete(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse> {
    let db = rss_db(&state)?;
    db.lock().feed_delete(&name).map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({"status": true})))
}

// ---------------------------------------------------------------------------
// RSS item handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct RssItemsQuery {
    pub feed: Option<String>,
    pub limit: Option<usize>,
}

/// GET /rss/items -- List RSS feed items.
pub async fn h_rss_items_list(
    State(state): State<ApiState>,
    Query(q): Query<RssItemsQuery>,
) -> Result<impl IntoResponse> {
    let db = rss_db(&state)?;
    let limit = q.limit.unwrap_or(500);
    let items = db
        .lock()
        .rss_items_list(q.feed.as_deref(), limit)
        .map_err(ApiError::from)?;
    Ok(Json(items))
}

/// POST /rss/items/{id}/download -- Download a specific RSS feed item as a torrent.
pub async fn h_rss_item_download(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse> {
    let rss = rss_db(&state)?;

    let item = {
        let db = rss.lock();
        db.rss_item_get(&id)
            .map_err(ApiError::from)?
            .ok_or_else(|| ApiError::from(anyhow::anyhow!("RSS item not found")))?
    };

    let url = item
        .url
        .as_ref()
        .ok_or_else(|| ApiError::from(anyhow::anyhow!("No download URL for this item")))?;

    // Add the torrent via the session
    use crate::session::{AddTorrent, AddTorrentOptions};
    use std::borrow::Cow;

    let opts = AddTorrentOptions {
        overwrite: true,
        category: item.category.clone(),
        ..Default::default()
    };

    state
        .api
        .session()
        .add_torrent(AddTorrent::Url(Cow::Owned(url.clone())), Some(opts))
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to add torrent: {:#}", e)))?;

    // Mark as downloaded
    {
        let db = rss.lock();
        let _ = db.rss_item_mark_downloaded(&id, item.category.as_deref());
    }

    Ok(Json(serde_json::json!({"status": true})))
}

// ---------------------------------------------------------------------------
// RSS rule handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RssRuleBody {
    pub name: String,
    pub feed_names: Vec<String>,
    pub category: Option<String>,
    pub priority: Option<i32>,
    pub match_regex: String,
    pub enabled: Option<bool>,
    /// Skip episodes already downloaded. Defaults to on.
    pub track_episodes: Option<bool>,
}

/// GET /rss/rules -- List RSS download rules.
pub async fn h_rss_rules_list(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let db = rss_db(&state)?;
    let rules = db.lock().rss_rule_list().map_err(ApiError::from)?;
    Ok(Json(rules))
}

/// POST /rss/rules -- Add an RSS download rule.
pub async fn h_rss_rule_add(
    State(state): State<ApiState>,
    Json(body): Json<RssRuleBody>,
) -> Result<impl IntoResponse> {
    // Validate the regex
    rules::BoundedPattern::compile(&body.match_regex).map_err(pattern_error)?;

    let rule = RssRule {
        id: uuid::Uuid::new_v4().to_string(),
        name: body.name,
        feed_names: body.feed_names,
        category: body.category,
        priority: body.priority.unwrap_or(1),
        match_regex: body.match_regex,
        enabled: body.enabled.unwrap_or(true),
        track_episodes: body.track_episodes.unwrap_or(true),
    };
    let db = rss_db(&state)?;
    db.lock().rss_rule_insert(&rule).map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({"status": true})))
}

/// PUT /rss/rules/{id} -- Update an RSS download rule.
pub async fn h_rss_rule_update(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<RssRuleBody>,
) -> Result<impl IntoResponse> {
    rules::BoundedPattern::compile(&body.match_regex).map_err(pattern_error)?;

    let rule = RssRule {
        id,
        name: body.name,
        feed_names: body.feed_names,
        category: body.category,
        priority: body.priority.unwrap_or(1),
        match_regex: body.match_regex,
        enabled: body.enabled.unwrap_or(true),
        track_episodes: body.track_episodes.unwrap_or(true),
    };
    let db = rss_db(&state)?;
    db.lock().rss_rule_update(&rule).map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({"status": true})))
}

/// DELETE /rss/rules/{id} -- Delete an RSS download rule.
pub async fn h_rss_rule_delete(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse> {
    let db = rss_db(&state)?;
    db.lock().rss_rule_delete(&id).map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({"status": true})))
}

/// Default and maximum number of episodes `GET /rss/rules/{id}/episodes` returns.
pub(crate) const RULE_EPISODES_DEFAULT_LIMIT: usize = 100;
pub(crate) const RULE_EPISODES_MAX_LIMIT: usize = 1000;

#[derive(Deserialize, Default)]
pub struct RuleEpisodesQuery {
    pub limit: Option<usize>,
}

/// GET /rss/rules/{id}/episodes -- Episodes a rule matched, newest first.
pub async fn h_rss_rule_episodes(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<RuleEpisodesQuery>,
) -> Result<impl IntoResponse> {
    let db = rss_db(&state)?;
    let db = db.lock();
    if !db.rss_rule_exists(&id).map_err(ApiError::from)? {
        return Err(ApiError::from((
            axum::http::StatusCode::NOT_FOUND,
            anyhow::anyhow!("RSS rule not found"),
        )));
    }
    let limit = q
        .limit
        .unwrap_or(RULE_EPISODES_DEFAULT_LIMIT)
        .clamp(1, RULE_EPISODES_MAX_LIMIT);
    let episodes = db.rule_episodes_list(&id, limit).map_err(ApiError::from)?;
    Ok(Json(episodes))
}

// ---------------------------------------------------------------------------
// RSS settings handler
// ---------------------------------------------------------------------------

/// GET /rss/settings -- Get RSS settings.
pub async fn h_rss_settings_get(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(serde_json::json!({
        "rss_history_limit": state.opts.rss_history_limit.unwrap_or(500),
    })))
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, sync::Arc};

    use axum::{
        Json,
        extract::{Path, Query, State},
        response::IntoResponse,
    };
    use http::StatusCode;

    use super::*;
    use crate::{
        Api, ListenerMode, Session, SessionOptions,
        http_api::{HttpApi, HttpApiOptions},
        listen::ListenerOptions,
        rss::{db::RssDatabase, models::RssRuleEpisode},
    };

    async fn rss_state() -> (
        ApiState,
        Arc<parking_lot::Mutex<RssDatabase>>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::TempDir::with_prefix("rss_handlers").unwrap();
        let session = Session::new_with_opts(
            dir.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    mode: ListenerMode::TcpOnly,
                    listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = Api::new(
            session,
            None,
            #[cfg(feature = "tracing-subscriber-utils")]
            None,
        );
        let db = Arc::new(parking_lot::Mutex::new(
            RssDatabase::open(&dir.path().join("rss.db")).unwrap(),
        ));
        let state = Arc::new(HttpApi::new(
            api,
            Some(HttpApiOptions {
                rss_db: Some(db.clone()),
                ..Default::default()
            }),
        ));
        (state, db, dir)
    }

    fn body(track_episodes: Option<bool>) -> RssRuleBody {
        RssRuleBody {
            name: "Show".into(),
            feed_names: vec!["feed".into()],
            category: None,
            priority: None,
            match_regex: "S01".into(),
            enabled: None,
            track_episodes,
        }
    }

    async fn json(response: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn track_episodes_defaults_on_and_can_be_turned_off() {
        let (state, db, _dir) = rss_state().await;
        h_rss_rule_add(State(state.clone()), Json(body(None)))
            .await
            .unwrap();
        let id = db.lock().rss_rule_list().unwrap()[0].id.clone();
        assert!(db.lock().rss_rule_list().unwrap()[0].track_episodes);

        h_rss_rule_update(State(state.clone()), Path(id), Json(body(Some(false))))
            .await
            .unwrap();
        let (status, listed) = json(
            h_rss_rules_list(State(state))
                .await
                .unwrap()
                .into_response(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed[0]["track_episodes"], false);
    }

    #[tokio::test]
    async fn rule_episodes_lists_matches_newest_first_and_caps_the_limit() {
        let (state, db, _dir) = rss_state().await;
        h_rss_rule_add(State(state.clone()), Json(body(None)))
            .await
            .unwrap();
        let id = db.lock().rss_rule_list().unwrap()[0].id.clone();
        for episode in 1..=3 {
            db.lock()
                .rule_episode_record(
                    &id,
                    &RssRuleEpisode {
                        show: "show".into(),
                        season: 1,
                        episode,
                        title: format!("Show S01E0{episode}"),
                        matched_at: chrono::Utc::now(),
                        torrent_id: Some(episode as usize),
                    },
                )
                .unwrap();
        }

        let (status, listed) = json(
            h_rss_rule_episodes(
                State(state.clone()),
                Path(id.clone()),
                Query(RuleEpisodesQuery { limit: None }),
            )
            .await
            .unwrap()
            .into_response(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let listed = listed.as_array().unwrap();
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0]["episode"], 3);
        assert_eq!(listed[0]["show"], "show");
        assert_eq!(listed[0]["season"], 1);
        assert_eq!(listed[0]["title"], "Show S01E03");
        assert_eq!(listed[0]["torrent_id"], 3);
        assert!(listed[0]["matched_at"].is_string());

        let (_, capped) = json(
            h_rss_rule_episodes(
                State(state),
                Path(id),
                Query(RuleEpisodesQuery { limit: Some(1) }),
            )
            .await
            .unwrap()
            .into_response(),
        )
        .await;
        assert_eq!(capped.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rule_episodes_for_an_unknown_rule_is_not_found() {
        let (state, _db, _dir) = rss_state().await;
        let response = h_rss_rule_episodes(
            State(state),
            Path("missing".into()),
            Query(RuleEpisodesQuery::default()),
        )
        .await
        .map(IntoResponse::into_response)
        .unwrap_or_else(IntoResponse::into_response);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

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

// ---------------------------------------------------------------------------
// RSS settings handler
// ---------------------------------------------------------------------------

/// GET /rss/settings -- Get RSS settings.
pub async fn h_rss_settings_get(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(serde_json::json!({
        "rss_history_limit": state.opts.rss_history_limit.unwrap_or(500),
    })))
}

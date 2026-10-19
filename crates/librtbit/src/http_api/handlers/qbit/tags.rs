//! Torrents controller (`torrents/*`): tag endpoints, backed by the session's tags.

use super::*;

// ---------------------------------------------------------------------------
// Tag endpoints (backed by the session's tags)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub(super) struct TagsForm {
    #[serde(default)]
    pub(super) hashes: String,
    #[serde(default)]
    pub(super) tags: String,
}

pub(super) async fn h_torrents_tags(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    axum::Json(state.api_state.api.api_list_tags())
}

pub(super) async fn h_torrents_create_tags(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if let Err(error) = state
        .api_state
        .api
        .api_create_tags(&parse_tags(&form.tags))
        .await
    {
        warn!(%error, "qbit compat: error creating tags");
    }
    "Ok."
}

pub(super) async fn h_torrents_delete_tags(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if let Err(error) = state
        .api_state
        .api
        .api_delete_tags(&parse_tags(&form.tags))
        .await
    {
        warn!(%error, "qbit compat: error deleting tags");
    }
    "Ok."
}

#[derive(Clone, Copy)]
pub(super) enum TagEdit {
    Add,
    Remove,
    Set,
}

/// Apply a per-torrent tag edit to every torrent named in the form.
pub(super) async fn edit_torrent_tags(
    state: &QbitState,
    body: &[u8],
    edit: TagEdit,
) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(body).unwrap_or_default();
    let api = &state.api_state.api;
    let tags = parse_tags(&form.tags);
    // addTags with nothing to add is a no-op rather than an error.
    if matches!(edit, TagEdit::Add) && tags.is_empty() {
        return "Ok.";
    }
    for idx in resolve_hashes(api, &form.hashes) {
        let result = match edit {
            TagEdit::Add => api.api_add_torrent_tags(idx, &tags).await,
            TagEdit::Remove => api.api_remove_torrent_tags(idx, &tags).await,
            TagEdit::Set => api.api_set_torrent_tags(idx, &tags).await,
        };
        if let Err(error) = result {
            warn!(%error, "qbit compat: error editing torrent tags");
        }
    }
    "Ok."
}

pub(super) async fn h_torrents_add_tags(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    edit_torrent_tags(&state, &body, TagEdit::Add).await
}

pub(super) async fn h_torrents_remove_tags(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    edit_torrent_tags(&state, &body, TagEdit::Remove).await
}

pub(super) async fn h_torrents_set_tags(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    edit_torrent_tags(&state, &body, TagEdit::Set).await
}

#[derive(Deserialize, Default)]
pub(super) struct SetLocationForm {
    // setLocation uses `hashes`/`location`; setSavePath uses `id`/`path`.
    #[serde(default, alias = "id")]
    pub(super) hashes: String,
    #[serde(default, alias = "path")]
    pub(super) location: String,
}

pub(super) async fn h_torrents_set_location(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: SetLocationForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let location = form.location.trim();
    if location.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "location is required");
    }
    let new_root = PathBuf::from(location);
    let mut last_error = None;
    for idx in resolve_hashes(api, &form.hashes) {
        let Ok(handle) = api.mgr_handle(idx) else {
            continue;
        };
        if let Err(error) = api.session().set_location(&handle, new_root.clone()).await {
            last_error = Some(error);
        }
    }
    match last_error {
        Some(error) => rename_conflict(error),
        None => (StatusCode::OK, "Ok.").into_response(),
    }
}

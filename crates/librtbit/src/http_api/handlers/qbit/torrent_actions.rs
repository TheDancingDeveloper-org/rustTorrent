//! Torrents controller (`torrents/*`): add, state changes, queue, files and trackers.

use super::*;

// ---------------------------------------------------------------------------
// Torrent actions (add, pause, resume, delete)
// ---------------------------------------------------------------------------

pub(super) async fn h_torrents_add(
    State(state): State<Arc<QbitState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut urls: Vec<String> = Vec::new();
    let mut torrent_bytes: Vec<Bytes> = Vec::new();
    let mut savepath: Option<String> = None;
    let mut category: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut ratelimits = LimitsConfig::default();
    let mut paused = false;
    let mut content_layout = ContentLayout::Original;

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "urls" => {
                if let Ok(text) = field.text().await {
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && urls.len() < 100 {
                            urls.push(trimmed.to_string());
                        }
                    }
                }
            }
            "torrents" => {
                if torrent_bytes.len() >= 100 {
                    continue;
                }
                if let Ok(data) = field.bytes().await
                    && !data.is_empty()
                {
                    torrent_bytes.push(data);
                }
            }
            "category" => {
                if let Ok(text) = field.text().await
                    && !text.is_empty()
                {
                    category = Some(text);
                }
            }
            "tags" => {
                if let Ok(text) = field.text().await {
                    tags = parse_tags(&text);
                }
            }
            // Per-torrent limits in bytes per second; 0 or -1 is unlimited.
            "dlLimit" | "upLimit" => {
                let is_download = field_name == "dlLimit";
                if let Ok(text) = field.text().await
                    && let Ok(limit) = text.trim().parse::<i64>()
                {
                    let bps = parse_torrent_limit(limit);
                    if is_download {
                        ratelimits.download_bps = bps;
                    } else {
                        ratelimits.upload_bps = bps;
                    }
                }
            }
            "savepath" => {
                if let Ok(text) = field.text().await
                    && !text.is_empty()
                {
                    savepath = Some(text);
                }
            }
            "contentLayout" => {
                if let Ok(text) = field.text().await {
                    content_layout = ContentLayout::parse(&text);
                }
            }
            // Pre-4.3 spelling: true is `Subfolder`, false is `NoSubfolder`.
            "root_folder" => {
                if let Ok(text) = field.text().await {
                    match text.trim() {
                        "true" => content_layout = ContentLayout::Subfolder,
                        "false" => content_layout = ContentLayout::NoSubfolder,
                        _ => {}
                    }
                }
            }
            // `paused` was renamed to `stopped` in WebAPI 2.11; accept both.
            "paused" | "stopped" => {
                if let Ok(text) = field.text().await
                    && text.eq_ignore_ascii_case("true")
                {
                    paused = true;
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let api = &state.api_state.api;
    let mut had_error = false;

    let base_opts = || AddTorrentOptions {
        paused,
        overwrite: true,
        category: category.clone(),
        tags: tags.clone(),
        ratelimits,
        ..Default::default()
    };
    let save_root = savepath.unwrap_or_else(|| api.api_output_folder());

    for url in urls {
        let mut opts = base_opts();
        content_layout.apply(&mut opts, &save_root, None);
        if let Err(e) = api
            .api_add_torrent(AddTorrent::Url(url.into()), Some(opts))
            .await
        {
            warn!("qbit compat: error adding torrent URL: {e:#}");
            had_error = true;
        }
    }

    for data in torrent_bytes {
        let mut opts = base_opts();
        let single_file_name = single_file_torrent_name(&data);
        content_layout.apply(&mut opts, &save_root, single_file_name.as_deref());
        if let Err(e) = api
            .api_add_torrent(AddTorrent::TorrentFileBytes(data), Some(opts))
            .await
        {
            warn!("qbit compat: error adding torrent file: {e:#}");
            had_error = true;
        }
    }

    if had_error {
        (StatusCode::INTERNAL_SERVER_ERROR, "Error adding torrent(s)")
    } else {
        (StatusCode::OK, "Ok.")
    }
}

/// `torrents/add` `contentLayout`: where the payload goes under the save path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContentLayout {
    /// Multi-file torrents get their root folder; single files sit directly
    /// in the save path.
    Original,
    /// Every torrent gets a root folder, single files included.
    Subfolder,
    /// No root folder: a multi-file torrent's files go straight into the save
    /// path.
    NoSubfolder,
}

impl ContentLayout {
    pub(super) fn parse(value: &str) -> Self {
        match value.trim() {
            "Subfolder" | "Create" => Self::Subfolder,
            "NoSubfolder" | "NoCreate" => Self::NoSubfolder,
            _ => Self::Original,
        }
    }

    /// Set the output folder for this layout. `single_file_name` is the file
    /// name of a single-file torrent when it is known before the add (an
    /// uploaded `.torrent`); a URL or magnet single-file torrent under
    /// `Subfolder` falls back to `Original`, as its name is not known yet.
    pub(super) fn apply(
        self,
        opts: &mut AddTorrentOptions,
        save_root: &str,
        single_file_name: Option<&str>,
    ) {
        match (self, single_file_name) {
            (Self::NoSubfolder, _) => opts.output_folder = Some(save_root.to_owned()),
            (Self::Subfolder, Some(file)) => {
                let stem = Path::new(file)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.to_owned());
                opts.output_folder = Some(
                    Path::new(save_root)
                        .join(stem)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            _ => opts.output_folder_root = Some(save_root.to_owned()),
        }
    }
}

/// The name of a single-file `.torrent`, or `None` for a multi-file or
/// unparseable one.
pub(super) fn single_file_torrent_name(torrent_bytes: &[u8]) -> Option<String> {
    let meta = librtbit_core::torrent_metainfo::torrent_from_bytes(torrent_bytes).ok()?;
    let info = meta.info.data;
    if info.files.is_some() || info.length.is_none() {
        return None;
    }
    info.name
        .map(|n| String::from_utf8_lossy(n.as_ref()).into_owned())
}

#[derive(Deserialize, Default)]
pub(super) struct HashesForm {
    #[serde(default)]
    pub(super) hashes: String,
}

/// Resolve hash(es) from form body. "all" means all torrents.
pub(super) fn resolve_hashes(api: &Api, hashes_str: &str) -> Vec<TorrentIdOrHash> {
    if hashes_str == "all" {
        api.session().with_torrents(|iter| {
            iter.map(|(_, handle)| TorrentIdOrHash::Hash(handle.shared().info_hash))
                .collect()
        })
    } else {
        hashes_str
            .split('|')
            .filter_map(|h| {
                let h = h.trim();
                if h.is_empty() {
                    return None;
                }
                TorrentIdOrHash::parse(h).ok()
            })
            .collect()
    }
}

pub(super) async fn h_torrents_pause(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);

    for idx in hashes {
        if let Err(e) = api.api_torrent_action_pause(idx).await {
            warn!("qbit compat: error pausing torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

pub(super) async fn h_torrents_resume(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);

    for idx in hashes {
        if let Err(e) = api.api_torrent_action_start(idx).await {
            warn!("qbit compat: error resuming torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

/// `torrents/stop` — the WebAPI 2.11+ name for `torrents/pause`. We advertise
/// 2.11.3, so modern clients (qbittorrent-api, newer *arr) call this.
pub(super) async fn h_torrents_stop(state: State<Arc<QbitState>>, body: Bytes) -> &'static str {
    h_torrents_pause(state, body).await
}

/// `torrents/start` — the WebAPI 2.11+ name for `torrents/resume`.
pub(super) async fn h_torrents_start(state: State<Arc<QbitState>>, body: Bytes) -> &'static str {
    h_torrents_resume(state, body).await
}

/// Move the torrents named by `hashes` within the session queue.
pub(super) async fn move_in_queue(state: &QbitState, body: &[u8], mv: QueueMove) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(body).unwrap_or_default();
    let api = &state.api_state.api;
    let ids: Vec<_> = resolve_hashes(api, &form.hashes)
        .into_iter()
        .filter_map(|idx| api.mgr_handle(idx).ok().map(|h| h.id()))
        .collect();
    api.session().move_in_queue(&ids, mv).await;
    "Ok."
}

pub(super) async fn h_torrents_top_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    move_in_queue(&state, &body, QueueMove::Top).await
}

pub(super) async fn h_torrents_bottom_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    move_in_queue(&state, &body, QueueMove::Bottom).await
}

pub(super) async fn h_torrents_increase_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    move_in_queue(&state, &body, QueueMove::Up).await
}

pub(super) async fn h_torrents_decrease_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    move_in_queue(&state, &body, QueueMove::Down).await
}

#[derive(Deserialize, Default)]
pub(super) struct ForceStartForm {
    #[serde(default)]
    pub(super) hashes: String,
    #[serde(default)]
    pub(super) value: String,
}

/// `torrents/setForceStart` — `value=true` starts the torrents outside the
/// queue limits; `value=false` hands them back to the queue.
pub(super) async fn h_torrents_set_force_start(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: ForceStartForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let enabled = form.value.eq_ignore_ascii_case("true");
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        let Ok(handle) = api.mgr_handle(idx) else {
            continue;
        };
        if let Err(error) = api.session().set_force_start(&handle, enabled).await {
            warn!(%error, "qbit compat: error setting force start");
        }
    }
    "Ok."
}

/// `torrents/toggleSequentialDownload` — flips sequential piece order on each
/// torrent. The picker reads the flag on its next pick, so it applies live.
pub(super) async fn h_torrents_toggle_sequential_download(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            handle.set_sequential_download(!handle.is_sequential_download());
            api.session().try_update_persistence_metadata(&handle).await;
        }
    }
    "Ok."
}

/// `torrents/toggleFirstLastPiecePrio` — flips the first/last-piece boost on
/// each torrent.
pub(super) async fn h_torrents_toggle_first_last_piece_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            handle.set_first_last_piece_priority(!handle.is_first_last_piece_priority());
            api.session().try_update_persistence_metadata(&handle).await;
        }
    }
    "Ok."
}

/// qBittorrent's `priority` field: the 1-based queue position among
/// unfinished torrents while queueing is enabled, 0 otherwise. Seeds report 0.
pub(super) fn queue_priorities(api: &Api) -> HashMap<usize, u32> {
    let session = api.session();
    if !session.queueing_enabled() {
        return HashMap::new();
    }
    session
        .queue_order()
        .into_iter()
        .filter(|&id| session.get(id.into()).is_some_and(|h| !h.stats().finished))
        .zip(1u32..)
        .collect()
}

pub(super) async fn h_torrents_recheck(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        if let Err(error) = api.api_torrent_action_recheck(idx).await {
            warn!(%error, "qbit compat: error rechecking torrent");
        }
    }
    "Ok."
}

pub(super) async fn h_torrents_reannounce(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            handle.reannounce();
        }
    }
    "Ok."
}

/// Relative paths of every file in the torrent, indexed by file id.
pub(super) fn torrent_file_paths(
    handle: &crate::torrent_state::ManagedTorrentHandle,
) -> Vec<PathBuf> {
    handle
        .with_metadata(|metadata| {
            metadata
                .file_infos
                .iter()
                .map(|fi| fi.relative_filename.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Map a rename engine error onto qBittorrent's 409 Conflict (used for
/// "torrent must be stopped", path collisions, invalid paths, etc.).
pub(super) fn rename_conflict(error: anyhow::Error) -> axum::response::Response {
    (StatusCode::CONFLICT, format!("{error:#}")).into_response()
}

#[derive(Deserialize, Default)]
pub(super) struct RenamePathForm {
    #[serde(default)]
    pub(super) hash: String,
    #[serde(default, alias = "oldPath")]
    pub(super) old_path: String,
    #[serde(default, alias = "newPath")]
    pub(super) new_path: String,
}

pub(super) async fn h_torrents_rename_file(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: RenamePathForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "hash is required");
    }
    if form.old_path.is_empty() || form.new_path.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "oldPath and newPath are required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let paths = torrent_file_paths(&handle);
    let old = PathBuf::from(&form.old_path);
    let file_id = match paths.iter().position(|p| *p == old) {
        Some(id) => id,
        None => return qbit_error(StatusCode::CONFLICT, "oldPath does not exist"),
    };
    match api
        .session()
        .rename_files(&handle, &[(file_id, PathBuf::from(&form.new_path))])
        .await
    {
        Ok(()) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => rename_conflict(error),
    }
}

pub(super) async fn h_torrents_rename_folder(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: RenamePathForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "hash is required");
    }
    if form.old_path.is_empty() || form.new_path.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "oldPath and newPath are required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    // Rename every file whose path is under the old folder prefix.
    let old_prefix = PathBuf::from(&form.old_path);
    let new_prefix = PathBuf::from(&form.new_path);
    let mut renames: Vec<(usize, PathBuf)> = Vec::new();
    for (id, path) in torrent_file_paths(&handle).into_iter().enumerate() {
        if let Ok(rest) = path.strip_prefix(&old_prefix) {
            renames.push((id, new_prefix.join(rest)));
        }
    }
    if renames.is_empty() {
        return qbit_error(StatusCode::CONFLICT, "no files under oldPath");
    }
    match api.session().rename_files(&handle, &renames).await {
        Ok(()) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => rename_conflict(error),
    }
}

#[derive(Deserialize, Default)]
pub(super) struct RenameForm {
    #[serde(default)]
    pub(super) hash: String,
    #[serde(default)]
    pub(super) name: String,
}

pub(super) async fn h_torrents_rename(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: RenameForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    if form.name.trim().is_empty() {
        return (StatusCode::CONFLICT, "name must not be empty");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    handle.set_display_name(Some(form.name));
    (StatusCode::OK, "Ok.")
}

#[derive(Deserialize, Default)]
pub(super) struct DeleteForm {
    #[serde(default)]
    pub(super) hashes: String,
    #[serde(default, alias = "deleteFiles")]
    pub(super) delete_files: Option<String>,
}

pub(super) async fn h_torrents_delete(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: DeleteForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);
    let delete_files = form
        .delete_files
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("true"));

    for idx in hashes {
        let result = if delete_files {
            api.api_torrent_action_delete(idx).await
        } else {
            api.api_torrent_action_forget(idx).await
        };
        if let Err(e) = result {
            warn!("qbit compat: error deleting torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

#[derive(Deserialize, Default)]
pub(super) struct SetCategoryForm {
    #[serde(default)]
    pub(super) hashes: String,
    #[serde(default)]
    pub(super) category: String,
}

/// `torrents/setCategory`. An empty category clears it; an unknown one is a
/// 409, as upstream.
pub(super) async fn h_torrents_set_category(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: SetCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let category = (!form.category.is_empty()).then_some(form.category);
    if let Some(name) = &category
        && !api.session().categories().exists(name)
    {
        return qbit_error(StatusCode::CONFLICT, "Incorrect category name");
    }
    for idx in resolve_hashes(api, &form.hashes) {
        if let Err(error) = api.api_set_torrent_category(idx, category.clone()).await {
            warn!(%error, "qbit compat: error setting torrent category");
        }
    }
    (StatusCode::OK, "Ok.").into_response()
}

#[derive(Deserialize, Default)]
pub(super) struct AddTrackersForm {
    #[serde(default)]
    pub(super) hash: String,
    #[serde(default)]
    pub(super) urls: String,
}

pub(super) async fn h_torrents_add_trackers(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: AddTrackersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let trackers: Vec<String> = form
        .urls
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if trackers.is_empty() {
        return (StatusCode::OK, "Ok.");
    }
    match api.api_torrent_action_add_trackers(idx, trackers).await {
        Ok(_) => (StatusCode::OK, "Ok."),
        Err(error) => {
            warn!(%error, "qbit compat: error adding trackers");
            (StatusCode::NOT_FOUND, "Not found")
        }
    }
}

#[derive(Deserialize, Default)]
pub(super) struct RemoveTrackersForm {
    #[serde(default)]
    pub(super) hash: String,
    /// Pipe-separated tracker URLs.
    #[serde(default)]
    pub(super) urls: String,
}

pub(super) async fn h_torrents_remove_trackers(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: RemoveTrackersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let urls: Vec<String> = form
        .urls
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    match api.session().remove_trackers(&handle, &urls).await {
        Ok(0) if !urls.is_empty() => qbit_error(StatusCode::CONFLICT, "All urls were not found"),
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => {
            warn!(%error, "qbit compat: error removing trackers");
            (StatusCode::OK, "Ok.").into_response()
        }
    }
}

#[derive(Deserialize, Default)]
pub(super) struct EditTrackerForm {
    #[serde(default)]
    pub(super) hash: String,
    #[serde(default, alias = "origUrl")]
    pub(super) orig_url: String,
    #[serde(default, alias = "newUrl")]
    pub(super) new_url: String,
}

pub(super) async fn h_torrents_edit_tracker(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: EditTrackerForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let Some(new_url) = url::Url::parse(&form.new_url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https" | "udp"))
    else {
        return qbit_error(StatusCode::BAD_REQUEST, "invalid newUrl");
    };
    {
        let configured = handle.shared().trackers.read();
        let orig_known = url::Url::parse(&form.orig_url).is_ok_and(|u| configured.contains(&u));
        if !orig_known {
            return qbit_error(StatusCode::CONFLICT, "origUrl not found");
        }
        if configured.contains(&new_url) {
            return qbit_error(StatusCode::CONFLICT, "newUrl already exists");
        }
    }
    match api
        .session()
        .edit_tracker(&handle, &form.orig_url, &form.new_url)
        .await
    {
        Ok(()) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => {
            warn!(%error, "qbit compat: error editing tracker");
            qbit_error(StatusCode::CONFLICT, "Failed")
        }
    }
}

#[derive(Deserialize, Default)]
pub(super) struct AddPeersForm {
    #[serde(default)]
    pub(super) hashes: String,
    #[serde(default)]
    pub(super) peers: String,
}

pub(super) async fn h_torrents_add_peers(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: AddPeersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let peers: Vec<std::net::SocketAddr> = form
        .peers
        .split('|')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    for idx in resolve_hashes(api, &form.hashes) {
        let Ok(handle) = api.mgr_handle(idx) else {
            continue;
        };
        let Some(live) = handle.live() else {
            continue;
        };
        for addr in &peers {
            let _ = live.add_peer_if_not_seen(*addr);
        }
    }
    "Ok."
}

#[derive(Deserialize, Default)]
pub(super) struct FilePrioForm {
    #[serde(default)]
    pub(super) hash: String,
    /// Pipe-separated file indices.
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) priority: u8,
}

pub(super) async fn h_torrents_file_prio(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: FilePrioForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let num_files = match api.api_torrent_details(idx) {
        Ok(details) => details.files.map(|f| f.len()).unwrap_or(0),
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };

    // qBittorrent priority 0 means "do not download"; anything else downloads.
    // We only model an include/exclude selection, so map onto only_files.
    let mut included: HashSet<usize> = match handle.only_files() {
        Some(files) => files.into_iter().collect(),
        None => (0..num_files).collect(),
    };
    let download = form.priority != 0;
    for id in form
        .id
        .split('|')
        .filter_map(|s| s.trim().parse::<usize>().ok())
    {
        if id >= num_files {
            return (StatusCode::CONFLICT, "Invalid file id");
        }
        if download {
            included.insert(id);
        } else {
            included.remove(&id);
        }
    }
    match api
        .api_torrent_action_update_only_files(idx, &included)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok."),
        Err(error) => {
            warn!(%error, "qbit compat: error setting file priority");
            (StatusCode::CONFLICT, "Failed")
        }
    }
}

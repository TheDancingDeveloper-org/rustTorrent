//! Torrents controller (`torrents/*`): listing and per-torrent queries.

use super::*;

// ---------------------------------------------------------------------------
// Torrent management endpoints
// ---------------------------------------------------------------------------

pub(super) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Map rtbit torrent state to qBittorrent state string.
///
/// We advertise WebAPI 2.11.3, so we emit the post-2.11 `stoppedDL`/`stoppedUP`
/// names (renamed from `pausedDL`/`pausedUP` in 2.11); clients that switch on
/// the advertised version expect these.
///
/// A torrent the queue paused reports `queuedDL`/`queuedUP` rather than
/// stopped, and a force-started live torrent reports `forcedDL`/`forcedUP`.
pub(super) fn map_state(
    state: TorrentStatsState,
    finished: bool,
    queued: bool,
    forced: bool,
) -> &'static str {
    match (state, finished) {
        (TorrentStatsState::Initializing, _) => "metaDL",
        (TorrentStatsState::Live, false) if forced => "forcedDL",
        (TorrentStatsState::Live, true) if forced => "forcedUP",
        (TorrentStatsState::Live, false) => "downloading",
        (TorrentStatsState::Live, true) => "uploading",
        (TorrentStatsState::Paused, false) if queued => "queuedDL",
        (TorrentStatsState::Paused, true) if queued => "queuedUP",
        (TorrentStatsState::Paused, false) => "stoppedDL",
        (TorrentStatsState::Paused, true) => "stoppedUP",
        (TorrentStatsState::Error, _) => "error",
    }
}

/// [`map_state`] plus the `missingFiles` flag: a stopped torrent whose
/// payload is missing or short on read-only storage cannot be resumed.
pub(super) fn qbit_state_for(
    stats: &crate::torrent_state::stats::TorrentStats,
    queued: bool,
    forced: bool,
) -> &'static str {
    if matches!(stats.state, TorrentStatsState::Paused)
        && stats.error.as_deref() == Some(crate::torrent_state::READ_ONLY_INCOMPLETE_PAYLOAD)
    {
        return "missingFiles";
    }
    map_state(stats.state, stats.finished, queued, forced)
}

#[derive(Deserialize, Default)]
pub(super) struct TorrentsInfoQuery {
    pub(super) filter: Option<String>,
    pub(super) category: Option<String>,
    pub(super) tag: Option<String>,
    pub(super) hashes: Option<String>,
    pub(super) sort: Option<String>,
    pub(super) reverse: Option<bool>,
    pub(super) limit: Option<usize>,
    pub(super) offset: Option<usize>,
}

/// Check if a torrent matches the qBit filter.
pub(super) fn matches_filter(
    filter: &str,
    qbit_state: &str,
    stats: &crate::torrent_state::stats::TorrentStats,
) -> bool {
    match filter {
        "all" => true,
        "downloading" => matches!(
            qbit_state,
            "downloading" | "metaDL" | "forcedDL" | "queuedDL"
        ),
        "seeding" => matches!(qbit_state, "uploading" | "forcedUP" | "queuedUP"),
        "completed" => stats.finished,
        // `paused` was renamed to `stopped` in 2.11; accept both spellings.
        "paused" | "stopped" => qbit_state == "stoppedDL" || qbit_state == "stoppedUP",
        "active" => is_active_state(qbit_state),
        "inactive" => !is_active_state(qbit_state),
        // `resumed` was renamed to `running` in 2.11; accept both spellings.
        "resumed" | "running" => !matches!(qbit_state, "stoppedDL" | "stoppedUP" | "missingFiles"),
        "stalled" | "stalled_uploading" | "stalled_downloading" => {
            matches!(stats.state, TorrentStatsState::Live)
                && stats
                    .live
                    .as_ref()
                    .map(|l| l.download_speed.mbps < 0.001 && l.upload_speed.mbps < 0.001)
                    .unwrap_or(true)
        }
        "errored" => matches!(qbit_state, "error" | "missingFiles"),
        _ => true,
    }
}

pub(super) fn is_active_state(qbit_state: &str) -> bool {
    matches!(
        qbit_state,
        "downloading" | "uploading" | "forcedDL" | "forcedUP"
    )
}

/// Build the qBittorrent `torrents/info` view of a single torrent. Shared by
/// `torrents/info` and `sync/maindata`.
pub(super) fn build_torrent_info(
    handle: &crate::torrent_state::ManagedTorrentHandle,
    stats: &crate::torrent_state::stats::TorrentStats,
    now: u64,
) -> QbitTorrentInfo {
    let info_hash = handle.shared().info_hash.as_string();
    let name = handle
        .name()
        .unwrap_or_else(|| format!("torrent_{}", handle.id()));
    let output_folder = qbit_save_path(handle);
    let content_path = qbit_content_path(handle, &name);
    let forced = handle.is_force_started();
    let qbit_state = qbit_state_for(stats, handle.is_queued(), forced);
    let limits = handle.rate_limits();
    let category = handle.shared().category.read().clone().unwrap_or_default();
    let share_limits = *handle.shared().share_limits.read();
    let ratio_limit = ratio_to_qbit(share_limits.max_ratio);
    let seeding_time_limit = minutes_to_qbit(share_limits.max_seeding_time);
    let inactive_seeding_time_limit = minutes_to_qbit(share_limits.max_inactive_seeding_time);

    let dl_speed = stats
        .live
        .as_ref()
        .map(|l| l.download_speed.as_bytes())
        .unwrap_or(0);
    let up_speed = stats
        .live
        .as_ref()
        .map(|l| l.upload_speed.as_bytes())
        .unwrap_or(0);

    let progress = if stats.total_bytes > 0 {
        stats.progress_bytes as f64 / stats.total_bytes as f64
    } else {
        0.0
    };

    let eta = stats
        .total_bytes
        .saturating_sub(stats.progress_bytes)
        .checked_div(dl_speed)
        .map(|seconds| i64::try_from(seconds.min(8_640_000)).unwrap_or(8_640_000))
        .unwrap_or(8_640_000);

    let num_seeds = stats
        .live
        .as_ref()
        .map(|l| l.snapshot.connected_seeders)
        .unwrap_or(0);
    let num_leechs = stats
        .live
        .as_ref()
        .map(|l| l.snapshot.connected_leechers)
        .unwrap_or(0);
    let added_on = handle.shared().added_on;
    let completion_on = handle
        .shared()
        .completion_on
        .load(std::sync::atomic::Ordering::Relaxed);
    let ratio = if stats.progress_bytes == 0 {
        0.0
    } else {
        stats.uploaded_bytes as f64 / stats.progress_bytes as f64
    };
    let time_active = now.saturating_sub(added_on);
    let seeding_time = completion_on
        .checked_sub(added_on)
        .map_or(0, |_| now.saturating_sub(completion_on));

    let tracker = handle
        .shared()
        .trackers
        .read()
        .iter()
        .next()
        .map(|u| u.to_string())
        .unwrap_or_default();

    let trackers_count = handle.shared().trackers.read().len();
    let magnet_uri = {
        let trackers = handle
            .shared()
            .trackers
            .read()
            .iter()
            .map(|u| u.to_string())
            .collect();
        let mut magnet =
            librtbit_core::magnet::Magnet::from_id20(handle.shared().info_hash, trackers, None);
        magnet.name = Some(name.clone());
        magnet.to_string()
    };

    QbitTorrentInfo {
        added_on,
        amount_left: stats.total_bytes.saturating_sub(stats.progress_bytes),
        auto_tmm: false,
        availability: -1,
        category,
        completed: stats.progress_bytes,
        completion_on: if completion_on > 0 {
            i64::try_from(completion_on).unwrap_or(i64::MAX)
        } else {
            -1
        },
        content_path,
        dl_limit: qbit_torrent_limit(limits.download_bps),
        dlspeed: dl_speed,
        download_path: String::new(),
        downloaded: stats.progress_bytes,
        downloaded_session: 0,
        eta,
        f_l_piece_prio: handle.is_first_last_piece_priority(),
        force_start: forced,
        hash: info_hash.clone(),
        inactive_seeding_time_limit,
        infohash_v1: info_hash,
        infohash_v2: String::new(),
        last_activity: if completion_on > 0 {
            completion_on
        } else {
            added_on
        },
        max_inactive_seeding_time: inactive_seeding_time_limit,
        magnet_uri,
        max_ratio: ratio_limit,
        max_seeding_time: seeding_time_limit,
        name,
        num_complete: num_seeds,
        num_incomplete: num_leechs,
        num_leechs,
        num_seeds,
        priority: 0,
        progress,
        ratio,
        ratio_limit,
        save_path: output_folder,
        seeding_time,
        seeding_time_limit,
        seen_complete: if completion_on > 0 {
            i64::try_from(completion_on).unwrap_or(i64::MAX)
        } else {
            -1
        },
        seq_dl: handle.is_sequential_download(),
        size: stats.total_bytes,
        state: qbit_state.to_string(),
        super_seeding: false,
        tags: handle
            .shared()
            .tags
            .read()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        time_active,
        total_size: stats.total_bytes,
        tracker,
        trackers_count,
        up_limit: qbit_torrent_limit(limits.upload_bps),
        uploaded: stats.uploaded_bytes,
        uploaded_session: stats.uploaded_bytes,
        upspeed: up_speed,
    }
}

/// Apply qBittorrent `offset`/`limit` pagination. An offset at or past the end
/// yields an empty list (not the whole list); a `limit` of 0 means "no limit".
pub(super) fn apply_offset_limit<T>(
    mut items: Vec<T>,
    offset: usize,
    limit: Option<usize>,
) -> Vec<T> {
    if offset >= items.len() {
        return Vec::new();
    }
    if offset > 0 {
        items = items.split_off(offset);
    }
    if let Some(limit) = limit
        && limit > 0
    {
        items.truncate(limit);
    }
    items
}

pub(super) async fn h_torrents_info(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<TorrentsInfoQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let now = now_unix();

    let hash_filter: Option<Vec<String>> = query
        .hashes
        .as_ref()
        .map(|h| h.split('|').map(|s| s.to_lowercase()).collect());

    let priorities = queue_priorities(api);
    let mut torrents: Vec<QbitTorrentInfo> = api.session().with_torrents(|iter| {
        iter.filter_map(|(_id, handle)| {
            let stats = handle.stats();
            let mut info = build_torrent_info(handle, &stats, now);
            info.priority = priorities.get(&handle.id()).copied().unwrap_or(0);

            // Filter by hash if specified.
            if let Some(ref hashes) = hash_filter
                && !hashes.contains(&info.hash)
            {
                return None;
            }

            // qBittorrent treats an empty category and "uncategorized" as the
            // uncategorized bucket; otherwise category matching is exact.
            if let Some(ref requested) = query.category
                && !matches_category(requested, &info.category)
            {
                return None;
            }

            // An empty tag query selects untagged torrents; otherwise the
            // torrent must carry that tag.
            if let Some(ref tag) = query.tag
                && !matches_tag(tag, &handle.shared().tags.read())
            {
                return None;
            }

            // Apply the state filter.
            if let Some(ref filter) = query.filter
                && !matches_filter(filter, &info.state, &stats)
            {
                return None;
            }

            Some(info)
        })
        .collect()
    });

    // Sort
    if let Some(ref sort_field) = query.sort {
        let reverse = query.reverse.unwrap_or(false);
        torrents.sort_by(|a, b| {
            let cmp = match sort_field.as_str() {
                "name" => a.name.cmp(&b.name),
                "size" | "total_size" => a.total_size.cmp(&b.total_size),
                "progress" => a
                    .progress
                    .partial_cmp(&b.progress)
                    .unwrap_or(std::cmp::Ordering::Equal),
                "dlspeed" => a.dlspeed.cmp(&b.dlspeed),
                "upspeed" => a.upspeed.cmp(&b.upspeed),
                "eta" => a.eta.cmp(&b.eta),
                "state" => a.state.cmp(&b.state),
                "added_on" => a.added_on.cmp(&b.added_on),
                "hash" => a.hash.cmp(&b.hash),
                "downloaded" => a.downloaded.cmp(&b.downloaded),
                "uploaded" => a.uploaded.cmp(&b.uploaded),
                "ratio" => a
                    .ratio
                    .partial_cmp(&b.ratio)
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => std::cmp::Ordering::Equal,
            };
            if reverse { cmp.reverse() } else { cmp }
        });
    }

    let torrents = apply_offset_limit(torrents, query.offset.unwrap_or(0), query.limit);
    axum::Json(torrents)
}

pub(super) fn matches_tag(requested: &str, tags: &BTreeSet<String>) -> bool {
    if requested.is_empty() {
        tags.is_empty()
    } else {
        tags.contains(requested)
    }
}

pub(super) fn matches_category(requested: &str, category: &str) -> bool {
    match requested {
        "all" => true,
        "uncategorized" | "" => category.is_empty(),
        value => category == value,
    }
}

#[derive(Deserialize)]
pub(super) struct HashQuery {
    pub(super) hash: String,
}

/// Descriptive fields from the top level of a `.torrent` file.
pub(super) struct TorrentFileMetadata {
    pub(super) comment: String,
    pub(super) created_by: String,
    /// Unix seconds; -1 when the file does not say.
    pub(super) creation_date: i64,
}

impl Default for TorrentFileMetadata {
    fn default() -> Self {
        Self {
            comment: String::new(),
            created_by: String::new(),
            creation_date: -1,
        }
    }
}

pub(super) fn torrent_file_metadata(torrent_bytes: &[u8]) -> TorrentFileMetadata {
    // Torrents added from a magnet have no outer dictionary to read.
    let Ok(meta) = librtbit_core::torrent_metainfo::torrent_from_bytes(torrent_bytes) else {
        return TorrentFileMetadata::default();
    };
    let text = |buf: Option<buffers::ByteBuf<'_>>| {
        buf.map(|b| String::from_utf8_lossy(b.as_ref()).into_owned())
            .unwrap_or_default()
    };
    TorrentFileMetadata {
        comment: text(meta.comment),
        created_by: text(meta.created_by),
        creation_date: meta
            .creation_date
            .and_then(|date| i64::try_from(date).ok())
            .unwrap_or(-1),
    }
}

pub(super) async fn h_torrents_properties(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let stats = handle.stats();
    let output_folder = qbit_save_path(&handle);
    let now = now_unix();
    let added_on = handle.shared().added_on;
    let completion_on = handle
        .shared()
        .completion_on
        .load(std::sync::atomic::Ordering::Relaxed);
    let ratio = if stats.progress_bytes == 0 {
        0.0
    } else {
        stats.uploaded_bytes as f64 / stats.progress_bytes as f64
    };

    let dl_speed = stats
        .live
        .as_ref()
        .map(|l| l.download_speed.as_bytes())
        .unwrap_or(0);
    let up_speed = stats
        .live
        .as_ref()
        .map(|l| l.upload_speed.as_bytes())
        .unwrap_or(0);

    let eta = stats
        .total_bytes
        .saturating_sub(stats.progress_bytes)
        .checked_div(dl_speed)
        .map(|seconds| i64::try_from(seconds).unwrap_or(8_640_000))
        .unwrap_or(8_640_000);

    let piece_size = handle
        .with_metadata(|m| m.info.lengths().default_piece_length() as u64)
        .unwrap_or(0);
    let pieces_num = handle
        .with_metadata(|m| m.info.lengths().total_pieces())
        .unwrap_or(0);
    let meta = handle
        .with_metadata(|m| torrent_file_metadata(&m.torrent_bytes))
        .unwrap_or_default();

    axum::Json(QbitTorrentProperties {
        save_path: output_folder,
        creation_date: meta.creation_date,
        piece_size,
        comment: meta.comment,
        total_wasted: 0,
        total_uploaded: stats.uploaded_bytes,
        total_uploaded_session: stats.uploaded_bytes,
        total_downloaded: stats.progress_bytes,
        total_downloaded_session: stats.progress_bytes,
        up_limit: qbit_torrent_limit(handle.rate_limits().upload_bps),
        dl_limit: qbit_torrent_limit(handle.rate_limits().download_bps),
        time_elapsed: now.saturating_sub(added_on),
        seeding_time: if completion_on > 0 {
            now.saturating_sub(completion_on)
        } else {
            0
        },
        nb_connections: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.peer_stats.live)
            .unwrap_or(0),
        nb_connections_limit: -1,
        share_ratio: ratio,
        addition_date: added_on,
        completion_date: if completion_on > 0 {
            i64::try_from(completion_on).unwrap_or(i64::MAX)
        } else {
            -1
        },
        created_by: meta.created_by,
        dl_speed_avg: dl_speed,
        dl_speed,
        eta,
        last_seen: if completion_on > 0 {
            completion_on
        } else {
            added_on
        },
        peers: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_leechers)
            .unwrap_or(0),
        peers_total: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_leechers)
            .unwrap_or(0),
        pieces_have: stats
            .progress_bytes
            .checked_div(piece_size)
            .map(|pieces| u32::try_from(pieces).unwrap_or(u32::MAX))
            .unwrap_or(0),
        pieces_num,
        reannounce: 0,
        seeds: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_seeders)
            .unwrap_or(0),
        seeds_total: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_seeders)
            .unwrap_or(0),
        total_size: stats.total_bytes,
        up_speed_avg: up_speed,
        up_speed,
    })
    .into_response()
}

pub(super) async fn h_torrents_files(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let details = match api.api_torrent_details(idx) {
        Ok(d) => d,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let stats = handle.stats();
    let is_seed = stats.finished;

    let details_name = details.name.clone();
    let details_files = details.files.unwrap_or_default();
    let qbit_root = handle.shared().options.output_folder_root.is_some();
    let is_multi_file = handle
        .with_metadata(|metadata| metadata.info.info().files.is_some())
        .unwrap_or(false);
    // Source per-file names from file_infos (which reflects renames) when its
    // count lines up with the details file list; otherwise fall back to the
    // metadata names to avoid any index skew (e.g. padding files).
    let rel_names: Vec<String> = handle
        .with_metadata(|metadata| {
            metadata
                .file_infos
                .iter()
                .map(|fi| fi.relative_filename.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let use_rel_names = rel_names.len() == details_files.len();
    // Inclusive [first, last] piece indices, as upstream reports them.
    let piece_ranges: Vec<[u32; 2]> = handle
        .with_metadata(|metadata| {
            metadata
                .file_infos
                .iter()
                .map(|fi| {
                    let r = &fi.piece_range;
                    [r.start, r.end.saturating_sub(1).max(r.start)]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let use_piece_ranges = piece_ranges.len() == details_files.len();
    let files: Vec<QbitFileInfo> = details_files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let file_progress = stats.file_progress.get(i).copied().unwrap_or(0);
            let progress = if f.length > 0 {
                file_progress as f64 / f.length as f64
            } else {
                1.0
            };
            let base_name = if use_rel_names {
                rel_names[i].as_str()
            } else {
                f.name.as_str()
            };
            let name = qbit_file_name(details_name.as_deref(), base_name, is_multi_file, qbit_root);
            QbitFileInfo {
                index: i,
                name,
                size: f.length,
                progress,
                priority: if f.included { 1 } else { 0 },
                is_seed,
                piece_range: if use_piece_ranges {
                    piece_ranges[i]
                } else {
                    [0, 0]
                },
                availability: if progress >= 1.0 { 1.0 } else { progress },
            }
        })
        .collect();

    axum::Json(files).into_response()
}

#[derive(Serialize)]
pub(super) struct QbitTrackerInfo {
    pub(super) url: String,
    /// qBittorrent tracker status: 0 disabled, 1 not contacted, 2 working,
    /// 3 updating, 4 not working.
    pub(super) status: u8,
    pub(super) tier: i32,
    pub(super) num_peers: i64,
    pub(super) num_seeds: i64,
    pub(super) num_leeches: i64,
    pub(super) num_downloaded: i64,
    pub(super) msg: String,
}

/// Map our tracker announce state to qBittorrent's integer status code.
pub(super) fn qbit_tracker_status(state: tracker_comms::TrackerAnnounceState) -> u8 {
    use tracker_comms::TrackerAnnounceState::*;
    match state {
        Disabled => 0,
        NotContacted => 1,
        Working => 2,
        Updating => 3,
        Error => 4,
    }
}

pub(super) async fn h_torrents_trackers(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let response = match api.api_tracker_status(idx) {
        Ok(r) => r,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };

    let trackers: Vec<QbitTrackerInfo> = response
        .trackers
        .into_iter()
        .map(|t| QbitTrackerInfo {
            status: qbit_tracker_status(t.state),
            tier: 0,
            num_peers: t.peers_returned.map_or(-1, i64::from),
            num_seeds: t.seeders.map_or(-1, i64::from),
            num_leeches: t.leechers.map_or(-1, i64::from),
            num_downloaded: -1,
            msg: t.last_error.unwrap_or_default(),
            url: t.url,
        })
        .collect();
    axum::Json(trackers).into_response()
}

pub(super) async fn h_torrents_piece_states(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let (bf, len) = match api.api_dump_haves(idx) {
        Ok(v) => v,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    // qBittorrent: 0 not downloaded, 1 downloading (requested), 2 downloaded.
    // We only distinguish have/not-have, so emit 0 or 2.
    let states: Vec<u8> = bf
        .iter()
        .take(len as usize)
        .map(|b| if *b { 2 } else { 0 })
        .collect();
    axum::Json(states).into_response()
}

pub(super) async fn h_torrents_piece_hashes(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let hashes: Option<Vec<String>> = handle
        .with_metadata(|m| {
            let info = m.info.info();
            let total = m.info.lengths().total_pieces();
            (0..total)
                .map(|p| {
                    info.get_hash(p).map(|h| {
                        h.iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    })
                })
                .collect::<Option<Vec<String>>>()
        })
        .ok()
        .flatten();
    match hashes {
        Some(hashes) => axum::Json(hashes).into_response(),
        // Metadata not yet available (magnet still resolving) or v2-only torrent.
        None => axum::Json(Vec::<String>::new()).into_response(),
    }
}

pub(super) async fn h_torrents_count(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let count = state
        .api_state
        .api
        .session()
        .with_torrents(|iter| iter.count());
    axum::Json(count)
}

/// `torrents/export` — return the raw `.torrent` file bytes for one torrent.
pub(super) async fn h_torrents_export(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    match handle.with_metadata(|meta| meta.torrent_bytes.clone()) {
        Ok(bytes) => ([("content-type", "application/x-bittorrent")], bytes).into_response(),
        // Metadata not resolved yet (magnet) — no file to export.
        Err(_) => qbit_error(StatusCode::CONFLICT, "Metadata not available"),
    }
}

#[derive(Serialize)]
pub(super) struct QbitWebSeed {
    pub(super) url: String,
}

pub(super) async fn h_torrents_webseeds(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return qbit_error(StatusCode::NOT_FOUND, "Not found"),
    };
    let seeds: Vec<QbitWebSeed> = handle
        .shared()
        .web_seed_urls
        .iter()
        .map(|url| QbitWebSeed { url: url.clone() })
        .collect();
    axum::Json(seeds).into_response()
}

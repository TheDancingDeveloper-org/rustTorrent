use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::Context;
use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use bytes::Bytes;
use http::{
    HeaderMap, HeaderName, HeaderValue, StatusCode,
    header::{CONTENT_DISPOSITION, CONTENT_TYPE},
};
use librtbit_core::magnet::Magnet;
use serde::{Deserialize, Serialize};

use super::ApiState;
#[cfg(feature = "swagger")]
use crate::api::{
    ApiAddTorrentResponse, EmptyJsonResponse, ReannounceResponse, RemoveTrackersResponse,
    TorrentDetailsResponse, TorrentListResponse,
};
use crate::{
    AddTorrent, ApiError, CreateTorrentOptions, SUPPORTED_SCHEMES,
    api::{ApiTorrentListOpts, Result, TorrentIdOrHash},
    api_error::WithStatusError,
    http_api::timeout::Timeout,
    http_api_types::TorrentAddQueryParams,
    torrent_state::peer::stats::snapshot::{PeerStatsFilter, PeerStatsFilterState},
    type_aliases::BF,
};

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents",
    params(ApiTorrentListOpts),
    responses(
        (status = 200, description = "List of torrents", body = TorrentListResponse)
    )
))]
pub async fn h_torrents_list(
    State(state): State<ApiState>,
    Query(opts): Query<ApiTorrentListOpts>,
) -> impl IntoResponse {
    axum::Json(state.api.api_torrent_list_ext(opts))
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents",
    params(TorrentAddQueryParams),
    request_body(content = Vec<u8>, description = "Torrent file bytes, magnet link, or URL"),
    responses(
        (status = 200, description = "Torrent added successfully", body = ApiAddTorrentResponse)
    )
))]
pub async fn h_torrents_post(
    State(state): State<ApiState>,
    Query(params): Query<TorrentAddQueryParams>,
    Timeout(timeout): Timeout<600_000, 3_600_000>,
    data: Bytes,
) -> Result<impl IntoResponse> {
    let is_url = params.is_url;
    let opts = params.into_add_torrent_options();
    let data = data.to_vec();
    let maybe_magnet = |data: &[u8]| -> bool {
        std::str::from_utf8(data)
            .ok()
            .and_then(|s| Magnet::parse(s).ok())
            .is_some()
    };
    let add = match is_url {
        Some(true) => AddTorrent::Url(
            String::from_utf8(data)
                .context("invalid utf-8 for passed URL")?
                .into(),
        ),
        Some(false) => AddTorrent::TorrentFileBytes(data.into()),

        // Guess the format.
        None if SUPPORTED_SCHEMES
            .iter()
            .any(|s| data.starts_with(s.as_bytes()))
            || maybe_magnet(&data) =>
        {
            AddTorrent::Url(
                String::from_utf8(data)
                    .context("invalid utf-8 for passed URL")?
                    .into(),
            )
        }
        _ => AddTorrent::TorrentFileBytes(data.into()),
    };
    tokio::time::timeout(timeout, state.api.api_add_torrent(add, Some(opts)))
        .await
        .context("timeout")?
        .map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/{id}",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Torrent details", body = TorrentDetailsResponse)
    )
))]
pub async fn h_torrent_details(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state.api.api_torrent_details(idx).map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/{id}/haves",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Bitfield of pieces (SVG or binary depending on Accept header)")
    )
))]
pub async fn h_torrent_haves(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    fn generate_svg(bits: &BF, len: u32) -> String {
        if len == 0 {
            return r#"<svg width="100%" height="100" xmlns="http://www.w3.org/2000/svg"></svg>"#
                .to_string();
        }

        const HAVE_COLOR: &str = "#22c55e";
        const MISSING_COLOR: &str = "#374151";

        let bit_width = 100.0 / len as f64;
        let mut svg_segments = String::new();

        let mut bits_iter = bits.iter().map(|b| *b).enumerate().peekable();

        while let Some((i, value)) = bits_iter.next() {
            let mut count = 1;

            // Peek ahead to find how many subsequent bits have the same value
            while let Some((_, next_value)) = bits_iter.peek() {
                if *next_value == value {
                    count += 1;
                    bits_iter.next();
                } else {
                    break;
                }
            }

            let color = if value { HAVE_COLOR } else { MISSING_COLOR };
            let x_pos = i as f64 * bit_width;
            let segment_width = count as f64 * bit_width;

            svg_segments.push_str(&format!(
                r#"<rect x="{:.4}%" y="0" width="{:.4}%" height="100%" fill="{}" />"#,
                x_pos, segment_width, color
            ));
        }

        format!(
            r#"<svg width="100%" height="20" viewBox="0 0 100 100" preserveAspectRatio="none" xmlns="http://www.w3.org/2000/svg">
                {}
            </svg>"#,
            svg_segments
        )
    }

    let (bf, len) = state.api.api_dump_haves(idx)?;

    // Check if binary format is requested
    let wants_binary = headers
        .get(http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("application/octet-stream"));

    if wants_binary {
        let bytes = bf.into_boxed_slice();
        Ok((
            [
                (
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/octet-stream"),
                ),
                (
                    HeaderName::from_static("x-bitfield-len"),
                    HeaderValue::from_str(&len.to_string()).unwrap(),
                ),
            ],
            bytes,
        )
            .into_response())
    } else {
        let svg = generate_svg(&bf, len);
        Ok((
            [(CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"))],
            svg,
        )
            .into_response())
    }
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/{id}/stats",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Torrent stats (v0, deprecated)")
    )
))]
pub async fn h_torrent_stats_v0(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state.api.api_stats_v0(idx).map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/{id}/stats/v1",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Torrent stats (current)")
    )
))]
pub async fn h_torrent_stats_v1(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state.api.api_stats_v1(idx).map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/{id}/trackers",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Per-tracker announce status (seeders/leechers per tracker)")
    )
))]
pub async fn h_torrent_trackers(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state.api.api_tracker_status(idx).map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/{id}/peer_stats",
    params(
        ("id" = String, Path, description = "Torrent ID or info hash"),
        ("state" = Option<String>, Query, description = "Filter peer state (all or live)")
    ),
    responses(
        (status = 200, description = "Per-peer statistics")
    )
))]
pub async fn h_peer_stats(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    Query(filter): Query<PeerStatsFilter>,
) -> Result<impl IntoResponse> {
    state.api.api_peer_stats(idx, filter).map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/pause",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Torrent paused", body = EmptyJsonResponse)
    )
))]
pub async fn h_torrent_action_pause(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_pause(idx)
        .await
        .map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/start",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Torrent started", body = EmptyJsonResponse)
    )
))]
#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/queue/{action}",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Torrent moved in the queue", body = EmptyJsonResponse)
    )
))]
async fn h_torrent_queue_move(
    state: ApiState,
    idx: TorrentIdOrHash,
    mv: crate::session::scheduler::QueueMove,
) -> Result<impl IntoResponse> {
    let id = state.api.mgr_handle(idx)?.id();
    state.api.session().move_in_queue(&[id], mv).await;
    Ok(axum::Json(crate::api::EmptyJsonResponse {}))
}

pub async fn h_torrent_queue_top(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    h_torrent_queue_move(state, idx, crate::session::scheduler::QueueMove::Top).await
}

pub async fn h_torrent_queue_up(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    h_torrent_queue_move(state, idx, crate::session::scheduler::QueueMove::Up).await
}

pub async fn h_torrent_queue_down(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    h_torrent_queue_move(state, idx, crate::session::scheduler::QueueMove::Down).await
}

pub async fn h_torrent_queue_bottom(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    h_torrent_queue_move(state, idx, crate::session::scheduler::QueueMove::Bottom).await
}

pub async fn h_torrent_action_start(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_start(idx)
        .await
        .map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/recheck",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Full integrity check started", body = EmptyJsonResponse)
    )
))]
pub async fn h_torrent_action_recheck(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_recheck(idx)
        .await
        .map(axum::Json)
}

#[derive(Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct AddTrackersRequest {
    trackers: Vec<String>,
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/trackers",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    request_body(content = AddTrackersRequest, description = "Tracker announce URLs to merge"),
    responses(
        (status = 200, description = "Trackers attached", body = EmptyJsonResponse)
    )
))]
pub async fn h_torrent_action_add_trackers(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    axum::Json(req): axum::Json<AddTrackersRequest>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_add_trackers(idx, req.trackers)
        .await
        .map(axum::Json)
}

#[derive(Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct RemoveTrackersRequest {
    trackers: Vec<String>,
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/trackers/remove",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    request_body(content = RemoveTrackersRequest, description = "Tracker announce URLs to remove"),
    responses(
        (status = 200, description = "Trackers removed; their announce loops stop", body = RemoveTrackersResponse)
    )
))]
pub async fn h_torrent_action_remove_trackers(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    axum::Json(req): axum::Json<RemoveTrackersRequest>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_remove_trackers(idx, req.trackers)
        .await
        .map(axum::Json)
}

#[derive(Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct EditTrackerRequest {
    orig_url: String,
    new_url: String,
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/trackers/edit",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    request_body(content = EditTrackerRequest, description = "Tracker URL to replace and its replacement"),
    responses(
        (status = 200, description = "Tracker replaced", body = EmptyJsonResponse)
    )
))]
pub async fn h_torrent_action_edit_tracker(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    axum::Json(req): axum::Json<EditTrackerRequest>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_edit_tracker(idx, req.orig_url, req.new_url)
        .await
        .map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/reannounce",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Announce loops woken for an immediate announce", body = ReannounceResponse)
    )
))]
pub async fn h_torrent_action_reannounce(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state.api.api_torrent_action_reannounce(idx).map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/forget",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Torrent forgotten (files kept)", body = EmptyJsonResponse)
    )
))]
pub async fn h_torrent_action_forget(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_forget(idx)
        .await
        .map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/delete",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Torrent deleted (files removed)", body = EmptyJsonResponse)
    )
))]
pub async fn h_torrent_action_delete(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_delete(idx)
        .await
        .map(axum::Json)
}

#[derive(Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct UpdateOnlyFilesRequest {
    only_files: Vec<usize>,
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/update_only_files",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    request_body(content = UpdateOnlyFilesRequest, description = "File indices to download"),
    responses(
        (status = 200, description = "File selection updated", body = EmptyJsonResponse)
    )
))]
pub async fn h_torrent_action_update_only_files(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    axum::Json(req): axum::Json<UpdateOnlyFilesRequest>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_torrent_action_update_only_files(idx, &req.only_files.into_iter().collect())
        .await
        .map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/stats",
    responses(
        (status = 200, description = "Global session stats")
    )
))]
pub async fn h_session_stats(State(state): State<ApiState>) -> impl IntoResponse {
    axum::Json(state.api.api_session_stats())
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/{id}/peer_stats/prometheus",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Per-peer stats in Prometheus format", body = String)
    )
))]
pub async fn h_peer_stats_prometheus(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    let handle = state.api.mgr_handle(idx)?;

    let live = handle
        .live()
        .with_status_error(StatusCode::PRECONDITION_FAILED, "torrent is not live")?;

    let peer_stats = live.per_peer_stats_snapshot(PeerStatsFilter {
        state: PeerStatsFilterState::Live,
    });

    let mut buf = String::new();

    const NAME: &str = "rtbit_peer_fetched_bytes";

    use core::fmt::Write;
    writeln!(&mut buf, "# TYPE {NAME} counter").unwrap();
    for (addr, stats) in peer_stats.peers.iter() {
        // Filter out useless peers that never sent us much.
        const THRESHOLD: u64 = 1024 * 1024;
        if stats.counters.fetched_bytes >= THRESHOLD {
            writeln!(
                &mut buf,
                "{NAME}{{addr=\"{addr}\"}} {}",
                stats.counters.fetched_bytes - THRESHOLD
            )
            .unwrap();
        }
    }

    Ok(buf)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/{id}/metadata",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    responses(
        (status = 200, description = "Download .torrent file", content_type = "application/x-bittorrent")
    )
))]
pub async fn h_metadata(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    let handle = state.api.mgr_handle(idx)?;

    let (filename, bytes) = handle
        .with_metadata(|meta| {
            (
                meta.info
                    .name_or_else(|| format!("torrent_{idx}"))
                    .into_owned(),
                meta.torrent_bytes.clone(),
            )
        })
        .map_err(ApiError::from)?;

    Ok((
        [(
            http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}.torrent\""),
        )],
        bytes,
    ))
}

#[derive(Serialize)]
struct AddPeersResult {
    added: usize,
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/{id}/add_peers",
    params(("id" = String, Path, description = "Torrent ID or info hash")),
    request_body(content = String, description = "Newline-delimited peer socket addresses"),
    responses(
        (status = 200, description = "Peers added")
    )
))]
pub async fn h_add_peers(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let handle = state.api.mgr_handle(idx)?;
    let live = handle.live().ok_or(crate::Error::TorrentIsNotLive)?;

    let body =
        std::str::from_utf8(&body).with_status_error(StatusCode::BAD_REQUEST, "invalid utf-8")?;

    let addrs = body
        .split('\n')
        .filter_map(|s| SocketAddr::from_str(s).ok());

    let mut count = 0;
    for addr in addrs {
        if live.add_peer_if_not_seen(addr)? {
            count += 1;
        }
    }

    Ok(axum::Json(AddPeersResult { added: count }))
}

#[derive(Default, Deserialize, Debug)]
enum CreateTorrentOutput {
    #[default]
    #[serde(rename = "magnet")]
    Magnet,
    #[serde(rename = "torrent")]
    Torrent,
}

#[derive(Default, Deserialize, Debug)]
pub struct HttpCreateTorrentOptions {
    #[serde(default)]
    output: CreateTorrentOutput,
    #[serde(default)]
    trackers: Vec<String>,
    name: Option<String>,
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/create",
    request_body(content = String, description = "Local folder path to create torrent from"),
    responses(
        (status = 200, description = "Torrent created and seeding")
    )
))]
pub async fn h_create_torrent(
    State(state): State<ApiState>,
    axum_extra::extract::Query(opts): axum_extra::extract::Query<HttpCreateTorrentOptions>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    if !state.opts.allow_create {
        return Err((
            StatusCode::FORBIDDEN,
            "creating torrents not allowed. Enable through CLI options",
        )
            .into());
    }

    let path = std::path::Path::new(
        std::str::from_utf8(body.as_ref())
            .with_status_error(StatusCode::BAD_REQUEST, "invalid utf-8")?,
    );

    let create_opts = CreateTorrentOptions {
        name: opts.name.as_deref(),
        trackers: opts.trackers,
        ..Default::default()
    };

    let (torrent, handle) = state
        .api
        .session()
        .create_and_serve_torrent(path, create_opts)
        .await?;

    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&handle.id().to_string()) {
        headers.insert("torrent-id", v);
    }
    if let Ok(v) = HeaderValue::from_str(&torrent.info_hash().as_string()) {
        headers.insert("torrent-info-hash", v);
    }

    match opts.output {
        CreateTorrentOutput::Magnet => {
            let magnet = torrent.as_magnet();
            Ok(magnet.to_string().into_response())
        }
        CreateTorrentOutput::Torrent => {
            let name = torrent
                .as_info()
                .info
                .data
                .name
                .as_ref()
                .map(|n| String::from_utf8_lossy(n.as_ref()))
                .unwrap_or("torrent".into());

            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/x-bittorrent"),
            );

            if let Ok(h) =
                HeaderValue::from_str(&format!("attachment; filename=\"{name}.torrent\""))
            {
                headers.insert(CONTENT_DISPOSITION, h);
            }

            Ok((headers, torrent.as_bytes()?).into_response())
        }
    }
}

#[derive(Deserialize)]
pub struct CreateOrEditCategoryRequest {
    pub name: String,
    pub save_path: Option<PathBuf>,
}

#[derive(Deserialize)]
pub struct SetTorrentCategoryRequest {
    pub category: Option<String>,
}

pub async fn h_list_categories(State(state): State<ApiState>) -> impl IntoResponse {
    axum::Json(state.api.api_list_categories())
}

pub async fn h_create_or_edit_category(
    State(state): State<ApiState>,
    axum::Json(req): axum::Json<CreateOrEditCategoryRequest>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_create_or_edit_category(req.name, req.save_path)
        .await
        .map(axum::Json)
}

pub async fn h_delete_category(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse> {
    state.api.api_remove_category(&name).await.map(axum::Json)
}

pub async fn h_set_torrent_category(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    axum::Json(req): axum::Json<SetTorrentCategoryRequest>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_set_torrent_category(idx, req.category)
        .await
        .map(axum::Json)
}

pub async fn h_get_torrent_rate_limits(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
) -> Result<impl IntoResponse> {
    state.api.api_torrent_rate_limits(idx).map(axum::Json)
}

pub async fn h_set_torrent_rate_limits(
    State(state): State<ApiState>,
    Path(idx): Path<TorrentIdOrHash>,
    axum::Json(limits): axum::Json<crate::session_persistence::TorrentRateLimits>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_set_torrent_rate_limits(idx, limits)
        .await
        .map(axum::Json)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Context;
    use axum::extract::{Path, State};
    use tempfile::TempDir;

    use super::{
        AddTrackersRequest, EditTrackerRequest, RemoveTrackersRequest,
        h_torrent_action_add_trackers, h_torrent_action_edit_tracker, h_torrent_action_recheck,
        h_torrent_action_remove_trackers, h_torrent_queue_bottom, h_torrent_queue_down,
        h_torrent_queue_top, h_torrent_queue_up,
    };
    use crate::{
        AddTorrent, AddTorrentOptions, Api, CreateTorrentOptions, Session, SessionOptions,
        create_torrent,
        http_api::{HttpApi, HttpApiOptions},
        spawn_utils::BlockingSpawner,
        torrent_state::{ManagedTorrentHandle, TorrentStatsState},
    };

    async fn torrent_api() -> anyhow::Result<(Arc<HttpApi>, ManagedTorrentHandle, TempDir)> {
        let payload = TempDir::with_prefix("rtbit_http_action_payload")?;
        std::fs::write(payload.path().join("payload.bin"), vec![0x5e; 32 * 1024])?;
        let torrent = create_torrent(
            payload.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await?
        .as_bytes()?
        .to_vec();
        let session = Session::new_with_opts(
            payload.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_trackers: true,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        )
        .await?;
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(payload.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await?
            .into_handle()
            .context("expected torrent handle")?;
        handle.wait_until_initialized().await?;
        let api = Api::new(
            session,
            None,
            #[cfg(feature = "tracing-subscriber-utils")]
            None,
        );
        Ok((
            Arc::new(HttpApi::new(api, Some(HttpApiOptions::default()))),
            handle,
            payload,
        ))
    }

    #[tokio::test]
    async fn add_trackers_handler_merges_urls() -> anyhow::Result<()> {
        let (state, handle, _payload) = torrent_api().await?;
        h_torrent_action_add_trackers(
            State(state),
            Path(handle.id().into()),
            axum::Json(AddTrackersRequest {
                trackers: vec!["https://tracker.example.test/announce".to_string()],
            }),
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(handle.shared().trackers.read().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn edit_and_remove_tracker_handlers_mutate_list() -> anyhow::Result<()> {
        let (state, handle, _payload) = torrent_api().await?;
        let orig = "https://tracker.example.test/announce".to_string();
        let new = "udp://tracker.example.test:6969/announce".to_string();
        h_torrent_action_add_trackers(
            State(state.clone()),
            Path(handle.id().into()),
            axum::Json(AddTrackersRequest {
                trackers: vec![orig.clone()],
            }),
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;

        // A bad replacement is rejected and leaves the original in place.
        let bad = h_torrent_action_edit_tracker(
            State(state.clone()),
            Path(handle.id().into()),
            axum::Json(EditTrackerRequest {
                orig_url: orig.clone(),
                new_url: "ftp://nope.example.test/".to_string(),
            }),
        )
        .await;
        assert!(bad.is_err());
        assert!(
            handle
                .shared()
                .trackers
                .read()
                .iter()
                .any(|t| t.as_str() == orig)
        );

        h_torrent_action_edit_tracker(
            State(state.clone()),
            Path(handle.id().into()),
            axum::Json(EditTrackerRequest {
                orig_url: orig.clone(),
                new_url: new.clone(),
            }),
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        {
            let trackers = handle.shared().trackers.read();
            assert_eq!(trackers.len(), 1);
            assert!(trackers.iter().any(|t| t.as_str() == new));
        }
        let urls: Vec<String> = handle
            .shared()
            .tracker_status
            .snapshot()
            .into_iter()
            .map(|t| t.url)
            .collect();
        assert_eq!(urls, vec![new.clone()]);

        h_torrent_action_remove_trackers(
            State(state),
            Path(handle.id().into()),
            axum::Json(RemoveTrackersRequest {
                trackers: vec![new, orig],
            }),
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert!(handle.shared().trackers.read().is_empty());
        assert!(handle.shared().tracker_status.snapshot().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn recheck_handler_starts_full_check() -> anyhow::Result<()> {
        let (state, handle, _payload) = torrent_api().await?;
        h_torrent_action_recheck(State(state), Path(handle.id().into()))
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        handle.wait_until_initialized().await?;
        assert!(matches!(handle.stats().state, TorrentStatsState::Paused));
        Ok(())
    }

    #[tokio::test]
    async fn queue_move_handlers_reorder() -> anyhow::Result<()> {
        let (state, first, payload) = torrent_api().await?;
        let session = state.api.session().clone();
        let torrent = create_torrent(
            payload.path(),
            CreateTorrentOptions {
                name: Some("second"),
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await?
        .as_bytes()?
        .to_vec();
        let second = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(payload.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await?
            .into_handle()
            .context("expected second torrent")?;
        second.wait_until_initialized().await?;

        let order = || session.queue_order();
        let first_id = first.id();
        let second_id = second.id();
        assert_eq!(order(), vec![first_id, second_id]);

        let move_err = |error| anyhow::anyhow!("{error:?}");
        h_torrent_queue_top(State(state.clone()), Path(second_id.into()))
            .await
            .map_err(move_err)?;
        assert_eq!(order(), vec![second_id, first_id]);
        h_torrent_queue_down(State(state.clone()), Path(second_id.into()))
            .await
            .map_err(move_err)?;
        assert_eq!(order(), vec![first_id, second_id]);
        h_torrent_queue_up(State(state.clone()), Path(second_id.into()))
            .await
            .map_err(move_err)?;
        assert_eq!(order(), vec![second_id, first_id]);
        h_torrent_queue_bottom(State(state), Path(second_id.into()))
            .await
            .map_err(move_err)?;
        assert_eq!(order(), vec![first_id, second_id]);
        Ok(())
    }
}

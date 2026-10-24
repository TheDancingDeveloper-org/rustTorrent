pub(crate) mod configure;
pub(crate) mod dht;
pub(crate) mod healthz;
pub(crate) mod indexarr;
pub(crate) mod logging;
pub(crate) mod other;
pub(crate) mod playlist;
pub(crate) mod qbit;
#[cfg(test)]
mod qbit_parity;
#[cfg(test)]
mod qbit_replay;
mod qbit_sync;
pub(crate) mod rss;
pub(crate) mod speed;
pub(crate) mod streaming;
pub(crate) mod torrents;

use std::sync::Arc;

#[cfg(feature = "webui")]
use axum::response::Redirect;

use axum::{
    Router,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use http::request::Parts;

use super::HttpApi;
type ApiState = Arc<HttpApi>;

#[allow(unused_variables)]
async fn h_api_root(parts: Parts) -> impl IntoResponse {
    // If browser, and webui enabled, redirect to web
    #[cfg(feature = "webui")]
    {
        if parts
            .headers
            .get("Accept")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| h.contains("text/html"))
        {
            return Redirect::temporary("./web/").into_response();
        }
    }

    let json = serde_json::json!({
        "apis": {
            "GET /": "list all available APIs",
            "GET /dht/stats": "DHT stats",
            "GET /dht/table": "DHT routing table",
            "GET /torrents": "List torrents",
            "GET /torrents/playlist": "Generate M3U8 playlist for all files in all torrents",
            "GET /stats": "Global session stats",
            "GET /metrics": "Prometheus metrics",
            "GET /stream_logs": "Continuously stream logs",
            "GET /web/": "Web UI",
            "GET /torrents/playlist": "Playlist for supported players",
            "GET /torrents/{id_or_infohash}": "Torrent details",
            "GET /torrents/{id_or_infohash}/metadata": "Download the corresponding torrent file",
            "GET /torrents/{id_or_infohash}/haves": "The bitfield of have pieces",
            "GET /torrents/{id_or_infohash}/playlist": "Generate M3U8 playlist for this torrent",
            "GET /torrents/{id_or_infohash}/stats/v1": "Torrent stats",
            "GET /torrents/{id_or_infohash}/trackers": "Per-tracker announce status",
            "GET /torrents/{id_or_infohash}/peer_stats": "Per peer stats",
            "GET /torrents/{id_or_infohash}/peer_stats/prometheus": "Per peer stats in prometheus format",
            "GET /torrents/{id_or_infohash}/stream/{file_idx}": "Stream a file. Accepts Range header to seek.",
            "GET /torrents/{id_or_infohash}/playlist": "Playlist for supported players",
            "POST /torrents": "Add a torrent here. magnet: or http:// or a local file.",
            "POST /torrents/create": "Create a torrent and start seeding. Body should be a local folder",
            "POST /torrents/resolve_magnet": "Resolve a magnet to torrent file bytes",
            "GET /torrents/seed_limits": "Global seed ratio and time limits",
            "POST /torrents/seed_limits": "Set global seed limits: {\"ratio_limit\": n, \"time_limit_secs\": n}, null = off",
            "GET /torrents/{id_or_infohash}/seed_limits": "Per-torrent seed ratio and time limits",
            "POST /torrents/{id_or_infohash}/seed_limits": "Set per-torrent seed limits",
            "GET /torrents/{id_or_infohash}/limits": "Per-torrent speed limits",
            "POST /torrents/{id_or_infohash}/limits": "Set per-torrent speed limits: {\"download_bps\": n, \"upload_bps\": n}, omitted = unlimited",
            "POST /torrents/{id_or_infohash}/pause": "Pause torrent",
            "POST /torrents/{id_or_infohash}/start": "Resume torrent",
            "POST /torrents/{id_or_infohash}/recheck": "Force a full integrity check",
            "POST /torrents/{id_or_infohash}/trackers": "Attach tracker announce URLs",
            "POST /torrents/{id_or_infohash}/forget": "Forget about the torrent, keep the files",
            "POST /torrents/{id_or_infohash}/delete": "Forget about the torrent, remove the files",
            "POST /torrents/{id_or_infohash}/add_peers": "Add peers (newline-delimited)",
            "POST /torrents/{id_or_infohash}/update_only_files": "Change the selection of files to download. You need to POST json of the following form {\"only_files\": [0, 1, 2]}",
            "POST /rust_log": "Set RUST_LOG to this post launch (for debugging)",
        },
        "server": "rtbit",
        "version": env!("CARGO_PKG_VERSION"),
    });

    ([("Content-Type", "application/json")], axum::Json(json)).into_response()
}

pub fn make_api_router(state: ApiState) -> Router {
    let mut api_router = Router::new()
        .route("/", get(h_api_root))
        .route("/healthz", get(healthz::h_healthz))
        .route("/rust_log", post(logging::h_set_rust_log))
        .route("/dht/stats", get(dht::h_dht_stats))
        .route("/dht/table", get(dht::h_dht_table))
        .route("/stats", get(torrents::h_session_stats))
        .route("/torrents", get(torrents::h_torrents_list))
        .route("/torrents/{id}", get(torrents::h_torrent_details))
        .route("/torrents/{id}/haves", get(torrents::h_torrent_haves))
        .route("/torrents/{id}/metadata", get(torrents::h_metadata))
        .route("/torrents/{id}/stats", get(torrents::h_torrent_stats_v0))
        .route("/torrents/{id}/stats/v1", get(torrents::h_torrent_stats_v1))
        .route("/torrents/{id}/trackers", get(torrents::h_torrent_trackers))
        .route("/torrents/{id}/peer_stats", get(torrents::h_peer_stats))
        .route(
            "/torrents/{id}/peer_stats/prometheus",
            get(torrents::h_peer_stats_prometheus),
        )
        .route("/torrents/{id}/playlist", get(playlist::h_torrent_playlist))
        .route("/torrents/playlist", get(playlist::h_global_playlist))
        .route("/torrents/resolve_magnet", post(other::h_resolve_magnet))
        .route(
            "/torrents/{id}/stream/{file_id}",
            get(streaming::h_torrent_stream_file),
        )
        .route(
            "/torrents/{id}/stream/{file_id}/{*filename}",
            get(streaming::h_torrent_stream_file),
        )
        .route("/torrents/seed_limits", get(torrents::h_get_seed_limits))
        .route("/torrents/limits", get(configure::h_get_session_ratelimits))
        .route(
            "/torrents/{id}/seed_limits",
            get(torrents::h_get_torrent_seed_limits),
        )
        .route(
            "/torrents/{id}/limits",
            get(torrents::h_get_torrent_rate_limits),
        )
        .route("/speed/alt", get(speed::h_get_alt_speed))
        .route("/speed/schedule", get(speed::h_get_speed_schedule))
        .route("/torrents/categories", get(torrents::h_list_categories))
        .route("/session/folders", get(configure::h_get_folders))
        .route("/session/downloads", get(configure::h_get_downloads))
        .route("/fs/browse", get(configure::h_browse_directory));

    #[cfg(feature = "tracing-subscriber-utils")]
    {
        api_router = api_router.route("/stream_logs", get(logging::h_stream_logs));
    }

    if !state.opts.read_only {
        api_router = api_router
            .route("/torrents", post(torrents::h_torrents_post))
            .route("/torrents/seed_limits", post(torrents::h_set_seed_limits))
            .route(
                "/torrents/limits",
                post(configure::h_update_session_ratelimits),
            )
            .route(
                "/torrents/{id}/pause",
                post(torrents::h_torrent_action_pause),
            )
            .route(
                "/torrents/{id}/start",
                post(torrents::h_torrent_action_start),
            )
            .route(
                "/torrents/{id}/recheck",
                post(torrents::h_torrent_action_recheck),
            )
            .route(
                "/torrents/{id}/trackers",
                post(torrents::h_torrent_action_add_trackers),
            )
            .route(
                "/torrents/{id}/trackers/remove",
                post(torrents::h_torrent_action_remove_trackers),
            )
            .route(
                "/torrents/{id}/trackers/edit",
                post(torrents::h_torrent_action_edit_tracker),
            )
            .route(
                "/torrents/{id}/reannounce",
                post(torrents::h_torrent_action_reannounce),
            )
            .route(
                "/torrents/{id}/forget",
                post(torrents::h_torrent_action_forget),
            )
            .route(
                "/torrents/{id}/delete",
                post(torrents::h_torrent_action_delete),
            )
            .route(
                "/torrents/{id}/update_only_files",
                post(torrents::h_torrent_action_update_only_files),
            )
            .route("/torrents/{id}/add_peers", post(torrents::h_add_peers))
            .route("/torrents/create", post(torrents::h_create_torrent))
            .route(
                "/torrents/categories",
                post(torrents::h_create_or_edit_category),
            )
            .route(
                "/torrents/categories/{name}",
                delete(torrents::h_delete_category),
            )
            .route(
                "/torrents/{id}/set_category",
                post(torrents::h_set_torrent_category),
            )
            .route(
                "/torrents/{id}/seed_limits",
                post(torrents::h_set_torrent_seed_limits),
            )
            .route(
                "/torrents/{id}/limits",
                post(torrents::h_set_torrent_rate_limits),
            )
            .route(
                "/torrents/{id}/queue/top",
                post(torrents::h_torrent_queue_top),
            )
            .route(
                "/torrents/{id}/queue/up",
                post(torrents::h_torrent_queue_up),
            )
            .route(
                "/torrents/{id}/queue/down",
                post(torrents::h_torrent_queue_down),
            )
            .route(
                "/torrents/{id}/queue/bottom",
                post(torrents::h_torrent_queue_bottom),
            )
            .route("/session/folders", post(configure::h_set_folders))
            .route("/session/downloads", post(configure::h_set_downloads))
            .route("/speed/alt", post(speed::h_toggle_alt_speed))
            .route("/speed/alt/config", post(speed::h_set_alt_speed_config))
            .route("/speed/schedule", post(speed::h_set_speed_schedule));
    }

    // Indexarr integration proxy endpoints
    api_router = api_router
        .route("/indexarr/status", get(indexarr::h_indexarr_status))
        .route("/indexarr/search", get(indexarr::h_indexarr_search))
        .route("/indexarr/recent", get(indexarr::h_indexarr_recent))
        .route("/indexarr/trending", get(indexarr::h_indexarr_trending))
        .route(
            "/indexarr/torrent/{info_hash}",
            get(indexarr::h_indexarr_torrent_detail),
        )
        .route(
            "/indexarr/identity/status",
            get(indexarr::h_indexarr_identity_status),
        )
        .route(
            "/indexarr/identity/acknowledge",
            post(indexarr::h_indexarr_identity_acknowledge),
        )
        .route(
            "/indexarr/sync/preferences",
            get(indexarr::h_indexarr_sync_preferences_get),
        )
        .route(
            "/indexarr/sync/preferences",
            post(indexarr::h_indexarr_sync_preferences_set),
        );

    // RSS endpoints
    api_router = api_router
        .route("/rss/feeds", get(rss::h_rss_feeds_list))
        .route("/rss/feeds", post(rss::h_rss_feed_add))
        .route("/rss/feeds/{name}", put(rss::h_rss_feed_update))
        .route("/rss/feeds/{name}", delete(rss::h_rss_feed_delete))
        .route("/rss/items", get(rss::h_rss_items_list))
        .route("/rss/items/{id}/download", post(rss::h_rss_item_download))
        .route("/rss/rules", get(rss::h_rss_rules_list))
        .route("/rss/rules", post(rss::h_rss_rule_add))
        .route("/rss/rules/{id}", put(rss::h_rss_rule_update))
        .route("/rss/rules/{id}", delete(rss::h_rss_rule_delete))
        .route("/rss/settings", get(rss::h_rss_settings_get));

    // Auth endpoints — status is always available; login/refresh/logout need token store;
    // setup and change_credentials need credential store.
    api_router = api_router.route("/auth/status", get(super::auth::h_auth_status));

    if state.opts.token_store.is_some() {
        api_router = api_router
            .route("/auth/login", post(super::auth::h_auth_login))
            .route("/auth/refresh", post(super::auth::h_auth_refresh))
            .route("/auth/logout", post(super::auth::h_auth_logout));
    }

    if state.opts.credential_store.is_some() {
        api_router = api_router
            .route("/auth/setup", post(super::auth::h_auth_setup))
            .route(
                "/auth/change_credentials",
                post(super::auth::h_auth_change_credentials),
            );
    }

    if state.opts.api_keys.is_some() {
        api_router = api_router
            .route(
                "/auth/api_keys",
                get(super::auth::h_api_keys_list).post(super::auth::h_api_key_create),
            )
            .route("/auth/api_keys/{id}", delete(super::auth::h_api_key_delete));
    }

    api_router.with_state(state)
}

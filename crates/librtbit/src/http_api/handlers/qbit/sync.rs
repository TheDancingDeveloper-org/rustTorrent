//! Sync controller (`sync/*`): the main polling endpoints.

use super::*;

// ---------------------------------------------------------------------------
// Sync (main polling endpoint)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub(super) struct QbitServerState {
    pub(super) connection_status: &'static str,
    pub(super) dht_nodes: u64,
    pub(super) dl_info_data: u64,
    pub(super) dl_info_speed: u64,
    pub(super) dl_rate_limit: u64,
    pub(super) up_info_data: u64,
    pub(super) up_info_speed: u64,
    pub(super) up_rate_limit: u64,
    pub(super) queueing: bool,
    pub(super) use_alt_speed_limits: bool,
    pub(super) refresh_interval: u64,
    pub(super) free_space_on_disk: u64,
    pub(super) global_ratio: String,
}

#[derive(Serialize)]
pub(super) struct QbitMainData {
    pub(super) torrents: HashMap<String, QbitTorrentInfo>,
    pub(super) categories: HashMap<String, QbitCategory>,
    pub(super) tags: Vec<String>,
    pub(super) server_state: QbitServerState,
}

#[derive(Deserialize, Default)]
pub(super) struct SyncQuery {
    #[serde(default)]
    pub(super) rid: u64,
}

/// `sync/maindata` — the primary polling endpoint for WebUI frontends and many
/// integrations. `rid=0` (or an unknown/expired rid) returns a `full_update`
/// snapshot; a rid from a previous response returns only what changed since
/// it (see `qbit_sync`).
pub(super) async fn h_sync_maindata(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<SyncQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let now = now_unix();

    let priorities = queue_priorities(api);
    let torrents: HashMap<String, QbitTorrentInfo> = api.session().with_torrents(|iter| {
        iter.map(|(_id, handle)| {
            let stats = handle.stats();
            let mut info = build_torrent_info(handle, &stats, now);
            info.priority = priorities.get(&handle.id()).copied().unwrap_or(0);
            (info.hash.clone(), info)
        })
        .collect()
    });

    let categories: HashMap<String, QbitCategory> = api
        .api_list_categories()
        .into_iter()
        .map(|(name, category)| {
            let save_path = category
                .save_path
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name.clone(), QbitCategory { name, save_path })
        })
        .collect();

    let session_stats = api.api_session_stats();
    let config = api.session().ratelimits.get_config();
    let downloaded = session_stats.counters.fetched_bytes;
    let uploaded = session_stats.counters.uploaded_bytes;
    let global_ratio = if downloaded == 0 {
        "0.00".to_string()
    } else {
        format!("{:.2}", uploaded as f64 / downloaded as f64)
    };

    let server_state = QbitServerState {
        connection_status: "connected",
        dht_nodes: dht_nodes(&state),
        dl_info_data: downloaded,
        dl_info_speed: session_stats.download_speed.as_bytes(),
        dl_rate_limit: bps_to_u64(config.download_bps),
        up_info_data: uploaded,
        up_info_speed: session_stats.upload_speed.as_bytes(),
        up_rate_limit: bps_to_u64(config.upload_bps),
        queueing: api.session().queueing_enabled(),
        use_alt_speed_limits: api.session().alt_speed_enabled(),
        refresh_interval: 1500,
        free_space_on_disk: 0,
        global_ratio,
    };

    let snapshot = to_json_map(&QbitMainData {
        torrents,
        categories,
        tags: api.api_list_tags(),
        server_state,
    });
    axum::Json(state.sync.respond("maindata", query.rid, snapshot))
}

pub(super) fn to_json_map(value: &impl Serialize) -> serde_json::Map<String, serde_json::Value> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => unreachable!("sync payloads serialize to JSON objects"),
    }
}

#[derive(Serialize)]
pub(super) struct QbitPeer {
    pub(super) client: String,
    pub(super) connection: &'static str,
    pub(super) country: String,
    pub(super) country_code: String,
    pub(super) dl_speed: u64,
    pub(super) downloaded: u64,
    pub(super) files: String,
    pub(super) flags: String,
    pub(super) flags_desc: String,
    pub(super) ip: String,
    pub(super) port: u16,
    pub(super) progress: f64,
    pub(super) relevance: f64,
    pub(super) up_speed: u64,
    pub(super) uploaded: u64,
    pub(super) peer_id_client: String,
}

#[derive(Serialize)]
pub(super) struct QbitTorrentPeers {
    pub(super) show_flags: bool,
    pub(super) peers: HashMap<String, QbitPeer>,
}

#[derive(Deserialize)]
pub(super) struct SyncPeersQuery {
    pub(super) hash: String,
    #[serde(default)]
    pub(super) rid: u64,
}

/// `sync/torrentPeers` — live peers of one torrent, keyed by `ip:port`, with
/// the same rid delta encoding as `sync/maindata`. Only the transfer counters
/// and connection kind are known natively; client name, country, flags,
/// per-peer progress and speeds are reported empty/zero.
pub(super) async fn h_sync_torrent_peers(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<SyncPeersQuery>,
) -> Response {
    let api = &state.api_state.api;
    let Ok(idx) = TorrentIdOrHash::parse(&query.hash) else {
        return qbit_error(StatusCode::NOT_FOUND, "Not found");
    };
    let Ok(handle) = api.mgr_handle(idx) else {
        return qbit_error(StatusCode::NOT_FOUND, "Not found");
    };
    // A torrent that is not live (paused, initializing) simply has no peers.
    let peers = handle
        .live()
        .map(|live| {
            live.per_peer_stats_snapshot(PeerStatsFilter::default())
                .peers
                .into_iter()
                .filter_map(|(addr, stats)| {
                    let socket: std::net::SocketAddr = addr.parse().ok()?;
                    let connection = match stats.conn_kind {
                        Some(ConnectionKind::Utp) => "μTP",
                        _ => "BT",
                    };
                    let peer = QbitPeer {
                        client: String::new(),
                        connection,
                        country: String::new(),
                        country_code: String::new(),
                        dl_speed: 0,
                        downloaded: stats.counters.fetched_bytes,
                        files: String::new(),
                        flags: String::new(),
                        flags_desc: String::new(),
                        ip: socket.ip().to_string(),
                        port: socket.port(),
                        progress: 0.0,
                        relevance: 0.0,
                        up_speed: 0,
                        uploaded: stats.counters.uploaded_bytes,
                        peer_id_client: String::new(),
                    };
                    Some((addr, peer))
                })
                .collect()
        })
        .unwrap_or_default();

    let snapshot = to_json_map(&QbitTorrentPeers {
        show_flags: true,
        peers,
    });
    let scope = format!("peers:{}", handle.info_hash().as_string());
    axum::Json(state.sync.respond(&scope, query.rid, snapshot)).into_response()
}

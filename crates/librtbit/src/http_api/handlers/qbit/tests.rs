use std::{net::Ipv4Addr, path::Path, sync::Arc, time::Duration};

use axum::response::IntoResponse;
use bytes::Bytes;
use http::StatusCode;
use tower::ServiceExt;

use crate::{
    AddTorrent, AddTorrentOptions, Api, CreateTorrentOptions, ListenerMode, Session,
    SessionOptions, create_torrent,
    http_api::{HttpApi, HttpApiOptions},
    listen::ListenerOptions,
    spawn_utils::BlockingSpawner,
    torrent_state::TorrentStatsState,
};

use super::{
    AddTaskForm, CreatorParams, CreatorStatus, CreatorTask, QbitSessions, QbitState,
    QbitTorrentCreator, RequiredTaskIdQuery, SyncSnapshots, creator_params, creator_task_status,
    h_app_preferences, h_app_set_preferences, h_app_version, h_torrentcreator_add_task,
    h_torrentcreator_delete_task, h_torrentcreator_torrent_file, h_torrents_add_tags,
    h_torrents_create_tags, h_torrents_delete_tags, h_torrents_info, h_torrents_recheck,
    h_torrents_remove_tags, h_torrents_set_tags, matches_category, origin_allowed, qbit_file_name,
};

async fn qbit_state() -> (Arc<QbitState>, Arc<Session>, tempfile::TempDir) {
    let output = tempfile::TempDir::with_prefix("qbit_preferences").unwrap();
    let session = Session::new_with_opts(
        output.path().to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                announce_port: Some(4241),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let api = Api::new(
        session.clone(),
        None,
        #[cfg(feature = "tracing-subscriber-utils")]
        None,
    );
    let api_state = Arc::new(HttpApi::new(
        api,
        Some(HttpApiOptions {
            web_ui_port: Some(3031),
            allow_create: true,
            ..Default::default()
        }),
    ));
    (
        Arc::new(QbitState {
            api_state,
            sessions: QbitSessions::new(),
            creator: QbitTorrentCreator::default(),
            sync: SyncSnapshots::default(),
            clientdata: super::ClientDataStore::open(None),
        }),
        session,
        output,
    )
}

#[tokio::test]
async fn preferences_reports_fixed_listener_and_runtime_announce_port() {
    let (state, session, _output) = qbit_state().await;

    let preferences = h_app_preferences(axum::extract::State(state)).await.0;

    assert_eq!(
        preferences.listen_port,
        session.listen_addr().unwrap().port()
    );
    assert_eq!(preferences.announce_port, 4241);
    assert_eq!(preferences.web_ui_port, 3031);
}

#[tokio::test]
async fn set_preferences_updates_port_idempotently_without_rebinding_listener() {
    let (state, session, _output) = qbit_state().await;
    let listen_addr = session.listen_addr().unwrap();
    let body = Bytes::from_static(b"json=%7B%22announce_port%22%3A51234%7D");

    for _ in 0..2 {
        let response = h_app_set_preferences(axum::extract::State(state.clone()), body.clone())
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    assert_eq!(session.announce_port(), Some(51_234));
    assert_eq!(session.listen_addr(), Some(listen_addr));
}

#[tokio::test]
async fn set_preferences_ignores_unsupported_fields_and_missing_port() {
    let (state, session, _output) = qbit_state().await;
    // A read-modify-write client posting its whole preferences object back.
    let body = Bytes::from(
        "json=%7B%22save_path%22%3A%22%2Ftmp%22%2C%22dl_limit%22%3A0%2C%22max_ratio%22%3A1.5%7D",
    );
    let response = h_app_set_preferences(axum::extract::State(state.clone()), body)
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(session.announce_port(), Some(4241));
}

#[tokio::test]
async fn set_preferences_rejects_invalid_announce_port_and_malformed_form() {
    let (state, session, _output) = qbit_state().await;
    let invalid_bodies = [
        "json=%7B%22announce_port%22%3A0%7D",
        "json=%7B%22announce_port%22%3A65536%7D",
        "json=not-json",
        "announce_port=4241",
    ];

    for body in invalid_bodies {
        let response = h_app_set_preferences(
            axum::extract::State(state.clone()),
            Bytes::copy_from_slice(body.as_bytes()),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "body={body}");
    }
    assert_eq!(session.announce_port(), Some(4241));
}

#[tokio::test]
async fn app_version_is_on_the_5x_line() {
    assert!(h_app_version().await.starts_with("v5."));
}

#[tokio::test]
async fn preferences_report_seeding_limits_as_disabled() {
    let (state, _session, _output) = qbit_state().await;
    let preferences =
        serde_json::to_value(h_app_preferences(axum::extract::State(state)).await.0).unwrap();
    assert_eq!(preferences["max_ratio_enabled"], false);
    assert_eq!(preferences["max_seeding_time_enabled"], false);
    assert!(preferences["max_ratio"].is_f64());
    assert!(preferences["max_ratio_act"].is_u64());
    assert!(preferences["max_seeding_time"].is_i64());
}

#[test]
fn sessions_slide_forward_on_use_and_prune_idle_entries() {
    let sessions = QbitSessions::with_idle_ttl(Duration::from_millis(300));
    let live = sessions.create_session();
    std::thread::sleep(Duration::from_millis(200));
    // Still inside the idle window, so this use slides the expiry forward.
    assert!(sessions.validate_session(&live));
    std::thread::sleep(Duration::from_millis(200));
    // 400ms since creation, but only 200ms since last use.
    assert!(sessions.validate_session(&live));

    let stale = sessions.create_session();
    std::thread::sleep(Duration::from_millis(400));
    // `live` and `stale` have both been idle longer than the TTL, so creating
    // a new session prunes them and only `fresh` remains.
    let fresh = sessions.create_session();
    assert_eq!(sessions.sessions.read().len(), 1);
    assert!(!sessions.sessions.read().contains_key(&stale));
    assert!(sessions.validate_session(&fresh));
}

#[test]
fn purge_expired_drops_only_idle_sessions() {
    let sessions = QbitSessions::with_idle_ttl(Duration::from_millis(300));
    let stale = sessions.create_session();
    std::thread::sleep(Duration::from_millis(400));
    sessions
        .sessions
        .write()
        .insert("fresh".into(), std::time::Instant::now());
    assert_eq!(sessions.purge_expired(), 1);
    assert!(!sessions.sessions.read().contains_key(&stale));
    assert!(sessions.validate_session("fresh"));
    assert_eq!(sessions.purge_expired(), 0);
}

#[tokio::test]
async fn set_preferences_partially_applies_speed_limits() {
    let (state, session, _output) = qbit_state().await;
    // Unknown keys are ignored; known keys apply.
    let json = r#"{"dl_limit":1048576,"alt_up_limit":2048,"bogus_key":[1,2],"locale":"de"}"#;
    let body = format!("json={}", urlencoding::encode(json));
    let response = h_app_set_preferences(axum::extract::State(state.clone()), Bytes::from(body))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let prefs = serde_json::to_value(
        h_app_preferences(axum::extract::State(state.clone()))
            .await
            .0,
    )
    .unwrap();
    assert_eq!(prefs["dl_limit"], 1_048_576);
    assert_eq!(prefs["up_limit"], 0);
    assert_eq!(prefs["alt_up_limit"], 2048);
    assert_eq!(prefs["alt_dl_limit"], 0);
    assert_eq!(session.alt_speed_status().config.upload_rate, Some(2048));

    // -1 clears a limit; untouched keys keep their values.
    let body = format!("json={}", urlencoding::encode(r#"{"dl_limit":-1}"#));
    h_app_set_preferences(axum::extract::State(state.clone()), Bytes::from(body)).await;
    let prefs =
        serde_json::to_value(h_app_preferences(axum::extract::State(state)).await.0).unwrap();
    assert_eq!(prefs["dl_limit"], 0);
    assert_eq!(prefs["alt_up_limit"], 2048);
}

#[test]
fn origin_check_allows_same_origin_and_non_browser_clients() {
    let headers = |origin: Option<&str>, host: Option<&str>| {
        let mut map = http::HeaderMap::new();
        if let Some(origin) = origin {
            map.insert(http::header::ORIGIN, origin.parse().unwrap());
        }
        if let Some(host) = host {
            map.insert(http::header::HOST, host.parse().unwrap());
        }
        map
    };
    assert!(origin_allowed(&headers(None, None)));
    assert!(origin_allowed(&headers(None, Some("10.0.0.2:3030"))));
    assert!(origin_allowed(&headers(
        Some("http://10.0.0.2:3030"),
        Some("10.0.0.2:3030")
    )));
    assert!(origin_allowed(&headers(
        Some("HTTP://Example.TEST:3030/path"),
        Some("example.test:3030")
    )));
    assert!(!origin_allowed(&headers(
        Some("https://evil.example"),
        Some("10.0.0.2:3030")
    )));
    assert!(!origin_allowed(&headers(
        Some("null"),
        Some("10.0.0.2:3030")
    )));
    assert!(!origin_allowed(&headers(
        Some("http://10.0.0.2:3030"),
        None
    )));
}

#[tokio::test]
async fn cross_origin_login_post_is_rejected_by_the_router() {
    let (state, _session, _output) = qbit_state().await;
    let router = super::make_qbit_router(state.api_state.clone());
    let request = http::Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("host", "10.0.0.2:3030")
        .header("origin", "https://evil.example")
        .body(axum::body::Body::from("username=a&password=b"))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn map_state_reports_queued_and_forced() {
    use super::map_state;
    let (live, paused) = (TorrentStatsState::Live, TorrentStatsState::Paused);
    assert_eq!(map_state(paused, false, true, false), "queuedDL");
    assert_eq!(map_state(paused, true, true, false), "queuedUP");
    assert_eq!(map_state(paused, false, false, false), "stoppedDL");
    assert_eq!(map_state(live, false, false, true), "forcedDL");
    assert_eq!(map_state(live, true, false, true), "forcedUP");
    assert_eq!(map_state(live, true, false, false), "uploading");
}

#[test]
fn queue_preferences_map_to_limits() {
    use super::{SetPreferences, queue_limits_update};
    let prefs = |json: &str| serde_json::from_str::<SetPreferences>(json).unwrap();

    assert_eq!(queue_limits_update(&prefs("{}"), (None, None)), None);
    // Turning queueing on with no limits uses the defaults.
    assert_eq!(
        queue_limits_update(&prefs(r#"{"queueing_enabled":true}"#), (None, None)),
        Some((Some(3), Some(3)))
    );
    assert_eq!(
        queue_limits_update(
            &prefs(r#"{"queueing_enabled":true,"max_active_downloads":2,"max_active_uploads":-1}"#),
            (None, None)
        ),
        Some((Some(2), None))
    );
    // A limit change while queueing is on keeps the other limit.
    assert_eq!(
        queue_limits_update(&prefs(r#"{"max_active_downloads":5}"#), (Some(1), Some(4))),
        Some((Some(5), Some(4)))
    );
    // Limits sent while queueing is off do not turn it on.
    assert_eq!(
        queue_limits_update(&prefs(r#"{"max_active_downloads":5}"#), (None, None)),
        Some((None, None))
    );
    assert_eq!(
        queue_limits_update(&prefs(r#"{"queueing_enabled":false}"#), (Some(1), Some(1))),
        Some((None, None))
    );
}

#[tokio::test]
async fn prio_endpoints_reorder_queue_and_report_priority() {
    let (state, session, output) = qbit_state().await;
    let mut hashes = Vec::new();
    for byte in [0x41u8, 0x42, 0x43] {
        let dir = output.path().join(format!("src{byte}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("payload.bin"), vec![byte; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            &dir,
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().join("dl").to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();
        hashes.push(handle.info_hash().as_string());
    }
    let router = super::make_qbit_router(state.api_state.clone());
    let post = |action: &str, body: String| {
        http::Request::builder()
            .method("POST")
            .uri(format!("/torrents/{action}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from(body))
            .unwrap()
    };
    let priorities = || {
        let router = router.clone();
        let hashes = hashes.clone();
        async move {
            let request = http::Request::builder()
                .uri("/torrents/info")
                .body(axum::body::Body::empty())
                .unwrap();
            let response = router.oneshot(request).await.unwrap();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let list: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
            hashes
                .iter()
                .map(|h| {
                    list.iter().find(|t| t["hash"] == *h).unwrap()["priority"]
                        .as_u64()
                        .unwrap()
                })
                .collect::<Vec<_>>()
        }
    };

    // Queueing off: every priority is 0.
    assert_eq!(priorities().await, vec![0, 0, 0]);
    session.set_queue_limits(Some(1), None).await.unwrap();
    assert_eq!(priorities().await, vec![1, 2, 3]);

    for (action, hash, expected) in [
        ("topPrio", 2, vec![2, 3, 1]),
        ("increasePrio", 1, vec![3, 2, 1]),
        ("bottomPrio", 2, vec![2, 1, 3]),
        ("decreasePrio", 0, vec![3, 1, 2]),
    ] {
        let response = router
            .clone()
            .oneshot(post(action, format!("hashes={}", hashes[hash])))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{action}");
        assert_eq!(priorities().await, expected, "{action}");
    }

    // setForceStart starts the torrent outside the queue.
    let response = router
        .clone()
        .oneshot(post(
            "setForceStart",
            format!("hashes={}&value=true", hashes[0]),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let handle = state
        .api_state
        .api
        .mgr_handle(crate::api::TorrentIdOrHash::parse(&hashes[0]).unwrap())
        .unwrap();
    assert!(handle.is_force_started());
    assert!(!handle.is_paused());
}

#[tokio::test]
async fn queue_and_force_start_routes_succeed() {
    let (state, _session, _output) = qbit_state().await;
    let router = super::make_qbit_router(state.api_state.clone());
    for action in [
        "topPrio",
        "bottomPrio",
        "increasePrio",
        "decreasePrio",
        "setForceStart",
        "toggleSequentialDownload",
        "toggleFirstLastPiecePrio",
    ] {
        let request = http::Request::builder()
            .method("POST")
            .uri(format!("/torrents/{action}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from("hashes=all&value=true"))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{action}");
    }
}

#[tokio::test]
async fn recheck_endpoint_accepts_qbit_hash_form() {
    let (state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
    let torrent = create_torrent(
        output.path(),
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();

    let body = Bytes::from(format!("hashes={}", handle.info_hash().as_string()));
    assert_eq!(
        h_torrents_recheck(axum::extract::State(state), body).await,
        "Ok."
    );
    assert!(matches!(
        handle.stats().state,
        TorrentStatsState::Initializing
    ));
    handle.wait_until_initialized().await.unwrap();
    assert!(matches!(handle.stats().state, TorrentStatsState::Paused));
}

#[tokio::test]
async fn piece_order_toggles_flip_per_torrent_flags() {
    let (state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x52; 32 * 1024]).unwrap();
    let torrent = create_torrent(
        output.path(),
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();
    assert!(!handle.is_sequential_download());
    assert!(!handle.is_first_last_piece_priority());

    let body = || Bytes::from(format!("hashes={}", handle.info_hash().as_string()));
    let toggle_seq = || {
        super::h_torrents_toggle_sequential_download(axum::extract::State(state.clone()), body())
    };
    let toggle_fl = || {
        super::h_torrents_toggle_first_last_piece_prio(axum::extract::State(state.clone()), body())
    };

    assert_eq!(toggle_seq().await, "Ok.");
    assert!(handle.is_sequential_download());
    assert!(!handle.is_first_last_piece_priority());

    assert_eq!(toggle_fl().await, "Ok.");
    assert!(handle.is_first_last_piece_priority());

    let info = super::build_torrent_info(&handle, &handle.stats(), 0);
    assert!(info.seq_dl);
    assert!(info.f_l_piece_prio);

    assert_eq!(toggle_seq().await, "Ok.");
    assert_eq!(toggle_fl().await, "Ok.");
    assert!(!handle.is_sequential_download());
    assert!(!handle.is_first_last_piece_priority());
}

#[tokio::test]
async fn per_torrent_rate_limits_round_trip_on_handle() {
    use std::num::NonZeroU32;

    let (_state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
    let torrent = create_torrent(
        output.path(),
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();

    // Defaults to unlimited.
    assert_eq!(handle.rate_limits().download_bps, None);
    assert_eq!(handle.rate_limits().upload_bps, None);

    // Each direction is set independently and persists on the override
    // (this torrent is paused, so there is no live limiter to update).
    handle.set_download_limit(NonZeroU32::new(4096));
    handle.set_upload_limit(NonZeroU32::new(8192));
    assert_eq!(handle.rate_limits().download_bps, NonZeroU32::new(4096));
    assert_eq!(handle.rate_limits().upload_bps, NonZeroU32::new(8192));

    // Setting one leaves the other untouched.
    handle.set_download_limit(NonZeroU32::new(1024));
    assert_eq!(handle.rate_limits().download_bps, NonZeroU32::new(1024));
    assert_eq!(handle.rate_limits().upload_bps, NonZeroU32::new(8192));

    // None clears the limit.
    handle.set_download_limit(None);
    assert_eq!(handle.rate_limits().download_bps, None);
}

#[tokio::test]
async fn rename_file_moves_on_disk_and_updates_metadata_when_paused() {
    let (_state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
    let torrent = create_torrent(
        output.path(),
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();

    // Discover file 0's real on-disk location from the metadata.
    let old_rel = handle
        .with_metadata(|m| m.file_infos[0].relative_filename.clone())
        .unwrap();
    let old_abs = output.path().join(&old_rel);
    assert!(old_abs.exists(), "expected file at {old_abs:?}");

    // A path escaping the root is rejected before anything moves.
    assert!(
        handle
            .rename_files(&[(0, std::path::PathBuf::from("../escape.bin"))])
            .is_err()
    );
    assert!(old_abs.exists(), "rejected rename must not move anything");

    // A valid rename moves the file on disk and updates file_infos.
    let new_rel = std::path::PathBuf::from("renamed_dir/renamed.bin");
    handle.rename_files(&[(0, new_rel.clone())]).unwrap();
    assert_eq!(
        handle
            .with_metadata(|m| m.file_infos[0].relative_filename.clone())
            .unwrap(),
        new_rel
    );
    assert!(!old_abs.exists(), "old path should be gone");
    assert!(
        output.path().join(&new_rel).exists(),
        "new path should exist"
    );

    // The display-name override is independent of file renames.
    assert!(handle.name().is_some());
    handle.set_display_name(Some("Custom Name".to_string()));
    assert_eq!(handle.name().as_deref(), Some("Custom Name"));
    handle.set_display_name(Some("   ".to_string()));
    assert_ne!(
        handle.name().as_deref(),
        Some("   "),
        "blank name clears override"
    );
}

#[tokio::test]
async fn set_location_relocates_torrent_files_when_paused() {
    let (_state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
    let torrent = create_torrent(
        output.path(),
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();

    let rel = handle
        .with_metadata(|m| m.file_infos[0].relative_filename.clone())
        .unwrap();
    let old_abs = output.path().join(&rel);
    assert!(old_abs.exists(), "expected file at {old_abs:?}");

    // Relocate to a sibling directory on the same filesystem.
    let new_root = output.path().join("relocated");
    handle.set_location(new_root.clone()).unwrap();

    assert!(!old_abs.exists(), "old location should be gone");
    assert!(
        new_root.join(&rel).exists(),
        "file should now live under the new root"
    );
}

async fn add_single_file_torrent(
    session: &Arc<crate::Session>,
    root: &std::path::Path,
    paused: bool,
    file_renames: Vec<(usize, std::path::PathBuf)>,
) -> crate::torrent_state::ManagedTorrentHandle {
    let torrent = create_torrent(
        root,
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused,
                overwrite: true,
                output_folder: Some(root.to_string_lossy().into_owned()),
                file_renames,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();
    handle
}

#[tokio::test]
async fn session_set_location_pauses_live_torrent_and_resumes_it() {
    let (_state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
    let handle = add_single_file_torrent(&session, output.path(), false, Vec::new()).await;
    assert!(handle.live().is_some(), "torrent should be live");
    let rel = handle
        .with_metadata(|m| m.file_infos[0].relative_filename.clone())
        .unwrap();

    // The handle-level move still refuses a live torrent...
    let new_root = output.path().join("moved");
    assert!(handle.set_location(new_root.clone()).is_err());

    // ...while the session pauses it for the move and resumes it.
    session
        .set_location(&handle, new_root.clone())
        .await
        .unwrap();
    assert!(
        new_root.join(&rel).exists(),
        "file should be under new root"
    );
    assert!(
        !output.path().join(&rel).exists(),
        "old file should be gone"
    );
    assert_eq!(handle.output_folder(), new_root);
    assert!(
        !handle.is_paused(),
        "torrent should be resumed after the move"
    );
}

#[tokio::test]
async fn session_rename_files_works_on_live_torrent_and_reports_renames() {
    let (_state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
    let handle = add_single_file_torrent(&session, output.path(), false, Vec::new()).await;
    assert!(handle.renamed_files().is_empty());

    let new_rel = std::path::PathBuf::from("sub/renamed.bin");
    session
        .rename_files(&handle, &[(0, new_rel.clone())])
        .await
        .unwrap();
    assert!(output.path().join(&new_rel).exists());
    assert_eq!(handle.renamed_files(), vec![(0, new_rel)]);
    assert!(
        !handle.is_paused(),
        "torrent should be resumed after the rename"
    );
}

#[tokio::test]
async fn add_torrent_applies_restored_file_renames() {
    let (_state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
    let torrent = create_torrent(
        output.path(),
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    // Lay the file out the way an earlier rename would have left it.
    let dest = output.path().join("dest");
    let rel = std::path::PathBuf::from("restored/payload.bin");
    std::fs::create_dir_all(dest.join("restored")).unwrap();
    std::fs::rename(output.path().join("payload.bin"), dest.join(&rel)).unwrap();

    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(dest.to_string_lossy().into_owned()),
                file_renames: vec![(0, rel.clone())],
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();
    assert_eq!(
        handle
            .with_metadata(|m| m.file_infos[0].relative_filename.clone())
            .unwrap(),
        rel
    );
    assert_eq!(handle.renamed_files(), vec![(0, rel.clone())]);
    // The restored file is recognised as complete data at the renamed path.
    assert!(dest.join(&rel).exists());
    assert!(!dest.join("payload.bin").exists());
}

#[tokio::test]
async fn add_torrent_ignores_invalid_restored_file_renames() {
    let (_state, session, output) = qbit_state().await;
    std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
    let handle = add_single_file_torrent(
        &session,
        output.path(),
        true,
        vec![(0, std::path::PathBuf::from("../escape.bin"))],
    )
    .await;
    assert!(handle.renamed_files().is_empty());
}

#[test]
fn category_filter_supports_qbit_special_values() {
    assert!(matches_category("all", "Linux ISOs"));
    assert!(matches_category("all", ""));
    assert!(matches_category("uncategorized", ""));
    assert!(matches_category("", ""));
    assert!(!matches_category("uncategorized", "Linux ISOs"));
    assert!(matches_category("Linux ISOs", "Linux ISOs"));
    assert!(!matches_category("linux isos", "Linux ISOs"));
}

#[test]
fn qbit_multi_file_names_include_torrent_root_for_save_path_imports() {
    assert_eq!(
        qbit_file_name(Some("release"), "disc/file.bin", true, true),
        Path::new("release").join("disc/file.bin").to_string_lossy()
    );
    assert_eq!(
        qbit_file_name(Some("release"), "single.bin", false, true),
        "single.bin"
    );
    assert_eq!(
        qbit_file_name(Some("release"), "only.bin", true, true),
        Path::new("release").join("only.bin").to_string_lossy()
    );
    assert_eq!(
        qbit_file_name(Some("release"), "disc/file.bin", true, false),
        "disc/file.bin"
    );
}

#[test]
fn test_eta_overflow_safety() {
    // Simulate: very large remaining bytes / very small download speed
    // This should not panic, and should clamp to a safe fallback.
    let remaining: u64 = u64::MAX;
    let dl_speed: u64 = 1;
    let eta_secs = remaining / dl_speed;
    let eta = i64::try_from(eta_secs).unwrap_or(8640000i64);
    assert_eq!(eta, 8640000i64, "should clamp to fallback on overflow");

    // Large but within i64 range
    let remaining: u64 = 1_000_000_000_000;
    let dl_speed: u64 = 100;
    let eta_secs = remaining / dl_speed;
    let eta = i64::try_from(eta_secs).unwrap_or(8640000i64);
    assert_eq!(
        eta, 10_000_000_000i64,
        "should return exact ETA when it fits in i64"
    );

    // Zero speed: the calling code uses a guard, but verify the fallback path
    let dl_speed: u64 = 0;
    let eta = 1_000_000u64
        .checked_div(dl_speed)
        .map(|seconds| i64::try_from(seconds).unwrap_or(8_640_000))
        .unwrap_or(8_640_000);
    assert_eq!(eta, 8640000i64, "zero speed should return 8640000");
}

#[test]
fn test_timestamp_cast_safety() {
    // Current unix timestamp fits in i64, but test the boundary
    let now: u64 = i64::MAX as u64 + 1;
    let result = i64::try_from(now).unwrap_or(i64::MAX);
    assert_eq!(result, i64::MAX, "should clamp to i64::MAX on overflow");

    // Normal timestamp should pass through
    let now: u64 = 1_700_000_000;
    let result = i64::try_from(now).unwrap_or(i64::MAX);
    assert_eq!(result, 1_700_000_000i64);
}

#[test]
fn parse_tags_trims_splits_and_drops_empties() {
    assert_eq!(
        super::parse_tags("tv, , radarr ,,sonarr"),
        vec!["tv".to_string(), "radarr".to_string(), "sonarr".to_string()]
    );
    assert!(super::parse_tags("").is_empty());
    assert!(super::parse_tags("  , ,").is_empty());
}

/// Add a paused torrent over a single file of `byte` repeated, so each
/// call with a different byte yields a distinct info hash.
async fn add_paused_torrent(
    session: &Arc<Session>,
    output: &tempfile::TempDir,
    byte: u8,
) -> crate::torrent_state::ManagedTorrentHandle {
    let dir = output.path().join(format!("src{byte}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("payload.bin"), vec![byte; 32 * 1024]).unwrap();
    let torrent = create_torrent(
        &dir,
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();
    handle
}

fn torrent_tags(handle: &crate::torrent_state::ManagedTorrentHandle) -> Vec<String> {
    handle.shared().tags.read().iter().cloned().collect()
}

#[tokio::test]
async fn tag_endpoints_drive_backend_tags() {
    let (state, session, output) = qbit_state().await;
    let a = add_paused_torrent(&session, &output, 0x11).await;
    let b = add_paused_torrent(&session, &output, 0x22).await;
    let (ha, hb) = (a.info_hash().as_string(), b.info_hash().as_string());
    let st = || axum::extract::State(state.clone());
    let form = |s: String| Bytes::from(s);

    // createTags registers tags without touching any torrent.
    h_torrents_create_tags(st(), form("tags=tv, hd".into())).await;
    assert_eq!(session.tags().list(), vec!["hd", "tv"]);
    assert!(torrent_tags(&a).is_empty());

    // addTags assigns tags (registering new ones) to every listed torrent.
    h_torrents_add_tags(st(), form(format!("hashes={ha}|{hb}&tags=tv,new"))).await;
    assert_eq!(torrent_tags(&a), vec!["new", "tv"]);
    assert_eq!(torrent_tags(&b), vec!["new", "tv"]);
    assert!(session.tags().exists("new"));

    // removeTags with a specific tag only drops it from the given torrent.
    h_torrents_remove_tags(st(), form(format!("hashes={ha}&tags=tv"))).await;
    assert_eq!(torrent_tags(&a), vec!["new"]);
    assert_eq!(torrent_tags(&b), vec!["new", "tv"]);

    // setTags replaces the whole tag set.
    h_torrents_set_tags(st(), form(format!("hashes={hb}&tags=only"))).await;
    assert_eq!(torrent_tags(&b), vec!["only"]);

    // deleteTags unregisters a tag and strips it from every torrent.
    h_torrents_delete_tags(st(), form("tags=new".into())).await;
    assert!(torrent_tags(&a).is_empty());
    assert!(!session.tags().exists("new"));

    // removeTags with an empty list clears every tag on the torrent.
    h_torrents_add_tags(st(), form(format!("hashes={ha}&tags=x,y"))).await;
    h_torrents_remove_tags(st(), form(format!("hashes={ha}&tags="))).await;
    assert!(torrent_tags(&a).is_empty());

    // torrents/info exposes the tags and filters by them; an empty tag
    // selects untagged torrents.
    let info = |tag: &str| {
        let state = state.clone();
        let tag = tag.to_owned();
        async move {
            let query = super::TorrentsInfoQuery {
                tag: Some(tag),
                ..Default::default()
            };
            let response =
                h_torrents_info(axum::extract::State(state), axum::extract::Query(query))
                    .await
                    .into_response();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice::<Vec<serde_json::Value>>(&body).unwrap()
        }
    };
    let only = info("only").await;
    assert_eq!(only.len(), 1);
    assert_eq!(only[0]["hash"], hb);
    assert_eq!(only[0]["tags"], "only");
    let untagged = info("").await;
    assert_eq!(untagged.len(), 1);
    assert_eq!(untagged[0]["hash"], ha);
    assert!(info("missing").await.is_empty());
}

#[tokio::test]
async fn legacy_tag_store_is_imported_once() {
    let (state, session, output) = qbit_state().await;
    let a = add_paused_torrent(&session, &output, 0x33).await;
    let path = output.path().join("qbit-tags.json");
    let legacy = serde_json::json!({
        "all": ["movies"],
        "per_torrent": {
            a.info_hash().as_string(): ["hd"],
            "f".repeat(40): ["gone"],
        },
    });
    std::fs::write(&path, legacy.to_string()).unwrap();

    super::migrate_legacy_tag_store(&state.api_state.api, &path).await;

    assert_eq!(torrent_tags(&a), vec!["hd"]);
    assert_eq!(session.tags().list(), vec!["hd", "movies"]);
    assert!(!path.exists());
    assert!(path.with_extension("json.migrated").exists());
}

#[tokio::test]
async fn unreadable_legacy_tag_store_is_left_in_place() {
    let (state, session, output) = qbit_state().await;
    let path = output.path().join("qbit-tags.json");
    std::fs::write(&path, b"{ not json").unwrap();
    super::migrate_legacy_tag_store(&state.api_state.api, &path).await;
    assert!(path.exists());
    assert!(session.tags().list().is_empty());
}

#[test]
fn matches_tag_treats_empty_as_untagged() {
    let tags: std::collections::BTreeSet<String> = ["tv".to_owned()].into();
    assert!(super::matches_tag("tv", &tags));
    assert!(!super::matches_tag("hd", &tags));
    assert!(!super::matches_tag("", &tags));
    assert!(super::matches_tag("", &Default::default()));
}

#[test]
fn offset_limit_pagination_matches_qbittorrent_semantics() {
    use super::apply_offset_limit;
    let v = || vec![0, 1, 2, 3, 4];
    // Normal window.
    assert_eq!(apply_offset_limit(v(), 1, Some(2)), vec![1, 2]);
    // Offset past the end -> empty (not the whole list).
    assert_eq!(apply_offset_limit(v(), 5, None), Vec::<i32>::new());
    assert_eq!(apply_offset_limit(v(), 99, Some(3)), Vec::<i32>::new());
    // Offset exactly at the end -> empty.
    assert_eq!(apply_offset_limit(v(), 5, None), Vec::<i32>::new());
    // limit == 0 means no limit.
    assert_eq!(apply_offset_limit(v(), 0, Some(0)), vec![0, 1, 2, 3, 4]);
    // No offset, no limit.
    assert_eq!(apply_offset_limit(v(), 0, None), vec![0, 1, 2, 3, 4]);
}

#[test]
fn share_limit_wire_encoding_round_trips() {
    use super::{minutes_from_qbit, minutes_to_qbit, ratio_from_qbit, ratio_to_qbit};
    use std::time::Duration;
    assert_eq!(ratio_from_qbit(-1.0), None);
    assert_eq!(ratio_from_qbit(-2.0), None);
    assert_eq!(ratio_from_qbit(0.0), Some(0.0));
    assert_eq!(ratio_to_qbit(ratio_from_qbit(1.5)), 1.5);
    assert_eq!(ratio_to_qbit(None), -1.0);
    assert_eq!(minutes_from_qbit(-1), None);
    assert_eq!(minutes_from_qbit(-2), None);
    assert_eq!(minutes_from_qbit(90), Some(Duration::from_secs(5400)));
    assert_eq!(minutes_to_qbit(minutes_from_qbit(90)), 90);
    assert_eq!(minutes_to_qbit(None), -1);
}

#[test]
fn torrent_creator_task_store_lifecycle() {
    let creator = QbitTorrentCreator::default();
    let id = creator.new_task(CreatorParams {
        source_path: "/data/movie".to_string(),
        ..Default::default()
    });
    let task = creator.get(&id).unwrap();
    assert!(matches!(task.status, CreatorStatus::Running));
    assert_eq!(task.params.source_path, "/data/movie");
    assert_eq!(creator.all().len(), 1);

    creator.finish(
        &id,
        Some(bytes::Bytes::from_static(b"torrent")),
        "abc".to_string(),
    );
    let task = creator.get(&id).unwrap();
    assert!(matches!(task.status, CreatorStatus::Finished));
    assert_eq!(task.info_hash.as_deref(), Some("abc"));
    assert_eq!(task.torrent_bytes.as_deref(), Some(&b"torrent"[..]));

    let failing = creator.new_task(CreatorParams::default());
    creator.fail(&failing, "boom".to_string());
    let task = creator.get(&failing).unwrap();
    assert!(matches!(task.status, CreatorStatus::Failed));
    assert_eq!(task.error.as_deref(), Some("boom"));

    assert!(creator.remove(&id));
    assert!(!creator.remove(&id));
    assert!(creator.get(&id).is_none());
    assert_eq!(creator.all().len(), 1);
}

#[tokio::test]
async fn torrentcreator_add_task_produces_a_torrent_file() {
    let (state, _session, output) = qbit_state().await;
    std::fs::write(output.path().join("data.bin"), vec![7u8; 4096]).unwrap();

    let body = Bytes::from(format!("sourcePath={}", output.path().to_string_lossy()));
    let response = h_torrentcreator_add_task(axum::extract::State(state.clone()), body)
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    // The task is registered synchronously; poll it to completion.
    let all = state.creator.all();
    assert_eq!(all.len(), 1);
    let (id, _) = all.into_iter().next().unwrap();
    for _ in 0..200 {
        if !matches!(
            state.creator.get(&id).unwrap().status,
            CreatorStatus::Running
        ) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let task = state.creator.get(&id).unwrap();
    assert!(
        matches!(task.status, CreatorStatus::Finished),
        "creation did not finish: {:?}",
        task.error
    );
    assert!(
        task.torrent_bytes.is_some(),
        "expected generated torrent bytes"
    );
    assert!(task.info_hash.is_some());
}

async fn wait_for_creator_task(state: &QbitState, id: &str) -> CreatorTask {
    for _ in 0..250 {
        let task = state.creator.get(id).unwrap();
        if task.status != CreatorStatus::Running {
            return task;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("creator task {id} did not finish");
}

fn task_id_of(response: axum::response::Response) -> String {
    assert_eq!(response.status(), StatusCode::OK);
    let all =
        futures::executor::block_on(axum::body::to_bytes(response.into_body(), 1 << 16)).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&all).unwrap();
    json["taskID"].as_str().unwrap().to_owned()
}

#[test]
fn creator_params_validate_and_parse_the_form() {
    let form = |body: &str| -> AddTaskForm { serde_urlencoded::from_str(body).unwrap() };

    assert!(creator_params(form("")).is_err());
    assert!(creator_params(form("sourcePath=/d&format=v2")).is_err());
    assert!(creator_params(form("sourcePath=/d&format=hybrid")).is_err());
    assert!(creator_params(form("sourcePath=/d&format=v9")).is_err());
    assert!(creator_params(form("sourcePath=/d&pieceSize=1000")).is_err());
    assert!(creator_params(form("sourcePath=/d&pieceSize=8192")).is_err());

    let params = creator_params(form(
        "sourcePath=/d&pieceSize=65536&private=true&startSeeding=1&comment=hi\
         &trackers=http%3A%2F%2Fa%2Fann%7Chttp%3A%2F%2Fb%2Fann%0A%0Ahttp%3A%2F%2Fc%2Fann\
         &urlSeeds=http%3A%2F%2Fseed%2F%0A",
    ))
    .unwrap();
    assert_eq!(params.format, "v1");
    assert_eq!(params.piece_size, 65536);
    assert!(params.private);
    assert!(params.start_seeding);
    assert_eq!(params.comment, "hi");
    assert_eq!(
        params.trackers,
        vec!["http://a/ann", "http://b/ann", "http://c/ann"]
    );
    assert_eq!(params.url_seeds, vec!["http://seed/"]);
}

#[tokio::test]
async fn torrentcreator_add_task_honours_private_comment_seeds_and_file_path() {
    let (state, _session, output) = qbit_state().await;
    let src = output.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("data.bin"), vec![7u8; 40_000]).unwrap();
    let out_file = output.path().join("made.torrent");

    let body = serde_urlencoded::to_string([
        ("sourcePath", src.to_string_lossy().as_ref()),
        ("torrentFilePath", out_file.to_string_lossy().as_ref()),
        ("private", "true"),
        ("comment", "made by test"),
        ("trackers", "http://t1/announce|http://t2/announce"),
        ("urlSeeds", "http://seed.example/"),
        ("pieceSize", "16384"),
    ])
    .unwrap();
    let response =
        h_torrentcreator_add_task(axum::extract::State(state.clone()), Bytes::from(body))
            .await
            .into_response();
    let id = task_id_of(response);
    let task = wait_for_creator_task(&state, &id).await;
    assert_eq!(task.status, CreatorStatus::Finished, "{:?}", task.error);

    let bytes = task.torrent_bytes.unwrap();
    assert_eq!(std::fs::read(&out_file).unwrap(), bytes.as_ref());
    let meta = librtbit_core::torrent_metainfo::torrent_from_bytes(&bytes).unwrap();
    assert!(meta.info.data.private);
    assert_eq!(meta.info.data.piece_length, 16384);
    assert_eq!(
        meta.comment.as_ref().map(|c| c.as_ref()),
        Some(&b"made by test"[..])
    );
    let trackers: Vec<_> = meta.iter_announce().map(|t| t.as_ref().to_vec()).collect();
    assert_eq!(
        trackers,
        vec![
            b"http://t1/announce".to_vec(),
            b"http://t2/announce".to_vec()
        ]
    );
    let seeds: Vec<_> = meta
        .url_list
        .as_ref()
        .unwrap()
        .iter()
        .map(|u| u.as_ref().to_vec())
        .collect();
    assert_eq!(seeds, vec![b"http://seed.example/".to_vec()]);

    let status = creator_task_status(id.clone(), state.creator.get(&id).unwrap());
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["status"], "Finished");
    assert_eq!(json["progress"], 100.0);
    assert_eq!(json["private"], true);
    assert_eq!(json["pieceSize"], 16384);
    assert!(json["timeFinished"].is_string());
}

#[tokio::test]
async fn torrentcreator_start_seeding_adds_the_torrent_to_the_session() {
    let (state, session, output) = qbit_state().await;
    let src = output.path().join("seedme");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("a.bin"), vec![3u8; 20_000]).unwrap();

    let body = serde_urlencoded::to_string([
        ("sourcePath", src.to_string_lossy().as_ref()),
        ("startSeeding", "true"),
    ])
    .unwrap();
    let response =
        h_torrentcreator_add_task(axum::extract::State(state.clone()), Bytes::from(body))
            .await
            .into_response();
    let id = task_id_of(response);
    let task = wait_for_creator_task(&state, &id).await;
    assert_eq!(task.status, CreatorStatus::Finished, "{:?}", task.error);

    let info_hash =
        <librtbit_core::Id20 as std::str::FromStr>::from_str(task.info_hash.as_deref().unwrap())
            .unwrap();
    let handle = session
        .get(crate::api::TorrentIdOrHash::Hash(info_hash))
        .expect("created torrent should be managed");
    assert_eq!(handle.output_folder(), src);
}

#[tokio::test]
async fn torrentcreator_failed_task_and_unknown_ids() {
    let (state, _session, output) = qbit_state().await;
    let missing = output.path().join("does-not-exist");
    let body = format!("sourcePath={}", missing.to_string_lossy());
    let response =
        h_torrentcreator_add_task(axum::extract::State(state.clone()), Bytes::from(body))
            .await
            .into_response();
    let id = task_id_of(response);
    let task = wait_for_creator_task(&state, &id).await;
    assert_eq!(task.status, CreatorStatus::Failed);
    assert!(task.error.is_some());

    let file = h_torrentcreator_torrent_file(
        axum::extract::State(state.clone()),
        axum::extract::Query(RequiredTaskIdQuery {
            task_id: id.clone(),
        }),
    )
    .await;
    assert_eq!(file.status(), StatusCode::CONFLICT);
    let unknown = h_torrentcreator_torrent_file(
        axum::extract::State(state.clone()),
        axum::extract::Query(RequiredTaskIdQuery {
            task_id: "nope".into(),
        }),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let deleted = h_torrentcreator_delete_task(
        axum::extract::State(state.clone()),
        Bytes::from(format!("taskID={id}")),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let again = h_torrentcreator_delete_task(
        axum::extract::State(state.clone()),
        Bytes::from(format!("taskID={id}")),
    )
    .await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);

    let bad = h_torrentcreator_add_task(
        axum::extract::State(state.clone()),
        Bytes::from_static(b"sourcePath=/x&format=v2"),
    )
    .await
    .into_response();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    assert!(state.creator.all().is_empty());
}

#[test]
fn tracker_status_maps_to_qbit_codes() {
    use super::qbit_tracker_status;
    use tracker_comms::TrackerAnnounceState::*;
    assert_eq!(qbit_tracker_status(Disabled), 0);
    assert_eq!(qbit_tracker_status(NotContacted), 1);
    assert_eq!(qbit_tracker_status(Working), 2);
    assert_eq!(qbit_tracker_status(Updating), 3);
    assert_eq!(qbit_tracker_status(Error), 4);
}

#[test]
fn rate_limit_conversions_round_trip_and_treat_zero_as_unlimited() {
    use std::num::NonZeroU32;
    assert_eq!(super::limit_to_bps(0), None);
    assert_eq!(super::bps_to_u64(None), 0);
    assert_eq!(super::limit_to_bps(1024), NonZeroU32::new(1024));
    assert_eq!(super::bps_to_u64(NonZeroU32::new(1024)), 1024);
    // Values beyond u32 saturate rather than overflow.
    assert_eq!(super::limit_to_bps(u64::MAX), NonZeroU32::new(u32::MAX));
}

#[test]
fn per_torrent_limit_treats_non_positive_as_unlimited() {
    use std::num::NonZeroU32;
    assert_eq!(super::parse_torrent_limit(0), None);
    assert_eq!(super::parse_torrent_limit(-1), None);
    assert_eq!(super::parse_torrent_limit(-999), None);
    assert_eq!(super::parse_torrent_limit(2048), NonZeroU32::new(2048));
    assert_eq!(
        super::parse_torrent_limit(i64::MAX),
        NonZeroU32::new(u32::MAX)
    );
}

#[tokio::test]
async fn clientdata_survives_reopen_and_null_removes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("qbit-clientdata.json");
    let store = super::ClientDataStore::open(Some(path.clone()));
    let changes = serde_json::json!({"a": 1, "b": "x"});
    store
        .store(changes.as_object().unwrap().clone())
        .await
        .unwrap();
    let removal = serde_json::json!({"a": null});
    store
        .store(removal.as_object().unwrap().clone())
        .await
        .unwrap();

    let reopened = super::ClientDataStore::open(Some(path.clone()));
    assert_eq!(
        serde_json::Value::Object(reopened.load(&[]).await),
        serde_json::json!({"b": "x"})
    );

    // A corrupt file starts empty instead of failing the router.
    std::fs::write(&path, b"not json").unwrap();
    assert!(
        super::ClientDataStore::open(Some(path))
            .load(&[])
            .await
            .is_empty()
    );
}

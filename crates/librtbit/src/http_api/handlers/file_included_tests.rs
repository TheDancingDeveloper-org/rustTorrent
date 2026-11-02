//! Router-level tests for `POST /torrents/{id}/files/{index}/included`.

use std::{net::Ipv4Addr, sync::Arc};

use http::StatusCode;
use tower::ServiceExt;

use crate::{
    AddTorrent, AddTorrentOptions, Api, CreateTorrentOptions, ListenerMode, ManagedTorrent,
    Session, SessionOptions, create_torrent,
    http_api::{HttpApi, HttpApiOptions},
    listen::ListenerOptions,
    spawn_utils::BlockingSpawner,
};

/// The API router over a fresh session holding one paused two-file torrent.
async fn router_with_two_file_torrent() -> (axum::Router, Arc<ManagedTorrent>, tempfile::TempDir) {
    let output = tempfile::TempDir::with_prefix("file_included").unwrap();
    let payload = output.path().join("payload");
    std::fs::create_dir(&payload).unwrap();
    std::fs::write(payload.join("a.bin"), vec![0x41; 16 * 1024]).unwrap();
    std::fs::write(payload.join("b.bin"), vec![0x42; 16 * 1024]).unwrap();
    let torrent = create_torrent(
        &payload,
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

    let session = Session::new_with_opts(
        output.path().to_owned(),
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

    let api = Api::new(
        session,
        None,
        #[cfg(feature = "tracing-subscriber-utils")]
        None,
    );
    let state = Arc::new(HttpApi::new(api, Some(HttpApiOptions::default())));
    (super::make_api_router(state), handle, output)
}

fn post_included(id: usize, index: usize, body: &str) -> http::Request<axum::body::Body> {
    http::Request::builder()
        .method("POST")
        .uri(format!("/torrents/{id}/files/{index}/included"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_owned()))
        .unwrap()
}

fn selection(handle: &ManagedTorrent) -> Vec<usize> {
    let mut files = handle.only_files().unwrap_or_else(|| vec![0, 1]);
    files.sort_unstable();
    files
}

#[tokio::test]
async fn excluding_then_including_a_file_updates_the_selection() {
    let (router, handle, _output) = router_with_two_file_torrent().await;
    let id = handle.id();

    let response = router
        .clone()
        .oneshot(post_included(id, 1, r#"{"included":false}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(selection(&handle), vec![0]);

    let response = router
        .oneshot(post_included(id, 1, r#"{"included":true}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(selection(&handle), vec![0, 1]);
}

#[tokio::test]
async fn out_of_range_file_index_is_a_bad_request() {
    let (router, handle, _output) = router_with_two_file_torrent().await;

    let response = router
        .oneshot(post_included(handle.id(), 2, r#"{"included":false}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(selection(&handle), vec![0, 1]);
}

#[tokio::test]
async fn malformed_body_is_rejected() {
    let (router, handle, _output) = router_with_two_file_torrent().await;

    let response = router
        .oneshot(post_included(handle.id(), 0, r#"{"included":"yes"}"#))
        .await
        .unwrap();
    assert!(response.status().is_client_error(), "{}", response.status());
    assert_eq!(selection(&handle), vec![0, 1]);
}

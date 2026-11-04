//! Route-level tests for `check_remote_add_url`: each guarded add path must
//! refuse a `file://` URL naming a readable, valid `.torrent`. Without the
//! guard the session would read the file and add the torrent, so these fail
//! if the guard is removed.

use std::{net::Ipv4Addr, path::PathBuf, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use bytes::Bytes;

use crate::{
    Api, CreateTorrentOptions, ListenerMode, Session, SessionOptions, create_torrent,
    http_api::{HttpApi, HttpApiOptions, timeout::Timeout},
    http_api_types::TorrentAddQueryParams,
    listen::ListenerOptions,
    rss::{db::RssDatabase, models::RssItem},
    spawn_utils::BlockingSpawner,
};

use super::{other::h_resolve_magnet, rss::h_rss_item_download, torrents::h_torrents_post};

pub(crate) const REFUSAL: &str = "only magnet, http and https";

/// Writes a valid `.torrent` into `dir` and returns its `file://` URL.
pub(crate) async fn valid_torrent_file_url(dir: &std::path::Path) -> String {
    let payload = dir.join("payload.bin");
    std::fs::write(&payload, vec![7u8; 4096]).unwrap();
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
    let path: PathBuf = dir.join("readable.torrent");
    std::fs::write(&path, torrent).unwrap();
    url::Url::from_file_path(&path).unwrap().to_string()
}

pub(crate) async fn session_in(dir: &std::path::Path) -> Arc<Session> {
    Session::new_with_opts(
        dir.to_owned(),
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
    .unwrap()
}

struct Fixture {
    state: Arc<HttpApi>,
    session: Arc<Session>,
    db: Arc<parking_lot::Mutex<RssDatabase>>,
    file_url: String,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let dir = tempfile::TempDir::with_prefix("remote_add_guard").unwrap();
    let file_url = valid_torrent_file_url(dir.path()).await;
    let session = session_in(dir.path()).await;
    let api = Api::new(
        session.clone(),
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
    Fixture {
        state,
        session,
        db,
        file_url,
        _dir: dir,
    }
}

async fn assert_refused(response: axum::response::Response, session: &Session) {
    assert!(
        !response.status().is_success(),
        "expected refusal, got {}",
        response.status()
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains(REFUSAL), "unexpected body: {body}");
    assert_eq!(session.with_torrents(|t| t.count()), 0);
}

fn timeout<const D: usize, const M: usize>() -> Timeout<D, M> {
    Timeout(std::time::Duration::from_secs(30))
}

#[tokio::test]
async fn post_torrents_refuses_file_url_when_is_url_is_forced() {
    let f = fixture().await;
    let response = h_torrents_post(
        State(f.state.clone()),
        Query(TorrentAddQueryParams {
            is_url: Some(true),
            ..Default::default()
        }),
        timeout(),
        Bytes::from(f.file_url.clone()),
    )
    .await
    .into_response();
    assert_refused(response, &f.session).await;
}

#[tokio::test]
async fn post_torrents_refuses_file_url_when_format_is_guessed() {
    let f = fixture().await;
    let response = h_torrents_post(
        State(f.state.clone()),
        Query(TorrentAddQueryParams::default()),
        timeout(),
        Bytes::from(f.file_url.clone()),
    )
    .await
    .into_response();
    assert_refused(response, &f.session).await;
}

#[tokio::test]
async fn resolve_magnet_refuses_file_url() {
    let f = fixture().await;
    let response = h_resolve_magnet(
        State(f.state.clone()),
        timeout(),
        http::HeaderMap::new(),
        f.file_url.clone(),
    )
    .await
    .into_response();
    assert_refused(response, &f.session).await;
}

#[tokio::test]
async fn rss_item_download_refuses_file_url() {
    let f = fixture().await;
    f.db.lock()
        .rss_item_upsert(&RssItem {
            id: "item-1".into(),
            feed_name: "feed".into(),
            title: "Hostile".into(),
            url: Some(f.file_url.clone()),
            published_at: None,
            first_seen_at: chrono::Utc::now(),
            downloaded: false,
            downloaded_at: None,
            category: None,
            size_bytes: 0,
        })
        .unwrap();
    let response = h_rss_item_download(State(f.state.clone()), Path("item-1".into()))
        .await
        .into_response();
    assert_refused(response, &f.session).await;
    assert!(
        !f.db
            .lock()
            .rss_item_get("item-1")
            .unwrap()
            .unwrap()
            .downloaded
    );
}

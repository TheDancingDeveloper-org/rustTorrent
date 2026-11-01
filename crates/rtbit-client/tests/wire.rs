use std::path::PathBuf;

use rtbit_client::{Client, Error};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture")
}

#[tokio::test]
async fn login_then_lists_torrents_and_stats_from_fixtures() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .and(body_json(
            serde_json::json!({"username": "ada", "password": "secret"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/torrents"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("torrents.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/stats"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("stats.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    let tokens = client.login("ada", "secret").await.unwrap();
    assert_eq!(tokens.access_token, "access-1");
    let list = client.torrents().await.unwrap();
    assert_eq!(list.total, 1);
    assert_eq!(list.torrents[0].name.as_deref(), Some("example"));
    assert_eq!(list.torrents[0].tags.as_ref().map(Vec::len), Some(1));
    let stats = client.stats().await.unwrap();
    assert_eq!(stats.counters.fetched_bytes, 100);
    assert!((stats.download_speed.mbps - 1.5).abs() < f64::EPSILON);
    assert_eq!(stats.peers.live, 3);
    assert_eq!(stats.connections.tcp.v4.successes, 1);
}

#[tokio::test]
async fn a_401_triggers_exactly_one_refresh_and_one_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/torrents"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/refresh"))
        .and(body_json(serde_json::json!({"refresh_token": "refresh-1"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("refresh.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/torrents"))
        .and(header("authorization", "Bearer access-2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("torrents.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    let list = client.torrents().await.unwrap();
    assert_eq!(list.total, 1);
    server.verify().await;
}

#[tokio::test]
async fn a_second_401_after_refresh_is_unauthorised_and_does_not_refresh_again() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/torrents"))
        .respond_with(ResponseTemplate::new(401))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/refresh"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("refresh.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    let err = client.torrents().await.unwrap_err();
    assert!(matches!(err, Error::Unauthorised));
    server.verify().await;
}

#[tokio::test]
async fn setup_required_is_not_set_up() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(ResponseTemplate::new(403).set_body_string("setup required"))
        .mount(&server)
        .await;
    let mut client = Client::new(server.uri()).unwrap();
    let err = client.login("ada", "secret").await.unwrap_err();
    assert!(matches!(err, Error::NotSetUp));
}

#[tokio::test]
async fn start_pause_and_delete_post_the_native_routes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    for route in [
        "/torrents/7/start",
        "/torrents/7/pause",
        "/torrents/7/delete",
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .and(header("authorization", "Bearer access-1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
            .expect(1)
            .mount(&server)
            .await;
    }

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    client.start_torrent(7).await.unwrap();
    client.pause_torrent(7).await.unwrap();
    client.delete_torrent(7).await.unwrap();
}

#[tokio::test]
async fn n_polls_log_in_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/torrents"))
        .and(wiremock::matchers::query_param("with_stats", "true"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("torrents.json"), "application/json"),
        )
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/stats"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("stats.json"), "application/json"),
        )
        .expect(3)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    for _ in 0..3 {
        client.torrents_with_stats().await.unwrap();
        client.stats().await.unwrap();
    }
    server.verify().await;
}

#[tokio::test]
async fn add_magnet_posts_the_query_and_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Example";
    Mock::given(method("POST"))
        .and(path("/torrents"))
        .and(wiremock::matchers::query_param("is_url", "true"))
        .and(wiremock::matchers::query_param("paused", "true"))
        .and(wiremock::matchers::query_param(
            "output_folder",
            "/data/incoming",
        ))
        .and(wiremock::matchers::query_param("category", "movies"))
        .and(wiremock::matchers::body_string(magnet))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    client
        .add_magnet(magnet, Some("/data/incoming"), Some("movies"), true)
        .await
        .unwrap();
    server.verify().await;
}

#[tokio::test]
async fn add_torrent_file_posts_the_bytes_without_is_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/torrents"))
        .and(wiremock::matchers::query_param_is_missing("is_url"))
        .and(wiremock::matchers::body_bytes(b"d8:announce0:e".as_slice()))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    client
        .add_torrent_file(b"d8:announce0:e", None, None, false)
        .await
        .unwrap();
    server.verify().await;
}

#[tokio::test]
async fn detail_reads_files_trackers_peers_and_pieces() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/torrents/7"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("torrent.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/torrents/7/trackers"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("trackers.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/torrents/7/peer_stats"))
        .and(wiremock::matchers::query_param("state", "live"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("peers.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    // 0b1010_0000 then 0b1000_0000: pieces 0, 2 and 8 of 10.
    Mock::given(method("GET"))
        .and(path("/torrents/7/haves"))
        .and(header("accept", "application/octet-stream"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-bitfield-len", "10")
                .set_body_raw(vec![0b1010_0000, 0b1000_0000], "application/octet-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    let torrent = client.torrent(7).await.unwrap();
    let files = torrent.files.expect("files");
    assert_eq!(files.len(), 2);
    assert!(files[0].included);
    assert!(!files[1].included);
    let trackers = client.trackers(7).await.unwrap();
    assert_eq!(trackers.trackers[0].seeders, Some(4));
    let peers = client.peers(7).await.unwrap();
    assert_eq!(peers.peers[0].addr, "1.2.3.4:6881");
    assert_eq!(peers.peers[0].fetched_bytes, 20);
    let pieces = client.haves(7).await.unwrap();
    assert_eq!(pieces.have_count(), 3);
    assert!(pieces.have[0] && !pieces.have[1] && pieces.have[8]);
    server.verify().await;
}

#[tokio::test]
async fn file_selection_and_tracker_add_post_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/torrents/7/update_only_files"))
        .and(body_json(serde_json::json!({"only_files": [0, 2]})))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/torrents/7/trackers"))
        .and(body_json(
            serde_json::json!({"trackers": ["http://new.example/announce"]}),
        ))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    client.update_only_files(7, &[0, 2]).await.unwrap();
    client
        .add_trackers(7, &["http://new.example/announce".into()])
        .await
        .unwrap();
    server.verify().await;
}

#[tokio::test]
async fn settings_reads_the_five_tabs_and_posts_only_the_changed_section() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    for (route, file) in [
        ("/session/downloads", "downloads.json"),
        ("/session/advanced", "advanced.json"),
        ("/session/network", "network.json"),
        ("/session/proxy", "proxy.json"),
        ("/session/ip-filter", "ip-filter.json"),
        ("/session/folders", "folders.json"),
        ("/torrents/limits", "limits.json"),
        ("/speed/alt", "alt.json"),
        ("/speed/schedule", "schedule.json"),
        ("/torrents/seed_limits", "seed.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .and(header("authorization", "Bearer access-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(fixture(file), "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/torrents/seed_limits"))
        .and(body_json(
            serde_json::json!({"ratio_limit": 1.5, "time_limit_secs": null}),
        ))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    let snap = client.settings().await.unwrap();
    assert!(snap.network.dht_enabled);
    assert_eq!(
        snap.proxy.proxy_url.as_deref(),
        Some("socks5://user:***@127.0.0.1:1080")
    );
    assert_eq!(snap.folders.download_folder, "/data/in");
    assert_eq!(snap.seed_limits.ratio_limit, Some(2.0));
    // The masked proxy is not posted back. Only the changed seed limit is.
    client
        .set_seed_limits(&rtbit_client::SeedLimits {
            ratio_limit: Some(1.5),
            time_limit_secs: None,
        })
        .await
        .unwrap();
    server.verify().await;
}

#[test]
fn network_reads_saved_values_status_and_can_save() {
    let old: rtbit_client::NetworkConfig = serde_json::from_str(&fixture("network.json")).unwrap();
    assert!(old.can_save, "an older daemon without can_save is savable");
    assert_eq!(old.pending, None);
    assert_eq!(old.pending_status, None);

    let net: rtbit_client::NetworkConfig =
        serde_json::from_str(&fixture("network-pending.json")).unwrap();
    assert_eq!(
        net.pending_status,
        Some(rtbit_client::SavedNetworkStatus::FellBack)
    );
    let pending = net.pending.unwrap();
    assert_eq!(pending.listen_ip.as_deref(), Some("192.168.1.5"));
    assert_eq!(pending.listen_port, Some(51413));
    assert_eq!(pending.dht_enabled, Some(false));
    assert_eq!(pending.lsd_enabled, None);

    let unknown: rtbit_client::NetworkConfig = serde_json::from_str(
        r#"{"listen_ip":null,"listen_port":null,"dht_enabled":false,"pex_enabled":true,"lsd_enabled":false,"upnp_port_forward":false,"encryption":null,"pending_status":"later","can_save":false}"#,
    )
    .unwrap();
    assert_eq!(
        unknown.pending_status,
        Some(rtbit_client::SavedNetworkStatus::Unknown)
    );
    assert!(!unknown.can_save);
}

#[tokio::test]
async fn network_set_posts_only_given_fields_and_reset_posts_reset() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/session/network"))
        .and(body_json(
            serde_json::json!({"listen_port": 51413, "dht_enabled": false}),
        ))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"restart_required":true,"message":"saved"}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/session/network"))
        .and(body_json(serde_json::json!({"reset": true})))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    client
        .set_network(&rtbit_client::NetworkSettings {
            listen_port: Some(51413),
            dht_enabled: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    client.reset_network().await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn network_save_without_persistence_is_a_daemon_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/session/network"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;
    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    let err = client.reset_network().await.unwrap_err();
    assert!(matches!(err, rtbit_client::Error::Daemon { status, .. } if status.as_u16() == 409));
}

#[tokio::test]
async fn rss_lists_feeds_and_items_and_posts_one_download() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    for (route, file) in [
        ("/rss/feeds", "rss-feeds.json"),
        ("/rss/items", "rss-items.json"),
        ("/rss/rules", "rss-rules.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .and(header("authorization", "Bearer access-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(fixture(file), "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/rss/items/ep-1/download"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    let snap = client.rss().await.unwrap();
    assert_eq!(snap.feeds.len(), 1);
    assert_eq!(snap.feeds[0].name, "Show");
    assert_eq!(snap.items[0].title, "Episode One");
    assert!(!snap.items[0].downloaded);
    assert_eq!(snap.rules.len(), 1);
    assert_eq!(snap.rules[0].id, "r-1");
    assert_eq!(snap.rules[0].feed_names, ["Show"]);
    assert_eq!(snap.rules[0].category.as_deref(), Some("tv"));
    assert_eq!(snap.rules[0].priority, 2);
    assert!(snap.rules[0].enabled);
    assert!(
        snap.rules[0].track_episodes,
        "a daemon without the field tracks episodes"
    );
    client.download_rss_item("ep-1").await.unwrap();
    server.verify().await;
}

fn rule_body() -> rtbit_client::RssRuleBody {
    rtbit_client::RssRuleBody {
        name: "Show 1080p".into(),
        feed_names: vec!["Show".into()],
        category: Some("tv".into()),
        priority: 2,
        match_regex: "Show.*1080p".into(),
        enabled: true,
        track_episodes: false,
    }
}

async fn logged_in(server: &MockServer) -> Client {
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(server)
        .await;
    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    client
}

#[tokio::test]
async fn rss_rule_add_update_and_delete_use_the_rule_routes() {
    let server = MockServer::start().await;
    let expected = serde_json::json!({
        "name": "Show 1080p",
        "feed_names": ["Show"],
        "category": "tv",
        "priority": 2,
        "match_regex": "Show.*1080p",
        "enabled": true,
        "track_episodes": false,
    });
    Mock::given(method("POST"))
        .and(path("/rss/rules"))
        .and(header("authorization", "Bearer access-1"))
        .and(body_json(expected.clone()))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(r#"{"status":true}"#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rss/rules/r%201"))
        .and(header("authorization", "Bearer access-1"))
        .and(body_json(expected))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(r#"{"status":true}"#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/rss/rules/r-1"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(r#"{"status":true}"#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut client = logged_in(&server).await;
    client.add_rss_rule(&rule_body()).await.unwrap();
    client.update_rss_rule("r 1", &rule_body()).await.unwrap();
    client.delete_rss_rule("r-1").await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn rss_rule_episodes_reads_the_rule_history() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rss/rules/r%201/episodes"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"[{"show":"show name","season":1,"episode":2,"title":"Show Name S01E02",
                "matched_at":"2026-10-31T10:00:00Z","torrent_id":4},
               {"show":"show name","season":1,"episode":1,"title":"Show Name S01E01",
                "matched_at":"2026-10-30T10:00:00Z","torrent_id":null}]"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = logged_in(&server).await;
    let episodes = client.rss_rule_episodes("r 1").await.unwrap();
    assert_eq!(episodes.len(), 2);
    assert_eq!(episodes[0].episode, 2);
    assert_eq!(episodes[0].torrent_id, Some(4));
    assert_eq!(episodes[1].torrent_id, None);
    assert_eq!(episodes[1].title, "Show Name S01E01");
    server.verify().await;
}

#[tokio::test]
async fn a_rejected_rule_pattern_keeps_the_daemon_reason() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rss/rules"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"error_kind":"internal_error","human_readable":"invalid filter pattern: invalid regex: unclosed group","status":400}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = logged_in(&server).await;
    let err = client.add_rss_rule(&rule_body()).await.unwrap_err();
    match err {
        Error::Daemon {
            message: Some(text),
            ..
        } => assert_eq!(
            text,
            "invalid filter pattern: invalid regex: unclosed group"
        ),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn a_401_on_rule_delete_refreshes_once_and_retries() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/rss/rules/r-1"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/refresh"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("refresh.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/rss/rules/r-1"))
        .and(header("authorization", "Bearer access-2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(r#"{"status":true}"#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut client = logged_in(&server).await;
    client.delete_rss_rule("r-1").await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn set_file_included_posts_one_file_and_surfaces_bad_index() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/torrents/7/files/2/included"))
        .and(body_json(serde_json::json!({"included": true})))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/torrents/7/files/9/included"))
        .respond_with(ResponseTemplate::new(400))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    client.set_file_included(7, 2, true).await.unwrap();
    assert!(client.set_file_included(7, 9, true).await.is_err());
    server.verify().await;
}

#[test]
fn advanced_decodes_legacy_and_full_bodies() {
    // An older daemon sends only the three timeouts. The new knobs read as None.
    let legacy: rtbit_client::AdvancedConfig =
        serde_json::from_str(&fixture("advanced-legacy.json")).unwrap();
    assert_eq!(legacy.connect_timeout_secs, Some(10));
    assert_eq!(legacy.keep_alive_secs, Some(120));
    assert_eq!(legacy.fastresume_validation_denom, None);

    let full: rtbit_client::AdvancedConfig =
        serde_json::from_str(&fixture("advanced.json")).unwrap();
    assert_eq!(full.upload_slots, Some(4));
    assert_eq!(full.magnet_resolution_timeout_secs, Some(120));
    assert_eq!(full.max_concurrent_handshakes, Some(64));
    assert_eq!(full.fastresume_validation_denom, Some(50));
}

#[test]
fn advanced_post_body_omits_unset_knobs() {
    // A legacy-only body must serialise to the three-field shape an old daemon expects.
    let legacy = rtbit_client::AdvancedConfig {
        connect_timeout_secs: Some(10),
        read_write_timeout_secs: Some(30),
        keep_alive_secs: Some(120),
        ..Default::default()
    };
    let value = serde_json::to_value(&legacy).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"connect_timeout_secs": 10, "read_write_timeout_secs": 30, "keep_alive_secs": 120})
    );
}

#[tokio::test]
async fn set_advanced_round_trips_body_and_reads_restart_flag() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    let body = rtbit_client::AdvancedConfig {
        connect_timeout_secs: Some(10),
        read_write_timeout_secs: Some(30),
        keep_alive_secs: Some(120),
        upload_slots: Some(4),
        magnet_resolution_timeout_secs: Some(120),
        max_concurrent_handshakes: Some(64),
        fastresume_validation_denom: Some(50),
    };
    Mock::given(method("POST"))
        .and(path("/session/advanced"))
        .and(header("authorization", "Bearer access-1"))
        .and(body_json(serde_json::to_value(&body).unwrap()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(r#"{"restart_required": true}"#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    let saved = client.set_advanced(&body).await.unwrap();
    assert!(saved.restart_required);
    server.verify().await;
}

#[tokio::test]
async fn set_advanced_treats_empty_reply_as_no_restart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/session/advanced"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    let saved = client
        .set_advanced(&rtbit_client::AdvancedConfig {
            connect_timeout_secs: Some(10),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!saved.restart_required);
    server.verify().await;
}

/// The full fixture is the daemon's `GET /session/advanced` field set. The
/// daemon crate checks the same file, so a field added on one side only fails
/// a test on the other. `peer_limit` and `concurrent_init_limit` belong to
/// `/torrents/limits` and must not appear here: the daemon ignores them.
#[test]
fn advanced_field_set_matches_the_shared_fixture() {
    use std::collections::BTreeSet;

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct StrictCopy {
        connect_timeout_secs: Option<u64>,
        read_write_timeout_secs: Option<u64>,
        keep_alive_secs: Option<u64>,
        upload_slots: Option<u64>,
        magnet_resolution_timeout_secs: Option<u64>,
        max_concurrent_handshakes: Option<u64>,
        fastresume_validation_denom: Option<u64>,
    }
    let text = fixture("advanced.json");
    serde_json::from_str::<StrictCopy>(&text).expect("fixture has exactly the daemon fields");

    // Build the sent key set from a fully populated literal, not by decoding the
    // fixture: a field missing from the fixture decodes as `None`, and
    // `skip_serializing_if` then drops it, so the comparison would never see it.
    // The literal has no `..Default`, so a new client field fails to compile here.
    let full = rtbit_client::AdvancedConfig {
        connect_timeout_secs: Some(10),
        read_write_timeout_secs: Some(30),
        keep_alive_secs: Some(120),
        upload_slots: Some(4),
        magnet_resolution_timeout_secs: Some(120),
        max_concurrent_handshakes: Some(64),
        fastresume_validation_denom: Some(50),
    };
    // Exhaustive destructure, no `..`: adding a client field breaks compilation
    // until this test and the shared fixture list it.
    let rtbit_client::AdvancedConfig {
        connect_timeout_secs: _,
        read_write_timeout_secs: _,
        keep_alive_secs: _,
        upload_slots: _,
        magnet_resolution_timeout_secs: _,
        max_concurrent_handshakes: _,
        fastresume_validation_denom: _,
    } = &full;
    let sent: BTreeSet<String> = serde_json::to_value(&full)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let expected: BTreeSet<String> = serde_json::from_str::<serde_json::Value>(&text)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(sent, expected, "client body keys differ from the daemon's");
    assert!(!sent.contains("peer_limit") && !sent.contains("concurrent_init_limit"));
}

#[tokio::test]
async fn set_limits_posts_peer_and_init_limits_to_the_limits_route() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json"),
        )
        .mount(&server)
        .await;
    let limits = rtbit_client::RateLimits {
        upload_bps: None,
        download_bps: Some(1_000_000),
        peer_limit: Some(50),
        concurrent_init_limit: Some(4),
    };
    Mock::given(method("POST"))
        .and(path("/torrents/limits"))
        .and(header("authorization", "Bearer access-1"))
        .and(body_json(serde_json::to_value(&limits).unwrap()))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    let mut client = Client::new(server.uri()).unwrap();
    client.login("ada", "secret").await.unwrap();
    client.set_limits(&limits).await.unwrap();
    server.verify().await;
}

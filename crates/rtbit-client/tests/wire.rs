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

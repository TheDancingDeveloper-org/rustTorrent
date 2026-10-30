//! Network settings saved from the web UI must never stop the daemon from
//! starting (#201 review): a value that cannot bind falls back to the startup
//! flags, and `ignore_saved_network` skips the saved values entirely.

use std::{net::SocketAddr, path::Path};

use tempfile::TempDir;

use crate::{
    ListenerMode, ListenerOptions, SavedNetworkStatus, Session, SessionOptions,
    session::SessionPersistenceConfig, session_persistence::NetworkSettings,
};

fn opts(persist: &Path, cli_listen: SocketAddr, ignore_saved_network: bool) -> SessionOptions {
    SessionOptions {
        disable_dht: true,
        disable_dht_persistence: true,
        disable_trackers: true,
        disable_local_service_discovery: true,
        ignore_saved_network,
        persistence: Some(SessionPersistenceConfig::Json {
            folder: Some(persist.to_owned()),
        }),
        listen: Some(ListenerOptions {
            mode: ListenerMode::TcpOnly,
            listen_addr: cli_listen,
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn save(dir: &TempDir, persist: &Path, net: NetworkSettings) {
    let session = Session::new_with_opts(
        dir.path().to_owned(),
        opts(persist, "127.0.0.1:0".parse().unwrap(), false),
    )
    .await
    .unwrap();
    session.clear_pending_network();
    session.set_pending_network(net);
    session.persist_session_settings().await.unwrap();
    session.stop().await;
}

async fn start(dir: &TempDir, persist: &Path, ignore: bool) -> std::sync::Arc<Session> {
    Session::new_with_opts(
        dir.path().to_owned(),
        opts(persist, "127.0.0.1:0".parse().unwrap(), ignore),
    )
    .await
    .expect("a bad saved network value must not stop the session starting")
}

#[tokio::test]
async fn saved_listen_ip_that_cannot_bind_falls_back_to_startup_flags() {
    let dir = TempDir::new().unwrap();
    let persist = dir.path().join("persist");
    // 192.0.2.1 is TEST-NET-1: never an address of this host.
    save(
        &dir,
        &persist,
        NetworkSettings {
            listen_ip: Some("192.0.2.1".parse().unwrap()),
            ..Default::default()
        },
    )
    .await;

    let session = start(&dir, &persist, false).await;
    assert_eq!(
        session.listen_addr().unwrap().ip(),
        "127.0.0.1".parse::<std::net::IpAddr>().unwrap()
    );
    assert_eq!(
        session.saved_network_status(),
        Some(SavedNetworkStatus::FellBack)
    );
    // The bad value is kept so the web UI can show and reset it.
    assert!(session.pending_network().is_some());
    session.stop().await;
}

#[tokio::test]
async fn ignore_saved_network_uses_startup_flags_and_keeps_saved_values() {
    let dir = TempDir::new().unwrap();
    let persist = dir.path().join("persist");
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy_port = held.local_addr().unwrap().port();
    save(
        &dir,
        &persist,
        NetworkSettings {
            listen_port: Some(busy_port),
            ..Default::default()
        },
    )
    .await;

    let session = start(&dir, &persist, true).await;
    assert_ne!(session.listen_addr().unwrap().port(), busy_port);
    assert_eq!(
        session.saved_network_status(),
        Some(SavedNetworkStatus::Ignored)
    );
    assert_eq!(
        session.pending_network().unwrap().listen_port,
        Some(busy_port)
    );
    session.stop().await;
}

#[tokio::test]
async fn saved_values_apply_and_reset_clears_them() {
    let dir = TempDir::new().unwrap();
    let persist = dir.path().join("persist");
    let free_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    save(
        &dir,
        &persist,
        NetworkSettings {
            listen_port: Some(free_port),
            ..Default::default()
        },
    )
    .await;

    let session = start(&dir, &persist, false).await;
    assert_eq!(session.listen_addr().unwrap().port(), free_port);
    assert_eq!(
        session.saved_network_status(),
        Some(SavedNetworkStatus::Applied)
    );
    session.clear_pending_network();
    session.persist_session_settings().await.unwrap();
    session.stop().await;

    let session = start(&dir, &persist, false).await;
    assert_eq!(session.pending_network(), None);
    assert_eq!(session.saved_network_status(), None);
    session.stop().await;
}

//! End-to-end checks of the auth middleware on a real in-process server: the
//! pre-setup lock, the read-only role and the bearer paths. The unit tests in
//! `auth.rs` cover the stores; these cover how the router applies them.

use std::{net::Ipv4Addr, sync::Arc, time::Duration};

use librqbit_dualstack_sockets::{BindOpts, TcpListener};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::{
    Api, ListenerMode, Session, SessionOptions,
    http_api::{
        HttpApi, HttpApiOptions,
        auth::{ApiKeyStore, CredentialStore, Role},
    },
    listen::ListenerOptions,
};

struct Server {
    addr: std::net::SocketAddr,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    _dirs: Vec<tempfile::TempDir>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start(opts: HttpApiOptions, dirs: Vec<tempfile::TempDir>) -> Server {
    let output = tempfile::TempDir::with_prefix("auth_mw_session").unwrap();
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
    let api = Api::new(
        session,
        None,
        #[cfg(feature = "tracing-subscriber-utils")]
        None,
    );
    let http_api = HttpApi::new(api, Some(opts));
    let listener = TcpListener::bind_tcp(
        (Ipv4Addr::LOCALHOST, 0).into(),
        BindOpts {
            request_dualstack: false,
            reuseport: false,
            device: None,
        },
    )
    .unwrap();
    let addr = listener.socket().local_addr().unwrap();
    let task = tokio::spawn(http_api.make_http_api_and_run(listener, None));
    let mut dirs = dirs;
    dirs.push(output);
    Server {
        addr,
        task,
        _dirs: dirs,
    }
}

/// Sends a raw HTTP/1.1 request and returns the status code and body.
async fn send(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    authorization: Option<&str>,
) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let auth = authorization
        .map(|v| format!("Authorization: {v}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut response))
        .await
        .expect("response timed out")
        .unwrap();
    let text = String::from_utf8_lossy(&response).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status line in response: {text:?}"));
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn before_setup_only_the_setup_paths_are_reachable() {
    let config = tempfile::TempDir::with_prefix("auth_mw_presetup").unwrap();
    let credentials = CredentialStore::load(config.path().to_owned()).unwrap();
    assert!(!credentials.has_credentials());
    let server = start(
        HttpApiOptions {
            credential_store: Some(Arc::new(credentials)),
            ..Default::default()
        },
        vec![config],
    )
    .await;

    let (status, body) = send(server.addr, "GET", "/torrents", None).await;
    assert_eq!(status, 403);
    assert!(body.contains("setup required"), "body: {body:?}");
    assert_eq!(send(server.addr, "GET", "/auth/status", None).await.0, 200);
    assert_eq!(send(server.addr, "GET", "/healthz", None).await.0, 200);
}

#[tokio::test]
async fn a_read_only_api_key_can_read_but_not_write() {
    let config = tempfile::TempDir::with_prefix("auth_mw_keys").unwrap();
    let credentials = CredentialStore::load(config.path().to_owned()).unwrap();
    credentials.set_credentials("admin", "admin-pass").unwrap();
    let keys = ApiKeyStore::load(config.path().to_owned()).unwrap();
    let (_, read_only_key) = keys.create("dashboard", Role::ReadOnly).unwrap();
    let (_, admin_key) = keys.create("automation", Role::Admin).unwrap();
    let server = start(
        HttpApiOptions {
            credential_store: Some(Arc::new(credentials)),
            api_keys: Some(Arc::new(keys)),
            ..Default::default()
        },
        vec![config],
    )
    .await;

    let ro = format!("Bearer {read_only_key}");
    assert_eq!(
        send(server.addr, "GET", "/torrents", Some(&ro)).await.0,
        200
    );
    let (status, body) = send(server.addr, "POST", "/torrents/0/pause", Some(&ro)).await;
    assert_eq!(status, 403);
    assert!(body.contains("this account is read-only"), "body: {body:?}");

    // An admin key passes the role check (the torrent doesn't exist, so the
    // handler answers, not the middleware)
    let admin = format!("Bearer {admin_key}");
    let (status, _) = send(server.addr, "POST", "/torrents/0/pause", Some(&admin)).await;
    assert_ne!(status, 401);
    assert_ne!(status, 403);
}

#[tokio::test]
async fn unknown_bearer_tokens_and_missing_credentials_are_refused() {
    let config = tempfile::TempDir::with_prefix("auth_mw_bearer").unwrap();
    let credentials = CredentialStore::load(config.path().to_owned()).unwrap();
    credentials.set_credentials("admin", "admin-pass").unwrap();
    let keys = ApiKeyStore::load(config.path().to_owned()).unwrap();
    let server = start(
        HttpApiOptions {
            credential_store: Some(Arc::new(credentials)),
            api_keys: Some(Arc::new(keys)),
            ..Default::default()
        },
        vec![config],
    )
    .await;

    for token in ["Bearer rtb_not-a-real-key", "Bearer not-a-session-token"] {
        assert_eq!(
            send(server.addr, "GET", "/torrents", Some(token)).await.0,
            401,
            "{token}"
        );
    }
    assert_eq!(send(server.addr, "GET", "/torrents", None).await.0, 401);
}

#[tokio::test]
async fn read_only_basic_auth_user_is_limited_to_reads() {
    use base64::Engine;

    let config = tempfile::TempDir::with_prefix("auth_mw_basic").unwrap();
    let credentials = CredentialStore::load(config.path().to_owned()).unwrap();
    credentials.set_credentials("admin", "admin-pass").unwrap();
    credentials
        .set_read_only_user("viewer", "viewer-pass")
        .unwrap();
    let server = start(
        HttpApiOptions {
            credential_store: Some(Arc::new(credentials)),
            ..Default::default()
        },
        vec![config],
    )
    .await;

    let basic = |user_pass: &str| {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(user_pass)
        )
    };
    let viewer = basic("viewer:viewer-pass");
    assert_eq!(
        send(server.addr, "GET", "/torrents", Some(&viewer)).await.0,
        200
    );
    assert_eq!(
        send(server.addr, "POST", "/torrents/0/pause", Some(&viewer))
            .await
            .0,
        403
    );
    assert_eq!(
        send(
            server.addr,
            "GET",
            "/torrents",
            Some(&basic("viewer:wrong"))
        )
        .await
        .0,
        401
    );
}

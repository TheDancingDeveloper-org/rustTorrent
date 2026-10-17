use axum::response::IntoResponse;
use http::{HeaderValue, StatusCode, header};

/// Unauthenticated liveness probe for `GET /healthz`.
///
/// Answers 200 whenever the HTTP server is serving requests. It deliberately
/// does not touch the session or the torrent state, so a probe can never fail
/// because of a slow torrent operation. Route registration and the auth
/// allowlist live in the HTTP API router.
pub(crate) async fn h_healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            ),
        ],
        "ok",
    )
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, sync::Arc, time::Duration};

    use axum::response::IntoResponse;
    use http::{StatusCode, header};
    use librqbit_dualstack_sockets::{BindOpts, TcpListener};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    use super::h_healthz;
    use crate::{
        Api, ListenerMode, Session, SessionOptions,
        http_api::{HttpApi, HttpApiOptions, auth::CredentialStore},
        listen::ListenerOptions,
    };

    #[tokio::test]
    async fn healthz_returns_ok_plain_text_uncacheable() {
        let resp = h_healthz().await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            resp.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );

        let body = axum::body::to_bytes(resp.into_body(), 16).await.unwrap();
        assert_eq!(&body[..], b"ok");
    }

    /// Sends a raw HTTP/1.1 GET and returns the status code. Raw TCP keeps the test free
    /// of an extra HTTP client dependency.
    async fn get_status(addr: std::net::SocketAddr, path: &str) -> u16 {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut response))
            .await
            .expect("response timed out")
            .unwrap();
        let text = String::from_utf8_lossy(&response);
        let status_line = text.lines().next().unwrap_or_default();
        status_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or_else(|| panic!("no status line in response: {status_line:?}"))
    }

    #[tokio::test]
    async fn healthz_is_reachable_without_credentials_when_auth_is_enabled() {
        let config_dir = tempfile::TempDir::with_prefix("healthz_auth").unwrap();
        let credentials = CredentialStore::load(config_dir.path().to_owned()).unwrap();
        credentials
            .set_credentials("admin", "not-the-probe")
            .unwrap();

        let output = tempfile::TempDir::with_prefix("healthz_session").unwrap();
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
        let http_api = HttpApi::new(
            api,
            Some(HttpApiOptions {
                credential_store: Some(Arc::new(credentials)),
                ..Default::default()
            }),
        );

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
        let server = tokio::spawn(http_api.make_http_api_and_run(listener, None));

        // The probe must answer with no credentials at all...
        assert_eq!(get_status(addr, "/healthz").await, 200);
        // ...while an ordinary API route on the same server still requires them.
        assert_eq!(get_status(addr, "/torrents").await, 401);

        server.abort();
    }
}

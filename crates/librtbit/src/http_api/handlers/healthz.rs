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
    use axum::response::IntoResponse;
    use http::{StatusCode, header};

    use super::h_healthz;

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
}

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};

include!(concat!(env!("OUT_DIR"), "/webui_assets.rs"));

pub fn make_webui_router() -> Router {
    // One wildcard covers every embedded path, including nested asset
    // directories and files at the dist root. The handler never touches the
    // filesystem: an unknown path, including `..`, is a table miss.
    Router::new()
        .route("/", get(|| async { asset_response("index.html") }))
        .route("/{*path}", get(serve_path))
}

async fn serve_path(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    asset_response(&path)
}

fn asset_response(path: &str) -> Response {
    let Some(asset) = WEBUI_ASSETS.iter().find(|asset| asset.path == path) else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.content_type),
    );
    // The entry files keep fixed names across builds, so a cached copy can
    // point at chunks the next deploy no longer ships. Hashed chunks are
    // immutable for a year.
    let cache_control = if is_unhashed_entry(path) {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(asset.bytes))
        .unwrap();
    *response.headers_mut() = headers;
    response
}

fn is_unhashed_entry(path: &str) -> bool {
    matches!(
        path,
        "index.html" | "assets/index.js" | "assets/index.css" | "assets/logo.svg"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::Request};
    use tower::ServiceExt;

    async fn get(path: &str) -> Response {
        make_webui_router()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn dynamic_imports(index_js: &str) -> Vec<String> {
        // Vite emits import("./Chunk.js") or import(`./Chunk.js`).
        let mut found = Vec::new();
        for marker in ["import(\"", "import(`"] {
            let mut rest = index_js;
            while let Some(start) = rest.find(marker) {
                rest = &rest[start + marker.len()..];
                let quote = if marker.ends_with('`') { '`' } else { '"' };
                let end = rest.find(quote).expect("unterminated import");
                let spec = &rest[..end];
                if let Some(name) = spec.rsplit(['/', '\\']).next() {
                    found.push(name.to_owned());
                }
                rest = &rest[end..];
            }
        }
        found
    }

    #[tokio::test]
    async fn lazy_chunks_referenced_by_the_entry_are_served_as_javascript() {
        let index = get("/assets/index.js").await;
        assert_eq!(index.status(), StatusCode::OK);
        assert_eq!(
            index.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/javascript"
        );
        let bytes = to_bytes(index.into_body(), usize::MAX).await.unwrap();
        let source = String::from_utf8(bytes.to_vec()).unwrap();
        let imports = dynamic_imports(&source);
        for page in ["RSSPage", "IndexarrBrowse", "StatisticsPage"] {
            assert!(
                imports.iter().any(|name| name.starts_with(page)),
                "index.js does not dynamically import {page}; got {imports:?}"
            );
        }
        assert!(!imports.is_empty());
        for name in &imports {
            let response = get(&format!("/assets/{name}")).await;
            assert_eq!(response.status(), StatusCode::OK, "{name}");
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "text/javascript",
                "{name}"
            );
            assert!(
                !to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn every_embedded_file_is_served_by_exact_path_and_traversal_is_not() {
        assert!(
            WEBUI_ASSETS.iter().any(|asset| asset.path.contains('/')),
            "the manifest has no nested or root-relative path to exercise"
        );
        for asset in WEBUI_ASSETS {
            let response = get(&format!("/{}", asset.path)).await;
            assert_eq!(response.status(), StatusCode::OK, "{}", asset.path);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                asset.content_type,
                "{}",
                asset.path
            );
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(body.as_ref(), asset.bytes, "{}", asset.path);
        }
        for path in [
            "/../Cargo.toml",
            "/assets/../index.html",
            "/assets/index.js/..",
            "/no/such/nested/file.js",
        ] {
            let response = get(path).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn entry_files_are_not_cached_and_unknown_assets_are_missing() {
        for path in ["/", "/assets/index.js", "/assets/index.css"] {
            let response = get(path).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-cache",
                "{path}"
            );
        }
        let missing = get("/assets/does-not-exist.js").await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }
}

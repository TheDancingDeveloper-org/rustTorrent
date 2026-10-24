use axum::{extract::State, response::IntoResponse};
#[cfg(feature = "tracing-subscriber-utils")]
use futures::TryStreamExt;
#[cfg(feature = "tracing-subscriber-utils")]
use tracing::debug;

use super::ApiState;
use crate::api::Result;

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/rust_log",
    request_body(content = String, description = "New RUST_LOG value"),
    responses(
        (status = 200, description = "RUST_LOG updated")
    )
))]
pub async fn h_set_rust_log(
    State(state): State<ApiState>,
    new_value: String,
) -> Result<impl IntoResponse> {
    state.api.api_set_rust_log(new_value).map(axum::Json)
}

#[cfg(feature = "tracing-subscriber-utils")]
#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/stream_logs",
    responses(
        (status = 200, description = "Recent and continuous newline-delimited JSON log stream")
    )
))]
pub async fn h_stream_logs(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let s = state.api.api_log_lines_stream()?.map_err(|e| {
        debug!(error=%e, "stream_logs");
        e
    });
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/x-ndjson"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        axum::body::Body::from_stream(s),
    ))
}

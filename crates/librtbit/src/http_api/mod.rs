use anyhow::Context;
use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
#[cfg(any(feature = "webui", feature = "prometheus"))]
use axum::routing::get;
use base64::Engine;
use futures::FutureExt;
use futures::future::BoxFuture;
use http::{HeaderMap, StatusCode};
use librqbit_dualstack_sockets::TcpListener;
use std::sync::Arc;
use tower_http::trace::{DefaultOnFailure, DefaultOnResponse, OnFailure};
use tracing::{Span, debug, debug_span, info, warn};

use axum::Router;

use crate::api::Api;
use crate::rss::db::RssDatabase;

use crate::ApiError;

pub mod auth;
#[cfg(test)]
mod auth_middleware_tests;
mod handlers;
mod timeout;
#[cfg(feature = "webui")]
mod webui;

#[cfg(feature = "swagger")]
#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        handlers::torrents::h_torrents_list,
        handlers::torrents::h_torrents_post,
        handlers::torrents::h_torrent_details,
        handlers::torrents::h_torrent_haves,
        handlers::torrents::h_torrent_stats_v0,
        handlers::torrents::h_torrent_stats_v1,
        handlers::torrents::h_peer_stats,
        handlers::torrents::h_torrent_action_pause,
        handlers::torrents::h_torrent_action_start,
        handlers::torrents::h_torrent_action_recheck,
        handlers::torrents::h_torrent_action_add_trackers,
        handlers::torrents::h_torrent_action_remove_trackers,
        handlers::torrents::h_torrent_action_edit_tracker,
        handlers::torrents::h_torrent_action_reannounce,
        handlers::torrents::h_torrent_action_forget,
        handlers::torrents::h_torrent_action_delete,
        handlers::torrents::h_torrent_action_update_only_files,
        handlers::torrents::h_session_stats,
        handlers::torrents::h_peer_stats_prometheus,
        handlers::torrents::h_metadata,
        handlers::torrents::h_add_peers,
        handlers::torrents::h_create_torrent,
        handlers::configure::h_update_session_ratelimits,
        handlers::configure::h_get_session_ratelimits,
        handlers::dht::h_dht_stats,
        handlers::dht::h_dht_table,
        handlers::logging::h_set_rust_log,
        handlers::logging::h_stream_logs,
        handlers::streaming::h_torrent_stream_file,
        handlers::playlist::h_torrent_playlist,
        handlers::playlist::h_global_playlist,
        handlers::other::h_resolve_magnet,
    ),
    components(schemas(
        crate::api::TorrentListResponse,
        crate::api::TorrentDetailsResponse,
        crate::api::TorrentDetailsResponseFile,
        crate::api::EmptyJsonResponse,
        crate::api::ApiAddTorrentResponse,
        crate::api::RemoveTrackersResponse,
        crate::api::ReannounceResponse,
        crate::limits::LimitsConfig,
        handlers::torrents::UpdateOnlyFilesRequest,
        handlers::torrents::AddTrackersRequest,
        handlers::torrents::RemoveTrackersRequest,
        handlers::torrents::EditTrackerRequest,
    ))
)]
struct ApiDoc;

/// An HTTP server for the API.
pub struct HttpApi {
    api: Api,
    opts: HttpApiOptions,
    login_limiter: auth::LoginLimiter,
}

#[derive(Default)]
pub struct HttpApiOptions {
    pub read_only: bool,
    pub basic_auth: Option<(String, String)>,
    // Allow creating torrents via API.
    pub allow_create: bool,
    pub token_store: Option<Arc<auth::TokenStore>>,
    pub credential_store: Option<Arc<auth::CredentialStore>>,
    /// Keys for `Authorization: Bearer rtb_...`. None disables API keys.
    pub api_keys: Option<Arc<auth::ApiKeyStore>>,
    /// Actual port the HTTP API is bound to, reported by compatibility APIs.
    pub web_ui_port: Option<u16>,
    #[cfg(feature = "prometheus")]
    pub prometheus_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
    /// Indexarr integration: base URL (e.g. "http://indexarr:8080"). None = disabled.
    pub indexarr_url: Option<String>,
    /// Indexarr API key for authenticated requests.
    pub indexarr_api_key: Option<String>,
    /// RSS database for feed management. None = RSS disabled.
    pub rss_db: Option<Arc<parking_lot::Mutex<RssDatabase>>>,
    /// RSS feed history limit (max items to keep). None = unlimited, default 500.
    pub rss_history_limit: Option<usize>,
}

/// Constant-time byte comparison to prevent timing attacks on auth credentials.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Runs a request authenticated as `role`. A read-only caller gets 403 for anything that is not
/// a read. The role is recorded in the request for handlers that check it.
async fn admit(role: auth::Role, mut request: axum::extract::Request, next: Next) -> Response {
    if !role.allows_method(request.method()) {
        return (StatusCode::FORBIDDEN, "this account is read-only").into_response();
    }
    request.extensions_mut().insert(auth::Caller(role));
    next.run(request).await
}

/// The extra CORS origin pattern. RTBIT_CORS_ALLOW_REGEXP is the canonical name; the bare
/// CORS_ALLOW_REGEXP is still read so existing deployments keep working. An invalid pattern is
/// ignored with a warning rather than failing startup.
fn cors_allow_regex(rtbit: Option<String>, legacy: Option<String>) -> Option<regex::bytes::Regex> {
    let value = rtbit.or(legacy)?;
    match regex::bytes::Regex::new(&value) {
        Ok(re) => Some(re),
        Err(err) => {
            warn!("ignoring invalid CORS allow regexp: {err}");
            None
        }
    }
}

/// Auth endpoints that need no session. Logout is one: it only revokes the refresh token it is
/// sent, which the caller can only hold if it was issued to them, and it must work for a
/// read-only caller (whose POSTs are otherwise refused) and after the access token has expired.
fn is_public_auth_path(path: &str) -> bool {
    matches!(
        path,
        "/auth/login"
            | "/auth/refresh"
            | "/auth/logout"
            | "/auth/status"
            | "/auth/setup"
            | "/healthz"
    )
}

/// Paths reachable before any credentials exist: the setup status and setup endpoints,
/// and the static web UI that hosts the setup page. Everything else waits for setup.
fn is_pre_setup_path(path: &str) -> bool {
    path == "/auth/status" || path == "/auth/setup" || is_web_ui_path(path)
}

/// Static web UI. Served without a session so the browser can load the sign-in page
/// instead of a Basic-auth dialog. API calls from the page still authenticate.
fn is_web_ui_path(path: &str) -> bool {
    path == "/" || path == "/web" || path.starts_with("/web/")
}

impl HttpApi {
    pub fn new(api: Api, opts: Option<HttpApiOptions>) -> Self {
        Self {
            api,
            opts: opts.unwrap_or_default(),
            login_limiter: auth::LoginLimiter::default(),
        }
    }

    /// Run the HTTP server forever on the given address.
    /// If read_only is passed, no state-modifying methods will be exposed.
    #[inline(never)]
    pub fn make_http_api_and_run(
        #[allow(unused_mut)] mut self,
        listener: TcpListener,
        upnp_router: Option<Router>,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        #[cfg(feature = "prometheus")]
        let mut prometheus_handle = self.opts.prometheus_handle.take();

        let state = Arc::new(self);

        let mut main_router = handlers::make_api_router(state.clone());

        #[cfg(feature = "webui")]
        {
            use axum::response::Redirect;

            let webui_router = webui::make_webui_router();
            main_router = main_router.nest("/web/", webui_router);
            main_router = main_router.route("/web", get(|| async { Redirect::permanent("./web/") }))
        }

        #[cfg(feature = "swagger")]
        {
            use utoipa::OpenApi;
            use utoipa_swagger_ui::SwaggerUi;
            main_router = main_router
                .merge(SwaggerUi::new("/swagger").url("/api-docs/openapi.json", ApiDoc::openapi()));
        }

        #[cfg(feature = "prometheus")]
        if let Some(handle) = prometheus_handle.take() {
            let session = state.api.session().clone();
            main_router = main_router.route(
                "/metrics",
                get(move || async move {
                    let mut metrics = handle.render();
                    session.stats_snapshot().as_prometheus(&mut metrics);
                    metrics
                }),
            );
        }

        let cors_layer = {
            use tower_http::cors::{AllowHeaders, AllowOrigin};

            const ALLOWED_ORIGINS: [&[u8]; 4] = [
                // Webui-dev
                b"http://localhost:3031",
                b"http://127.0.0.1:3031",
                // Tauri dev
                b"http://localhost:1420",
                // Tauri prod
                b"tauri://localhost",
            ];

            let allow_regex = cors_allow_regex(
                std::env::var("RTBIT_CORS_ALLOW_REGEXP").ok(),
                std::env::var("CORS_ALLOW_REGEXP").ok(),
            );

            tower_http::cors::CorsLayer::default()
                .allow_origin(AllowOrigin::predicate(move |v, _| {
                    ALLOWED_ORIGINS.contains(&v.as_bytes())
                        || allow_regex
                            .as_ref()
                            .map(move |r| r.is_match(v.as_bytes()))
                            .unwrap_or(false)
                }))
                .allow_headers(AllowHeaders::any())
        };

        // Unified auth: supports Bearer token, Basic auth, and credential store.
        // The middleware checks credentials dynamically so runtime changes are picked up.
        {
            let token_store = state.opts.token_store.clone();
            let api_keys = state.opts.api_keys.clone();
            let basic_auth = state.opts.basic_auth.clone();
            let credential_store = state.opts.credential_store.clone();

            let has_initial_creds = basic_auth.is_some()
                || credential_store
                    .as_ref()
                    .is_some_and(|cs| cs.has_credentials());

            if has_initial_creds {
                info!("Enabling authentication in HTTP API");
            } else if let Some(cs) = &credential_store {
                info!("Authentication not yet configured; the API is closed until setup is done");
                if let Some(token) = cs.setup_token() {
                    // Written to stderr, not through tracing: the token is a secret, and log
                    // filters, JSON output and log shippers would otherwise keep a copy of it.
                    eprintln!("rtbit first-time setup token: {token}");
                    eprintln!(
                        "Send it as \"setup_token\" to POST /auth/setup, or enter it on the setup page."
                    );
                    eprintln!(
                        "It is also written to {} (owner-only) until setup completes.",
                        cs.setup_token_path().display()
                    );
                }
            }

            let limiter_state = state.clone();
            main_router = main_router.route_layer(axum::middleware::from_fn(
                move |headers: HeaderMap, request: axum::extract::Request, next: Next| {
                    let token_store = token_store.clone();
                    let api_keys = api_keys.clone();
                    let basic_auth = basic_auth.clone();
                    let credential_store = credential_store.clone();
                    let limiter_state = limiter_state.clone();
                    async move {
                        // Always skip auth for these public endpoints
                        let path = request.uri().path();
                        if is_public_auth_path(path) || is_web_ui_path(path) {
                            return Ok(next.run(request).await);
                        }

                        // Determine if any credentials are configured
                        let has_stored_creds = credential_store
                            .as_ref()
                            .is_some_and(|cs| cs.has_credentials());
                        let has_env_creds = basic_auth.is_some();

                        if !has_stored_creds && !has_env_creds {
                            // Without a credential store, auth cannot be configured at all.
                            if credential_store.is_none() {
                                return Ok(next.run(request).await);
                            }
                            // Setup is pending: nothing but the setup page may be reached.
                            if is_pre_setup_path(path) {
                                return Ok(next.run(request).await);
                            }
                            return Ok((StatusCode::FORBIDDEN, "setup required").into_response());
                        }

                        // Try Bearer first: an API key, or a session's access token.
                        let bearer = headers
                            .get("Authorization")
                            .and_then(|h| h.to_str().ok())
                            .and_then(|h| h.strip_prefix("Bearer "));
                        if let Some(token) =
                            bearer.filter(|_| token_store.is_some() || api_keys.is_some())
                        {
                            let current = |role: auth::Role| {
                                auth::fingerprint_for(
                                    credential_store.as_deref(),
                                    basic_auth.as_ref(),
                                    role,
                                )
                            };
                            let role = if token.starts_with(auth::API_KEY_PREFIX) {
                                api_keys.as_ref().and_then(|keys| keys.authenticate(token))
                            } else {
                                token_store
                                    .as_ref()
                                    .and_then(|ts| ts.validate_access_token(token, &current))
                            };
                            return match role {
                                Some(role) => Ok(admit(role, request, next).await),
                                None => Err(ApiError::unauthorized()),
                            };
                        }

                        // Try Basic auth — check credential store first, then env var
                        let user_pass = headers
                            .get("Authorization")
                            .and_then(|h| h.to_str().ok())
                            .and_then(|h| h.strip_prefix("Basic "))
                            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
                            .and_then(|v| String::from_utf8(v).ok());

                        let user_pass = match user_pass {
                            Some(up) => up,
                            None => {
                                // Browser navigation to the UI gets the sign-in page, not a
                                // Basic challenge. API clients still receive the challenge.
                                if headers
                                    .get("Accept")
                                    .and_then(|h| h.to_str().ok())
                                    .is_some_and(|h| h.contains("text/html"))
                                {
                                    return Ok((
                                        StatusCode::TEMPORARY_REDIRECT,
                                        [("Location", "/web/")],
                                        "sign in",
                                    )
                                        .into_response());
                                }
                                return Ok((
                                    StatusCode::UNAUTHORIZED,
                                    [("WWW-Authenticate", "Basic realm=\"API\"")],
                                )
                                    .into_response());
                            }
                        };

                        // Failed Basic attempts count as login attempts. A locked-out client is
                        // refused before any password hash is computed.
                        let client = auth::client_addr_from_extensions(request.extensions());
                        let now = std::time::Instant::now();
                        if let Some(wait) = limiter_state.login_limiter.check(client, now) {
                            return Ok(auth::too_many_login_attempts(wait));
                        }

                        if let Some((u, p)) = user_pass.split_once(':') {
                            // Check credential store
                            if has_stored_creds && let Some(cs) = &credential_store {
                                let verdict = auth::verify_credentials(
                                    &limiter_state.login_limiter,
                                    cs.clone(),
                                    u.to_owned(),
                                    p.to_owned(),
                                )
                                .await;
                                match verdict {
                                    auth::Verdict::Valid(role) => {
                                        limiter_state.login_limiter.record_success(client);
                                        return Ok(admit(role, request, next).await);
                                    }
                                    auth::Verdict::Busy => return Ok(auth::busy_response()),
                                    auth::Verdict::Invalid => {}
                                }
                            }

                            // Check env var credentials
                            if let Some((env_u, env_p)) = &basic_auth
                                && constant_time_eq(u.as_bytes(), env_u.as_bytes())
                                && constant_time_eq(p.as_bytes(), env_p.as_bytes())
                            {
                                limiter_state.login_limiter.record_success(client);
                                return Ok(admit(auth::Role::Admin, request, next).await);
                            }
                        }

                        limiter_state.login_limiter.record_failure(client, now);
                        Err(ApiError::unauthorized())
                    }
                },
            ));
        }

        // qBittorrent WebUI API v2 compatibility layer.
        // Mounted after auth so it handles its own cookie-based auth.
        {
            let qbit_router = handlers::qbit::make_qbit_router(state.clone());
            main_router = main_router.nest("/api/v2", qbit_router);
        }

        if let Some(upnp_router) = upnp_router {
            main_router = main_router.nest("/upnp", upnp_router);
        }

        let app = main_router
            .layer(axum::extract::DefaultBodyLimit::max(100 * 1024 * 1024)) // 100 MB max request body
            .layer(cors_layer)
            .layer(
                tower_http::trace::TraceLayer::new_for_http()
                    .make_span_with(|req: &Request| {
                        let method = req.method();
                        let uri = req.uri();
                        if let Some(ConnectInfo(addr)) = req
                            .extensions()
                            .get::<ConnectInfo<librqbit_dualstack_sockets::WrappedSocketAddr>>()
                        {
                            debug_span!("request", %method, %uri, addr=%addr.0)
                        } else {
                            debug_span!("request", %method, %uri)
                        }
                    })
                    .on_request(|req: &Request, _: &Span| {
                        if req.uri().path().starts_with("/upnp") {
                            debug!(headers=?req.headers())
                        }
                    })
                    .on_response(DefaultOnResponse::new().include_headers(true))
                    .on_failure({
                        let mut default = DefaultOnFailure::new();
                        move |failure_class, latency, span: &Span| match failure_class {
                            tower_http::classify::ServerErrorsFailureClass::StatusCode(
                                StatusCode::NOT_IMPLEMENTED,
                            ) => {}
                            _ => default.on_failure(failure_class, latency, span),
                        }
                    }),
            )
            .into_make_service_with_connect_info::<librqbit_dualstack_sockets::WrappedSocketAddr>();

        async move {
            axum::serve(listener, app)
                .await
                .context("error running HTTP API")
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::{cors_allow_regex, is_pre_setup_path, is_public_auth_path};

    #[test]
    fn cors_regex_prefers_the_rtbit_name_and_ignores_invalid_patterns() {
        let re = |rtbit: Option<&str>, legacy: Option<&str>| {
            cors_allow_regex(rtbit.map(str::to_owned), legacy.map(str::to_owned))
        };
        assert!(re(None, None).is_none());

        let legacy = re(None, Some("^https://legacy\\.test$")).unwrap();
        assert!(legacy.is_match(b"https://legacy.test"));

        let both = re(
            Some("^https://new\\.test$"),
            Some("^https://legacy\\.test$"),
        )
        .unwrap();
        assert!(both.is_match(b"https://new.test"));
        assert!(!both.is_match(b"https://legacy.test"));

        // An invalid canonical value disables the extra pattern; it doesn't fall back.
        assert!(re(Some("(unclosed"), Some("^https://legacy\\.test$")).is_none());
    }

    #[test]
    fn logout_needs_no_session_but_other_auth_writes_do() {
        for path in [
            "/auth/login",
            "/auth/refresh",
            "/auth/logout",
            "/auth/status",
            "/auth/setup",
            "/healthz",
        ] {
            assert!(is_public_auth_path(path), "{path}");
        }
        assert!(!is_public_auth_path("/auth/change_credentials"));
        assert!(!is_public_auth_path("/auth/api_keys"));
        assert!(!is_public_auth_path("/auth/logout/extra"));
        assert!(!is_public_auth_path("/torrents"));
    }

    #[test]
    fn only_setup_status_and_web_ui_are_reachable_before_setup() {
        assert!(is_pre_setup_path("/auth/status"));
        assert!(is_pre_setup_path("/auth/setup"));
        assert!(is_pre_setup_path("/web"));
        assert!(is_pre_setup_path("/web/"));
        assert!(is_pre_setup_path("/web/assets/index.js"));

        // `/` is the browser entry point; it redirects to the sign-in page.
        assert!(is_pre_setup_path("/"));
        assert!(!is_pre_setup_path("/torrents"));
        assert!(!is_pre_setup_path("/fs/browse"));
        assert!(!is_pre_setup_path("/auth/change_credentials"));
        assert!(!is_pre_setup_path("/webhooks/secret"));
    }
}

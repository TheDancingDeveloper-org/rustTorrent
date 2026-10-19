//! Auth controller (`auth/*`), the SID-cookie helpers and the cross-origin guard.

use super::*;

// ---------------------------------------------------------------------------
// Auth endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub(super) struct LoginForm {
    #[serde(default)]
    pub(super) username: String,
    #[serde(default)]
    pub(super) password: String,
}

pub(super) async fn h_auth_login(
    State(state): State<Arc<QbitState>>,
    crate::http_api::auth::ClientAddr(client): crate::http_api::auth::ClientAddr,
    body: Bytes,
) -> impl IntoResponse {
    use crate::http_api::auth::{Role, Verdict, busy_response, verify_credentials};

    let limiter = &state.api_state.login_limiter;
    if let Some(wait) = limiter.check(client, Instant::now()) {
        let retry_after = wait.as_secs().max(1).to_string();
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(RETRY_AFTER, retry_after)],
            "Fails.".to_string(),
        )
            .into_response();
    }

    let form: LoginForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();

    let stored_credentials = state
        .api_state
        .opts
        .credential_store
        .as_ref()
        .filter(|store| store.has_credentials());
    let verdict = match (stored_credentials, &state.api_state.opts.basic_auth) {
        (Some(store), _) => {
            match verify_credentials(limiter, Arc::clone(store), form.username, form.password).await
            {
                // The compat session carries no role, so a read-only account is refused here
                // rather than given write access through it.
                Verdict::Valid(Role::ReadOnly) => Verdict::Invalid,
                other => other,
            }
        }
        (None, Some((expected_user, expected_pass))) => {
            if crate::http_api::constant_time_eq(form.username.as_bytes(), expected_user.as_bytes())
                && crate::http_api::constant_time_eq(
                    form.password.as_bytes(),
                    expected_pass.as_bytes(),
                )
            {
                Verdict::Valid(Role::Admin)
            } else {
                Verdict::Invalid
            }
        }
        // No credentials at all: only acceptable when there is no credential store either.
        // With a store, setup has not happened yet and login must fail.
        (None, None) => {
            if state.api_state.opts.credential_store.is_none() {
                Verdict::Valid(Role::Admin)
            } else {
                Verdict::Invalid
            }
        }
    };

    match verdict {
        Verdict::Busy => busy_response(),
        Verdict::Valid(_) => {
            limiter.record_success(client);
            let sid = state.sessions.create_session();
            let cookie = format!("SID={sid}; Path=/; HttpOnly; SameSite=Strict");
            (StatusCode::OK, [(SET_COOKIE, cookie)], "Ok.".to_string()).into_response()
        }
        Verdict::Invalid => {
            limiter.record_failure(client, Instant::now());
            (StatusCode::FORBIDDEN, "Fails.".to_string()).into_response()
        }
    }
}

pub(super) async fn h_auth_logout(
    State(state): State<Arc<QbitState>>,
    headers: HeaderMap,
) -> &'static str {
    if let Some(sid) = extract_sid(&headers) {
        state.sessions.remove_session(&sid);
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// Auth middleware helper
// ---------------------------------------------------------------------------

pub(super) fn extract_sid(headers: &HeaderMap) -> Option<String> {
    headers
        .get(http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix("SID=").map(|s| s.to_string())
            })
        })
}

/// Plain-text error response used by every compat handler, so clients always
/// get the same shape: the status code and a short sentence in the body.
pub(super) fn qbit_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, message.into()).into_response()
}

/// Rejects cross-site POSTs. Browsers always send `Origin` on cross-origin
/// POSTs, and it must name the same authority as `Host`. Requests without an
/// `Origin` (native *arr clients, curl) are not browser requests and pass.
pub(super) fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(http::header::ORIGIN) else {
        return true;
    };
    let Some(origin) = origin.to_str().ok() else {
        return false;
    };
    let Some(host) = headers
        .get(http::header::HOST)
        .and_then(|host| host.to_str().ok())
    else {
        return false;
    };
    // `null` and anything without a scheme separator fail the split.
    origin
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .is_some_and(|authority| authority.eq_ignore_ascii_case(host))
}

pub(super) async fn reject_cross_origin_posts(request: Request, next: Next) -> Response {
    if request.method() == Method::POST && !origin_allowed(request.headers()) {
        return qbit_error(StatusCode::FORBIDDEN, "cross-origin request rejected");
    }
    next.run(request).await
}

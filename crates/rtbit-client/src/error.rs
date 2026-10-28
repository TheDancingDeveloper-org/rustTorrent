use http::StatusCode;

/// A failure talking to the daemon. The client does not panic on these.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The daemon could not be reached, or the response was not usable.
    #[error("daemon unreachable: {0}")]
    Unreachable(String),
    /// Login or refresh was refused.
    #[error("unauthorised")]
    Unauthorised,
    /// No credentials exist on the daemon yet, or setup is still required.
    #[error("daemon is not set up")]
    NotSetUp,
    /// The daemon answered with an error status.
    /// `message` is set only for a 400 whose body is a short, plain validation
    /// sentence. It never carries a token or a raw response body.
    #[error("daemon error {status}")]
    Daemon {
        status: StatusCode,
        message: Option<String>,
    },
}

impl Error {
    pub(crate) fn from_status(status: StatusCode) -> Self {
        Self::from_status_body(status, "")
    }

    /// Like [`from_status`], but a 400 with a plain validation sentence keeps that sentence.
    pub(crate) fn from_status_body(status: StatusCode, body: &str) -> Self {
        if status == StatusCode::UNAUTHORIZED {
            Self::Unauthorised
        } else if status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND {
            Self::NotSetUp
        } else {
            let message = if status == StatusCode::BAD_REQUEST {
                safe_validation_message(body)
            } else {
                None
            };
            Self::Daemon { status, message }
        }
    }
}

/// A daemon validation sentence is safe to show when it is short, single-line,
/// and has no token-shaped or structured content. Anything else is dropped.
fn safe_validation_message(body: &str) -> Option<String> {
    let text = validation_text(body)?;
    let text = text.trim();
    if text.is_empty() || text.len() > 200 || text.contains(['\n', '\r', '{', '}', '<', '>']) {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("token") || lower.contains("bearer") || lower.contains("password") {
        return None;
    }
    if !text.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
        return None;
    }
    Some(text.to_owned())
}

/// Plain text, or the `human_readable` string of the daemon's JSON error.
fn validation_text(body: &str) -> Option<&str> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('{') {
        let key = "\"human_readable\"";
        let at = rest.find(key)?;
        let after = rest[at + key.len()..].trim_start();
        let after = after.strip_prefix(':')?.trim_start();
        let quoted = after.strip_prefix('"')?;
        let end = quoted.find('"')?;
        return Some(&quoted[..end]);
    }
    Some(trimmed)
}

#[cfg(test)]
mod validation_message {
    use super::Error;
    use http::StatusCode;

    #[test]
    fn a_400_json_body_keeps_only_the_plain_sentence() {
        let body = r#"{"error_kind":"internal_error","human_readable":"timeout exceeds its cap","status":400}"#;
        let err = Error::from_status_body(StatusCode::BAD_REQUEST, body);
        match err {
            Error::Daemon {
                message: Some(text),
                ..
            } => assert_eq!(text, "timeout exceeds its cap"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_400_drops_a_body_that_is_not_a_plain_sentence() {
        for body in [
            r#"{"human_readable":"bearer eyJhbGciOi-secret"}"#,
            "timeout\nsecond line",
            "<html>no</html>",
            "password is wrong",
        ] {
            let err = Error::from_status_body(StatusCode::BAD_REQUEST, body);
            match err {
                Error::Daemon { message: None, .. } => {}
                other => panic!("kept unsafe body: {other:?}"),
            }
        }
    }

    #[test]
    fn other_statuses_ignore_the_body() {
        let err =
            Error::from_status_body(StatusCode::INTERNAL_SERVER_ERROR, "timeout exceeds its cap");
        match err {
            Error::Daemon { message: None, .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

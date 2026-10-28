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
    #[error("daemon error {status}")]
    Daemon { status: StatusCode },
}

impl Error {
    pub(crate) fn from_status(status: StatusCode) -> Self {
        if status == StatusCode::UNAUTHORIZED {
            Self::Unauthorised
        } else if status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND {
            Self::NotSetUp
        } else {
            Self::Daemon { status }
        }
    }
}

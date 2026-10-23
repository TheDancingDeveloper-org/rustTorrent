use std::{borrow::Cow, fmt::Display};

use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, error, trace};

/// Error for work abandoned because shutdown was requested. It is expected,
/// so it is logged without ERROR severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cancelled")]
pub struct Cancelled;

/// True if `e` is, or wraps, a [`Cancelled`].
pub fn is_cancelled<E: std::any::Any>(e: &E) -> bool {
    let any = e as &dyn std::any::Any;
    any.downcast_ref::<Cancelled>().is_some()
        || any
            .downcast_ref::<anyhow::Error>()
            .is_some_and(|e| e.downcast_ref::<Cancelled>().is_some())
}

/// Spawns a future with tracing instrumentation.
#[track_caller]
pub fn spawn<E: Display + Send + 'static>(
    span: tracing::Span,
    name: impl Into<Cow<'static, str>>,
    fut: impl std::future::Future<Output = Result<(), E>> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let name = name.into();
    let fut = async move {
        trace!("started");
        tokio::pin!(fut);
        let mut trace_interval = tokio::time::interval(std::time::Duration::from_secs(5));

        loop {
            tokio::select! {
                _ = trace_interval.tick() => {
                    trace!("still running");
                },
                r = &mut fut => {
                    match r {
                        Ok(_) => {
                            trace!("finished");
                        }
                        Err(e) if is_cancelled(&e) => {
                            debug!("{name} cancelled")
                        }
                        Err(e) => {
                            error!("{name} finished with error: {:#}", e)
                        }
                    }
                    return;
                }
            }
        }
    }
    .instrument(span);
    tokio::task::spawn(fut)
}

#[track_caller]
pub fn spawn_with_cancel<E: Display + Send + 'static>(
    span: tracing::Span,
    name: impl Into<Cow<'static, str>>,
    cancellation_token: CancellationToken,
    fut: impl std::future::Future<Output = Result<(), E>> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    spawn(span, name, async move {
        tokio::select! {
            _ = cancellation_token.cancelled() => {
                debug!("task cancelled");
                Ok(())
            },
            r = fut => r
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_is_detected_directly_and_through_anyhow() {
        assert!(is_cancelled(&Cancelled));
        assert!(is_cancelled(&anyhow::Error::from(Cancelled)));
        assert!(is_cancelled(
            &anyhow::Error::from(Cancelled).context("while starting")
        ));
    }

    #[test]
    fn other_errors_are_not_cancellation() {
        assert!(!is_cancelled(&anyhow::anyhow!("cancelled")));
        assert!(!is_cancelled(&"boom".to_string()));
    }
}

//! Native client shell. Screens and chrome live here; the daemon protocol
//! lives in `rtbit-client`. This crate does not depend on the daemon library.

mod app;

pub use app::{App, Connection, LoginAttempt, Screen};

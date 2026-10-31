//! Toolkit-neutral core of the native rustTorrent client: the daemon session,
//! screen view-models, list state, formatting and the token store. No UI
//! toolkit is linked here; `rtbit-egui` draws on top of it.

mod actions;
pub mod error;
pub mod format;
pub mod keychain;
mod session;
pub mod state;

pub use actions::{AddRequest, AddSource, DetailAction, RowAction, SettingsChanges};
pub use error::map_error;
pub use keychain::{MemoryStore, OsKeychain, TokenStore};
pub use session::{Connection, DaemonLogin, DetailSnapshot, LoginAttempt, Screen, Snapshot};
pub use state::{ListCache, StatusFilter};

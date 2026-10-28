//! Product screens. Connect stays in `app` until the shell is split further.

mod main_screen;

pub use main_screen::{AddDraft, AddRequest, AddSource, RowAction, RowActions, draw_main};

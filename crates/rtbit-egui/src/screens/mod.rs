//! Product screens. Connect stays in `app` until the shell is split further.

mod detail;
mod main_screen;

pub use detail::{DetailAction, DetailTab, DetailView, draw_detail};
pub use main_screen::{AddDraft, AddRequest, AddSource, RowAction, RowActions, draw_main};

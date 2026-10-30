//! Product screens. Connect stays in `app` until the shell is split further.

mod detail;
mod main_screen;
mod rss;
mod settings;

pub use detail::{DetailAction, DetailTab, DetailView, draw_detail};
pub use main_screen::{AddDraft, AddRequest, AddSource, RowAction, RowActions, draw_main};
pub use rss::{RssAction, RssView, RuleDraft, RuleEditor, draw_rss};
pub use settings::{SettingsAction, SettingsChanges, SettingsDraft, SettingsTab, draw_settings};

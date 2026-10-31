//! The tray menu as plain data, in display order.

use crate::action::MenuAction;

/// One entry in the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEntry {
    /// A clickable item that runs `action`.
    Action {
        action: MenuAction,
        label: &'static str,
    },
    /// The live status line. It is never enabled.
    Status {
        id: &'static str,
        label: &'static str,
    },
    /// A check item that runs `action` when clicked.
    Check {
        action: MenuAction,
        label: &'static str,
    },
    Separator,
}

/// The id of the status line, which the tray rewrites every few seconds.
pub const STATUS_ID: &str = "speed";

/// The tray menu. `self_update` adds Check for Updates.
pub fn tray_entries(self_update: bool) -> Vec<TrayEntry> {
    let mut entries = vec![
        TrayEntry::Action {
            action: MenuAction::ShowWindow,
            label: "Show rtbit",
        },
        TrayEntry::Action {
            action: MenuAction::AddTorrentFile,
            label: "Add .torrent file…",
        },
        TrayEntry::Status {
            id: STATUS_ID,
            label: "Speed: idle",
        },
        TrayEntry::Separator,
        TrayEntry::Action {
            action: MenuAction::ToggleTransfers,
            label: "Pause All",
        },
        TrayEntry::Action {
            action: MenuAction::OpenInBrowser,
            label: "Open in Browser",
        },
        TrayEntry::Separator,
        TrayEntry::Check {
            action: MenuAction::ToggleAutostart,
            label: "Start at login",
        },
    ];
    if self_update {
        entries.push(TrayEntry::Action {
            action: MenuAction::CheckForUpdates,
            label: "Check for Updates…",
        });
    }
    entries.push(TrayEntry::Separator);
    entries.push(TrayEntry::Action {
        action: MenuAction::Quit,
        label: "Quit",
    });
    entries
}

/// Every clickable tray id, in menu order.
pub fn tray_ids(self_update: bool) -> Vec<&'static str> {
    tray_entries(self_update)
        .iter()
        .filter_map(|entry| match entry {
            TrayEntry::Action { action, .. } | TrayEntry::Check { action, .. } => Some(action.id()),
            TrayEntry::Status { .. } | TrayEntry::Separator => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_ids;

    #[test]
    fn tray_lists_every_action_in_order() {
        assert_eq!(
            tray_ids(true),
            [
                "show",
                "add",
                "pause",
                "browser",
                "autostart",
                "updates",
                "quit"
            ]
        );
    }

    #[test]
    fn tray_without_self_update_has_no_update_item() {
        assert!(!tray_ids(false).contains(&"updates"));
        assert_eq!(tray_ids(false).len(), tray_ids(true).len() - 1);
    }

    #[test]
    fn every_tray_id_dispatches_to_its_own_action() {
        for self_update in [true, false] {
            for entry in tray_entries(self_update) {
                if let TrayEntry::Action { action, .. } | TrayEntry::Check { action, .. } = entry {
                    assert_eq!(MenuAction::from_id(action.id(), self_update), Some(action));
                }
            }
        }
    }

    #[test]
    fn the_status_line_is_not_an_action() {
        assert_eq!(MenuAction::from_id(STATUS_ID, true), None);
        assert!(!tray_ids(true).contains(&STATUS_ID));
    }

    #[test]
    fn quit_is_last_after_a_separator() {
        let entries = tray_entries(true);
        assert_eq!(entries[entries.len() - 2], TrayEntry::Separator);
        assert!(matches!(
            entries.last(),
            Some(TrayEntry::Action {
                action: MenuAction::Quit,
                ..
            })
        ));
    }

    #[test]
    fn every_menu_bar_item_is_also_in_the_tray() {
        let tray = tray_ids(false);
        for id in item_ids() {
            assert!(tray.contains(&id), "{id}");
            assert!(MenuAction::from_id(id, false).is_some(), "{id}");
        }
    }
}

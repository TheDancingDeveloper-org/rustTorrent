//! The native menu bar as plain data. The desktop binary turns this into a
//! Tauri menu; the tests here run without the window toolkit.

/// One entry in the menu bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    /// A clickable item. `id` is what the click handler receives.
    Item {
        id: &'static str,
        label: &'static str,
    },
    /// A divider inside a menu.
    Separator,
}

/// One top-level menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuBarMenu {
    pub label: &'static str,
    pub entries: &'static [MenuEntry],
}

/// File, Torrent and View. Quit and the torrent actions reuse the tray's ids,
/// so one handler serves both menus.
pub fn menus() -> &'static [MenuBarMenu] {
    const FILE: &[MenuEntry] = &[
        MenuEntry::Item {
            id: "add",
            label: "Add .torrent file…",
        },
        MenuEntry::Item {
            id: "browser",
            label: "Open in Browser",
        },
        MenuEntry::Separator,
        MenuEntry::Item {
            id: "quit",
            label: "Quit",
        },
    ];
    const TORRENT: &[MenuEntry] = &[MenuEntry::Item {
        id: "pause",
        label: "Pause / Resume All",
    }];
    const VIEW: &[MenuEntry] = &[MenuEntry::Item {
        id: "show",
        label: "Show Window",
    }];
    const MENUS: &[MenuBarMenu] = &[
        MenuBarMenu {
            label: "File",
            entries: FILE,
        },
        MenuBarMenu {
            label: "Torrent",
            entries: TORRENT,
        },
        MenuBarMenu {
            label: "View",
            entries: VIEW,
        },
    ];
    MENUS
}

/// Every clickable id, in menu order.
pub fn item_ids() -> Vec<&'static str> {
    menus()
        .iter()
        .flat_map(|menu| menu.entries)
        .filter_map(|entry| match entry {
            MenuEntry::Item { id, .. } => Some(*id),
            MenuEntry::Separator => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_bar_has_file_torrent_and_view() {
        let labels: Vec<_> = menus().iter().map(|menu| menu.label).collect();
        assert_eq!(labels, ["File", "Torrent", "View"]);
    }

    #[test]
    fn every_item_has_an_id_the_tray_handler_already_knows() {
        let known = ["add", "browser", "quit", "pause", "show"];
        let ids = item_ids();
        assert!(!ids.is_empty());
        for id in &ids {
            assert!(known.contains(id), "{id}");
        }
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn file_ends_with_quit_after_a_separator() {
        let file = menus().iter().find(|menu| menu.label == "File").unwrap();
        assert!(matches!(
            file.entries[file.entries.len() - 2],
            MenuEntry::Separator
        ));
        assert_eq!(
            file.entries.last(),
            Some(&MenuEntry::Item {
                id: "quit",
                label: "Quit"
            })
        );
    }
}

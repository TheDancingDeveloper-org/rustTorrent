//! What a click on a tray or menu-bar item does, decided without the window
//! toolkit. The desktop binary maps each [`MenuAction`] onto Tauri calls.

/// The action behind a menu id. The tray and the menu bar share these ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Show, unminimize and focus the main window.
    ShowWindow,
    /// Open the `.torrent` file picker.
    AddTorrentFile,
    /// Pause every running torrent, or resume every paused one.
    ToggleTransfers,
    /// Open the web UI in the default browser.
    OpenInBrowser,
    /// Flip start-at-login.
    ToggleAutostart,
    /// Run the manual update check.
    CheckForUpdates,
    /// Quit, asking first when transfers are active.
    Quit,
}

impl MenuAction {
    pub const ALL: [MenuAction; 7] = [
        MenuAction::ShowWindow,
        MenuAction::AddTorrentFile,
        MenuAction::ToggleTransfers,
        MenuAction::OpenInBrowser,
        MenuAction::ToggleAutostart,
        MenuAction::CheckForUpdates,
        MenuAction::Quit,
    ];

    /// The menu id this action is registered under.
    pub fn id(self) -> &'static str {
        match self {
            MenuAction::ShowWindow => "show",
            MenuAction::AddTorrentFile => "add",
            MenuAction::ToggleTransfers => "pause",
            MenuAction::OpenInBrowser => "browser",
            MenuAction::ToggleAutostart => "autostart",
            MenuAction::CheckForUpdates => "updates",
            MenuAction::Quit => "quit",
        }
    }

    /// The action for a clicked id. Unknown ids, and `updates` in a build
    /// without the self-updater, do nothing.
    pub fn from_id(id: &str, self_update: bool) -> Option<MenuAction> {
        let action = Self::ALL.into_iter().find(|action| action.id() == id)?;
        if action == MenuAction::CheckForUpdates && !self_update {
            return None;
        }
        Some(action)
    }
}

/// What Quit does once it knows how many torrents are downloading or seeding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuitDecision {
    /// Nothing is transferring: exit now.
    ExitNow,
    /// Ask first, with this message.
    Confirm { message: String },
}

pub fn quit_decision(active_torrents: usize) -> QuitDecision {
    if active_torrents == 0 {
        QuitDecision::ExitNow
    } else {
        QuitDecision::Confirm {
            message: format!(
                "{active_torrents} torrent(s) are still downloading or seeding. Quit anyway?"
            ),
        }
    }
}

/// The web UI address that Open in Browser hands to the OS.
pub fn browser_url(listen_addr: &str) -> String {
    format!("http://{listen_addr}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_round_trips_through_its_id() {
        for action in MenuAction::ALL {
            assert_eq!(MenuAction::from_id(action.id(), true), Some(action));
        }
    }

    #[test]
    fn ids_dispatch_to_the_expected_actions() {
        let cases = [
            ("show", MenuAction::ShowWindow),
            ("add", MenuAction::AddTorrentFile),
            ("pause", MenuAction::ToggleTransfers),
            ("browser", MenuAction::OpenInBrowser),
            ("autostart", MenuAction::ToggleAutostart),
            ("updates", MenuAction::CheckForUpdates),
            ("quit", MenuAction::Quit),
        ];
        for (id, action) in cases {
            assert_eq!(MenuAction::from_id(id, true), Some(action), "{id}");
        }
    }

    #[test]
    fn unknown_ids_do_nothing() {
        assert_eq!(MenuAction::from_id("", true), None);
        assert_eq!(MenuAction::from_id("speed", true), None);
        assert_eq!(MenuAction::from_id("Quit", true), None);
    }

    #[test]
    fn update_check_is_inert_without_the_self_updater() {
        assert_eq!(MenuAction::from_id("updates", false), None);
        assert_eq!(MenuAction::from_id("quit", false), Some(MenuAction::Quit));
    }

    #[test]
    fn quit_exits_at_once_when_idle_and_asks_otherwise() {
        assert_eq!(quit_decision(0), QuitDecision::ExitNow);
        assert_eq!(
            quit_decision(3),
            QuitDecision::Confirm {
                message: "3 torrent(s) are still downloading or seeding. Quit anyway?".into()
            }
        );
    }

    #[test]
    fn open_in_browser_targets_the_api_address() {
        assert_eq!(browser_url("127.0.0.1:3030"), "http://127.0.0.1:3030");
        assert_eq!(browser_url("[::1]:3030"), "http://[::1]:3030");
    }
}

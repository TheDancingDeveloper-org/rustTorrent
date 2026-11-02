//! Bottom status bar: connection badge, torrent total and live speeds.

use fluent_primitives::{ConnectionBadge, ConnectionBadgeState, Label, LabelSize};
use gpui::{Context, IntoElement, Render, Window, div, prelude::*};
use rtbit_ui_core::{Connection, Snapshot, format};

/// Status bar state. The shell pushes values in; nothing here talks to the daemon.
pub struct StatusBar {
    connection: Connection,
    torrents: u64,
    download: String,
    upload: String,
}

impl StatusBar {
    #[must_use]
    pub fn new() -> Self {
        Self {
            connection: Connection::Disconnected,
            torrents: 0,
            download: format::speed(&Default::default()),
            upload: format::speed(&Default::default()),
        }
    }

    /// The connection state the badge shows.
    #[must_use]
    pub fn connection(&self) -> Connection {
        self.connection
    }

    pub fn set_connection(&mut self, connection: Connection, cx: &mut Context<Self>) {
        self.connection = connection;
        cx.notify();
    }

    /// Take the torrent total and session speeds from a poll.
    pub fn apply_snapshot(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) {
        self.torrents = snapshot.torrents.total;
        self.download = format::speed(&snapshot.stats.download_speed);
        self.upload = format::speed(&snapshot.stats.upload_speed);
        cx.notify();
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

fn badge_state(connection: Connection) -> ConnectionBadgeState {
    match connection {
        Connection::Disconnected => ConnectionBadgeState::Disconnected,
        Connection::Connecting => ConnectionBadgeState::Pairing,
        Connection::Connected => ConnectionBadgeState::Paired,
        Connection::Error => ConnectionBadgeState::Error,
    }
}

impl Render for StatusBar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_1()
            .child(
                ConnectionBadge::new("status-connection", badge_state(self.connection))
                    .label(self.connection.label()),
            )
            .child(Label::new(format!("{} torrents", self.torrents)).size(LabelSize::Caption))
            .child(Label::new(format!("↓ {}", self.download)).size(LabelSize::Caption))
            .child(Label::new(format!("↑ {}", self.upload)).size(LabelSize::Caption))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use rtbit_client::{SessionStats, TorrentList};

    use super::*;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    fn snapshot() -> Snapshot {
        let read = |name: &str| std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap();
        Snapshot {
            torrents: serde_json::from_str::<TorrentList>(&read("torrents.json")).unwrap(),
            stats: serde_json::from_str::<SessionStats>(&read("stats.json")).unwrap(),
        }
    }

    #[test]
    fn badge_state_maps_every_connection() {
        assert_eq!(
            badge_state(Connection::Disconnected),
            ConnectionBadgeState::Disconnected
        );
        assert_eq!(
            badge_state(Connection::Connecting),
            ConnectionBadgeState::Pairing
        );
        assert_eq!(
            badge_state(Connection::Connected),
            ConnectionBadgeState::Paired
        );
        assert_eq!(badge_state(Connection::Error), ConnectionBadgeState::Error);
    }

    #[test]
    fn new_bar_is_disconnected_with_zero_speeds() {
        let bar = StatusBar::default();
        assert_eq!(bar.connection(), Connection::Disconnected);
        assert_eq!(bar.torrents, 0);
        assert_eq!(bar.download, "0 B/s");
        assert_eq!(bar.upload, "0 B/s");
    }

    #[gpui::test]
    fn snapshot_sets_count_and_formatted_speeds(cx: &mut TestAppContext) {
        let bar = cx.new(|_| StatusBar::new());
        let snap = snapshot();
        bar.update(cx, |bar, cx| bar.apply_snapshot(&snap, cx));
        bar.read_with(cx, |bar, _| {
            assert_eq!(bar.torrents, 1);
            assert_eq!(bar.download, format::speed(&snap.stats.download_speed));
            assert_eq!(bar.upload, format::speed(&snap.stats.upload_speed));
            assert_ne!(bar.download, "0 B/s");
            assert!(bar.download.ends_with("/s"));
        });
    }

    #[gpui::test]
    fn later_snapshot_replaces_earlier_values(cx: &mut TestAppContext) {
        let bar = cx.new(|_| StatusBar::new());
        let mut snap = snapshot();
        bar.update(cx, |bar, cx| bar.apply_snapshot(&snap, cx));
        snap.torrents.total = 7;
        snap.stats.download_speed.mbps = 0.0;
        bar.update(cx, |bar, cx| bar.apply_snapshot(&snap, cx));
        bar.read_with(cx, |bar, _| {
            assert_eq!(bar.torrents, 7);
            assert_eq!(bar.download, "0 B/s");
        });
    }

    #[gpui::test]
    fn set_connection_updates_state_and_keeps_numbers(cx: &mut TestAppContext) {
        let bar = cx.new(|_| StatusBar::new());
        let snap = snapshot();
        bar.update(cx, |bar, cx| bar.apply_snapshot(&snap, cx));
        bar.update(cx, |bar, cx| bar.set_connection(Connection::Error, cx));
        bar.read_with(cx, |bar, _| {
            assert_eq!(bar.connection(), Connection::Error);
            assert_eq!(bar.torrents, 1);
        });
    }
}

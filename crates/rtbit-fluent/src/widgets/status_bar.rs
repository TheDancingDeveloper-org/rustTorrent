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

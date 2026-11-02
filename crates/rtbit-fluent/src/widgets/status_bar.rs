//! Bottom status bar: connection badge, torrent total and live speeds.

use fluent_primitives::{Button, ButtonAppearance, ConnectionBadge, ConnectionBadgeState, Label, LabelSize, TextInput};
use gpui::{Context, IntoElement, Render, Task, Window, div, prelude::*};
use rtbit_client::{RateLimits, SettingsSnapshot};
use rtbit_ui_core::{Connection, Snapshot, format};

use crate::bridge::Bridge;

/// Status bar state. The shell pushes values in; nothing here talks to the daemon.
pub struct StatusBar {
    connection: Connection,
    torrents: u64,
    download: String,
    upload: String,
    limits: RateLimits,
    alt_enabled: bool,
    popover: bool,
    download_input: Option<gpui::Entity<TextInput>>,
    upload_input: Option<gpui::Entity<TextInput>>,
    _limit_request: Option<Task<()>>,
}

impl StatusBar {
    #[must_use]
    pub fn new() -> Self {
        Self {
            connection: Connection::Disconnected,
            torrents: 0,
            download: format::speed(&Default::default()),
            upload: format::speed(&Default::default()),
            limits: RateLimits::default(),
            alt_enabled: false,
            popover: false,
            download_input: None,
            upload_input: None,
            _limit_request: None,
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

    /// Update the limits displayed in the status bar after a settings poll.
    pub fn apply_settings(&mut self, settings: &SettingsSnapshot, cx: &mut Context<Self>) {
        self.limits = settings.limits.clone();
        self.alt_enabled = settings.alt.enabled;
        cx.notify();
    }

    /// Fetch the current session limits when a daemon connection becomes ready.
    pub fn load_settings(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<Bridge>() {
            return;
        }
        let entity = cx.entity();
        self._limit_request = Some(cx.spawn(async move |_, cx| {
            let reply = cx.update(|cx| cx.global::<Bridge>().call(|client| async move {
                client.lock().await.settings().await
            }));
            if let Ok(Ok(settings)) = reply.await {
                entity.update(cx, |bar, cx| bar.apply_settings(&settings, cx));
            }
        }));
    }

    fn toggle_alt(&mut self, cx: &mut Context<Self>) {
        let previous = self.alt_enabled;
        let next = !previous;
        self.alt_enabled = next;
        cx.notify();
        let entity = cx.entity();
        self._limit_request = Some(cx.spawn(async move |_, cx| {
            let result = cx.update(|cx| cx.global::<Bridge>().call(move |client| async move {
                client.lock().await.set_alt_enabled(next).await
            }));
            let ok = result.await.is_ok_and(|reply| reply.is_ok());
            if !ok {
                entity.update(cx, |bar, cx| {
                    bar.alt_enabled = previous;
                    cx.notify();
                });
            }
        }));
    }

    fn apply_limits(&mut self, cx: &mut Context<Self>) {
        let download_bps = self.download_input.as_ref().and_then(|input| input.read(cx).text().trim().parse::<u32>().ok());
        let upload_bps = self.upload_input.as_ref().and_then(|input| input.read(cx).text().trim().parse::<u32>().ok());
        let next = RateLimits { download_bps, upload_bps, ..self.limits.clone() };
        let previous = self.limits.clone();
        self.limits = next.clone();
        self.popover = false;
        cx.notify();
        let entity = cx.entity();
        self._limit_request = Some(cx.spawn(async move |_, cx| {
            let result = cx.update(|cx| cx.global::<Bridge>().call(move |client| async move {
                client.lock().await.set_limits(&next).await
            }));
            if !result.await.is_ok_and(|reply| reply.is_ok()) {
                entity.update(cx, |bar, cx| {
                    bar.limits = previous;
                    cx.notify();
                });
            }
        }));
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.download_input.is_none() {
            self.download_input = Some(cx.new(|_| TextInput::new().placeholder("Download B/s (blank = unlimited)")));
            self.upload_input = Some(cx.new(|_| TextInput::new().placeholder("Upload B/s (blank = unlimited)")));
        }
        let me = cx.entity();
        let toggle = me.clone();
        let download = me.clone();
        let upload = me.clone();
        let apply = me.clone();
        let limits = if self.alt_enabled {
            format!("{} / {}", format::bytes(self.limits.download_bps.unwrap_or(0) as u64), format::bytes(self.limits.upload_bps.unwrap_or(0) as u64))
        } else {
            format!("{} / {}", self.limits.download_bps.map_or_else(|| "∞".to_owned(), |v| format::bytes(v as u64)), self.limits.upload_bps.map_or_else(|| "∞".to_owned(), |v| format::bytes(v as u64)))
        };
        let popover = self.popover;
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
            .child(Button::new("status-download-limit").label(format!("DL {}", limits.split_once(" / ").map_or("∞", |v| v.0))).appearance(ButtonAppearance::Subtle).on_click(move |_, _, cx| {
                download.update(cx, |bar, cx| { bar.popover = true; cx.notify(); });
            }))
            .child(Button::new("status-upload-limit").label(format!("UL {}", limits.split_once(" / ").map_or("∞", |v| v.1))).appearance(ButtonAppearance::Subtle).on_click(move |_, _, cx| {
                upload.update(cx, |bar, cx| { bar.popover = true; cx.notify(); });
            }))
            .child(Button::new("status-alt-speed").label(if self.alt_enabled { "Alt speed: on" } else { "Alt speed: off" }).appearance(if self.alt_enabled { ButtonAppearance::Accent } else { ButtonAppearance::Subtle }).on_click(move |_, _, cx| {
                toggle.update(cx, |bar, cx| bar.toggle_alt(cx));
            }))
            .when(popover, |element| element.child(
                div().flex().gap_1()
                    .child(self.download_input.clone().expect("created above"))
                    .child(self.upload_input.clone().expect("created above"))
                    .child(Button::new("status-limit-apply").label("Apply").on_click(move |_, _, cx| {
                        apply.update(cx, |bar, cx| bar.apply_limits(cx));
                    })),
            ))
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

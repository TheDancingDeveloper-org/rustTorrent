//! System tray: the menu, the live status line, and the tray-only actions.

use std::time::Duration;

use librtbit::{
    Api, TorrentStatsState,
    api::{ApiTorrentListOpts, TorrentIdOrHash},
};
use tauri::{
    AppHandle, Manager, Wry,
    menu::{CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder},
    tray::{TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tracing::warn;

#[cfg(feature = "self-update")]
use crate::updates;
use crate::{
    EngineState,
    launch::{self, LaunchTarget},
};

pub const TRAY_ID: &str = "main";

/// Below this many MB/s a speed is shown as idle.
const IDLE_MBPS: f64 = 1.0 / 1024.0;

/// Menu items whose text or state follows the engine.
struct TrayItems {
    speed: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkAction {
    /// Pause every torrent that is running.
    Pause,
    /// Resume every torrent that is paused.
    Resume,
}

/// The tray is present, so closing the window can hide it rather than quit.
/// Without a tray nothing could bring the window back.
pub fn is_available(app: &AppHandle) -> bool {
    app.tray_by_id(TRAY_ID).is_some()
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn setup(app: &AppHandle) -> anyhow::Result<()> {
    let show = MenuItemBuilder::with_id("show", "Show rtbit").build(app)?;
    let add = MenuItemBuilder::with_id("add", "Add .torrent file…").build(app)?;
    let speed = MenuItemBuilder::with_id("speed", "Speed: idle")
        .enabled(false)
        .build(app)?;
    let pause = MenuItemBuilder::with_id("pause", "Pause All").build(app)?;
    let browser = MenuItemBuilder::with_id("browser", "Open in Browser").build(app)?;
    let autostart = CheckMenuItemBuilder::with_id("autostart", "Start at login")
        .checked(app.autolaunch().is_enabled().unwrap_or(false))
        .build(app)?;
    #[cfg(feature = "self-update")]
    let updates = MenuItemBuilder::with_id("updates", "Check for Updates…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show)
        .item(&add)
        .item(&speed)
        .separator()
        .item(&pause)
        .item(&browser)
        .separator()
        .item(&autostart);
    #[cfg(feature = "self-update")]
    let menu = menu.item(&updates);
    let menu = menu.separator().item(&quit).build()?;

    app.manage(TrayItems {
        speed,
        pause,
        autostart,
    });

    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("rtbit")
        .on_menu_event(|tray, event| on_menu_event(tray.app_handle(), event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { .. }) {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    let app_for_tray = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            refresh_status(&app_for_tray);
        }
    });

    Ok(())
}

fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main_window(app),
        "add" => pick_torrent_files(app),
        "pause" => {
            tauri::async_runtime::spawn(toggle_transfers(app.clone()));
        }
        "browser" => {
            if let Some(state) = app.try_state::<EngineState>() {
                let _ = open::that(format!("http://{}", state.listen_addr));
            }
        }
        "autostart" => toggle_autostart(app),
        #[cfg(feature = "self-update")]
        "updates" => updates::check_for_updates(app.clone()),
        "quit" => confirm_quit(app),
        _ => {}
    }
}

/// Re-reads the engine and brings the status line, Pause/Resume label and tooltip up to date.
/// Everything is derived from live torrent state, so nothing here can drift from it.
fn refresh_status(app: &AppHandle) {
    let (Some(state), Some(items)) = (app.try_state::<EngineState>(), app.try_state::<TrayItems>())
    else {
        return;
    };
    let stats = state.api.api_session_stats();
    let states: Vec<TorrentStatsState> = torrent_states(&state.api)
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    let status = status_text(&states, stats.download_speed.mbps, stats.upload_speed.mbps);
    let action = bulk_action(&states);

    let _ = items.speed.set_text(format!("Speed: {status}"));
    let _ = items.pause.set_text(match action {
        Some(BulkAction::Resume) => "Resume All",
        Some(BulkAction::Pause) | None => "Pause All",
    });
    let _ = items.pause.set_enabled(action.is_some());
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(format!("rtbit - {status}")));
    }
}

async fn toggle_transfers(app: AppHandle) {
    let Some(api) = app.try_state::<EngineState>().map(|s| s.api.clone()) else {
        return;
    };
    let torrents = torrent_states(&api);
    let states: Vec<TorrentStatsState> = torrents.iter().map(|(_, s)| *s).collect();
    let Some(action) = bulk_action(&states) else {
        return;
    };

    let mut failed = Vec::new();
    for (id, state) in torrents {
        if !applies_to(action, &state) {
            continue;
        }
        let idx = TorrentIdOrHash::Id(id);
        let result = match action {
            BulkAction::Pause => api.api_torrent_action_pause(idx).await,
            BulkAction::Resume => api.api_torrent_action_start(idx).await,
        };
        if let Err(e) = result {
            warn!("tray: could not {action:?} torrent {id}: {e:#}");
            failed.push(id.to_string());
        }
    }
    if !failed.is_empty() {
        crate::send_notification(
            &app,
            "Some torrents did not change",
            &format!("Could not update torrent(s): {}", failed.join(", ")),
        );
    }
    refresh_status(&app);
}

fn pick_torrent_files(app: &AppHandle) {
    let handle = app.clone();
    app.dialog()
        .file()
        .add_filter("Torrent files", &["torrent"])
        .pick_files(move |picked| {
            let targets = picked
                .unwrap_or_default()
                .into_iter()
                .filter_map(|path| path.into_path().ok())
                .map(LaunchTarget::TorrentFile)
                .collect();
            launch::spawn_add(handle, targets);
        });
}

fn toggle_autostart(app: &AppHandle) {
    let launcher = app.autolaunch();
    let change = match launcher.is_enabled() {
        Ok(true) => launcher.disable(),
        Ok(false) => launcher.enable(),
        Err(e) => Err(e),
    };
    let enabled = launcher.is_enabled().unwrap_or(false);
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.autostart.set_checked(enabled);
    }
    if let Err(e) = change {
        warn!("could not change start-at-login: {e}");
        crate::send_notification(app, "Start at login", &format!("Could not change it: {e}"));
    }
}

/// Quitting with transfers running asks first. Quitting from idle does not.
fn confirm_quit(app: &AppHandle) {
    let active = app.try_state::<EngineState>().map_or(0, |state| {
        torrent_states(&state.api)
            .iter()
            .filter(|(_, s)| is_active(s))
            .count()
    });
    if active == 0 {
        app.exit(0);
        return;
    }
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "{active} torrent(s) are still downloading or seeding. Quit anyway?"
        ))
        .title("Quit rtbit?")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Quit".to_owned(),
            "Cancel".to_owned(),
        ))
        .show(move |quit| {
            if quit {
                handle.exit(0);
            }
        });
}

fn torrent_states(api: &Api) -> Vec<(usize, TorrentStatsState)> {
    api.api_torrent_list_ext(ApiTorrentListOpts {
        with_stats: true,
        ..Default::default()
    })
    .torrents
    .iter()
    .filter_map(|torrent| Some((torrent.id?, torrent.stats.as_ref()?.state)))
    .collect()
}

fn is_active(state: &TorrentStatsState) -> bool {
    matches!(
        state,
        TorrentStatsState::Initializing | TorrentStatsState::Live
    )
}

fn is_paused(state: &TorrentStatsState) -> bool {
    matches!(state, TorrentStatsState::Paused)
}

fn applies_to(action: BulkAction, state: &TorrentStatsState) -> bool {
    match action {
        BulkAction::Pause => is_active(state),
        BulkAction::Resume => is_paused(state),
    }
}

/// What the Pause/Resume item does: pause if anything is running, otherwise resume if anything is paused.
pub fn bulk_action(states: &[TorrentStatsState]) -> Option<BulkAction> {
    if states.iter().any(is_active) {
        Some(BulkAction::Pause)
    } else if states.iter().any(is_paused) {
        Some(BulkAction::Resume)
    } else {
        None
    }
}

/// The one-line status shown in the tooltip and the speed item.
pub fn status_text(states: &[TorrentStatsState], download_mbps: f64, upload_mbps: f64) -> String {
    if states.iter().any(is_active) {
        if download_mbps < IDLE_MBPS && upload_mbps < IDLE_MBPS {
            "idle".to_owned()
        } else {
            format!(
                "D: {} | U: {}",
                format_speed(download_mbps),
                format_speed(upload_mbps)
            )
        }
    } else if states.iter().any(is_paused) {
        "paused".to_owned()
    } else {
        "idle".to_owned()
    }
}

pub fn format_speed(mbps: f64) -> String {
    if mbps < IDLE_MBPS {
        "0 B/s".to_string()
    } else if mbps < 1.0 {
        format!("{:.0} KB/s", mbps * 1024.0)
    } else {
        format!("{:.1} MB/s", mbps)
    }
}

#[cfg(test)]
mod tests {
    use librtbit::TorrentStatsState::{Error, Initializing, Live, Paused};

    use super::{BulkAction, bulk_action, format_speed, status_text};

    #[test]
    fn pause_all_is_offered_while_anything_runs() {
        assert_eq!(bulk_action(&[Live]), Some(BulkAction::Pause));
        assert_eq!(bulk_action(&[Initializing]), Some(BulkAction::Pause));
        assert_eq!(bulk_action(&[Paused, Live]), Some(BulkAction::Pause));
    }

    #[test]
    fn resume_all_is_offered_only_when_everything_active_is_paused() {
        assert_eq!(bulk_action(&[Paused]), Some(BulkAction::Resume));
        assert_eq!(bulk_action(&[Paused, Error]), Some(BulkAction::Resume));
    }

    #[test]
    fn nothing_to_toggle_without_torrents_or_only_errors() {
        assert_eq!(bulk_action(&[]), None);
        assert_eq!(bulk_action(&[Error]), None);
    }

    #[test]
    fn status_reports_idle_paused_and_speeds() {
        assert_eq!(status_text(&[], 0.0, 0.0), "idle");
        assert_eq!(status_text(&[Paused], 0.0, 0.0), "paused");
        assert_eq!(status_text(&[Live], 0.0, 0.0), "idle");
        assert_eq!(
            status_text(&[Live, Paused], 2.5, 0.5),
            "D: 2.5 MB/s | U: 512 KB/s"
        );
    }

    #[test]
    fn speeds_are_human_scaled() {
        assert_eq!(format_speed(0.0), "0 B/s");
        assert_eq!(format_speed(0.5), "512 KB/s");
        assert_eq!(format_speed(2.5), "2.5 MB/s");
    }
}

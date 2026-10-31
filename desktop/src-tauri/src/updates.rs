//! Manual update check. An update is only installed after its signature checks
//! out against the public key in `tauri.conf.json`, so an empty key means
//! updates fail closed.

use rtbit_desktop_menu::update::{self, AvailableUpdate, UpdateFeed, UpdateOutcome, UpdatePrompt};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::{Update, UpdaterExt};
use tracing::warn;

pub fn check_for_updates(app: AppHandle) {
    // The feed and the dialogs block, so the check runs off the event loop thread.
    tauri::async_runtime::spawn_blocking(move || {
        let current = app.package_info().version.to_string();
        let mut feed = TauriFeed {
            app: app.clone(),
            pending: None,
        };
        let outcome = update::run_check(&mut feed, &current, &mut |prompt| show(&app, prompt));
        match outcome {
            UpdateOutcome::Installed => app.restart(),
            UpdateOutcome::Failed(e) => warn!("update check failed: {e}"),
            UpdateOutcome::UpToDate | UpdateOutcome::Declined => {}
        }
    });
}

/// The Tauri updater behind the testable check in `rtbit_desktop_menu::update`.
struct TauriFeed {
    app: AppHandle,
    pending: Option<Update>,
}

impl UpdateFeed for TauriFeed {
    fn check(&mut self) -> Result<Option<AvailableUpdate>, String> {
        let updater = self.app.updater().map_err(|e| e.to_string())?;
        let update = tauri::async_runtime::block_on(updater.check()).map_err(|e| e.to_string())?;
        Ok(update.map(|update| {
            let available = AvailableUpdate {
                version: update.version.clone(),
                current_version: update.current_version.clone(),
            };
            self.pending = Some(update);
            available
        }))
    }

    fn install(&mut self, _: &AvailableUpdate) -> Result<(), String> {
        let update = self.pending.take().ok_or("no update to install")?;
        tauri::async_runtime::block_on(update.download_and_install(|_, _| {}, || {}))
            .map_err(|e| e.to_string())
    }
}

fn show(app: &AppHandle, prompt: UpdatePrompt) -> bool {
    match prompt {
        UpdatePrompt::Info { title, body } => {
            app.dialog()
                .message(body)
                .title(title)
                .kind(MessageDialogKind::Info)
                .blocking_show();
            false
        }
        UpdatePrompt::Offer { title, body } => app
            .dialog()
            .message(body)
            .title(title)
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Install".to_owned(),
                "Later".to_owned(),
            ))
            .blocking_show(),
    }
}

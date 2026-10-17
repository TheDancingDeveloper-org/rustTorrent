//! Manual update check. An update is only installed after its signature checks
//! out against the public key in `tauri.conf.json`, so an empty key means
//! updates fail closed.

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;
use tracing::warn;

pub fn check_for_updates(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_check(&app).await {
            warn!("update check failed: {e}");
            info_dialog(&app, "Update check failed", &e);
        }
    });
}

async fn run_check(app: &AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        let current = app.package_info().version.to_string();
        info_dialog(
            app,
            "rtbit is up to date",
            &format!("rtbit {current} is the latest version."),
        );
        return Ok(());
    };

    // Blocking here is fine: this runs on a task, not the event loop thread.
    let install = app
        .dialog()
        .message(format!(
            "rtbit {} is available (you have {}). Download and install it now?",
            update.version, update.current_version
        ))
        .title("Update available")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install".to_owned(),
            "Later".to_owned(),
        ))
        .blocking_show();
    if !install {
        return Ok(());
    }

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

fn info_dialog(app: &AppHandle, title: &str, body: &str) {
    app.dialog()
        .message(body)
        .title(title)
        .kind(MessageDialogKind::Info)
        .show(|_| {});
}

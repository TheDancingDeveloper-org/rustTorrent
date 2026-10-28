//! Turns the menu description into the window's native menu bar.

use rtbit_desktop_menu::{MenuEntry, menus};
use tauri::{
    AppHandle,
    menu::{Menu, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
};

use crate::tray;

/// Build the menu bar and attach it to the window named `main`.
pub fn install(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let mut bar = Menu::new(app)?;
    for menu in menus() {
        let mut submenu = SubmenuBuilder::new(app, menu.label);
        for entry in menu.entries {
            match entry {
                MenuEntry::Separator => {
                    submenu = submenu.item(&PredefinedMenuItem::separator(app)?);
                }
                MenuEntry::Item { id, label } => {
                    let item = MenuItemBuilder::with_id(*id, *label).build(app)?;
                    submenu = submenu.item(&item);
                }
            }
        }
        bar.append(&submenu.build()?)?;
    }
    if let Some(window) = app.get_webview_window("main") {
        window.set_menu(bar.clone())?;
        window.on_menu_event(|window, event| {
            tray::on_menu_id(window.app_handle(), event.id().as_ref());
        });
    }
    Ok(bar)
}

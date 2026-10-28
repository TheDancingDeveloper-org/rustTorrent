//! RSS, read-only. Feeds and items, plus "download this item".
//!
//! There is no rule editor. A downloaded item is shown but the button is off.

use fluent2_egui::controls::{Appearance, Button};
use rtbit_client::{RssFeed, RssItem, RssSnapshot};

/// What the RSS screen is showing, from one read of the daemon.
#[derive(Clone, Debug, Default)]
pub struct RssView {
    pub feeds: Vec<RssFeed>,
    pub items: Vec<RssItem>,
}

impl RssView {
    pub fn from_snapshot(snap: &RssSnapshot) -> Self {
        Self {
            feeds: snap.feeds.clone(),
            items: snap.items.clone(),
        }
    }
}

/// Draw the lists. Returns the item id the user asked to download.
pub fn draw_rss(ui: &mut egui::Ui, view: &RssView, busy: bool, error: &str) -> Option<String> {
    ui.label("Feeds");
    if view.feeds.is_empty() {
        ui.label("No feeds.");
    }
    for feed in &view.feeds {
        let state = if feed.enabled { "on" } else { "off" };
        ui.label(format!("{} ({state})", feed.name));
    }
    ui.add_space(8.0);
    ui.label("Items");
    if view.items.is_empty() {
        ui.label("No items.");
    }
    let mut download = None;
    for item in &view.items {
        ui.horizontal(|ui| {
            ui.label(item.title.as_str());
            if item.downloaded {
                ui.label("Downloaded");
            } else if item.url.is_none() {
                ui.label("No link");
            } else if ui
                .add_enabled(
                    !busy,
                    Button::new("Download").appearance(Appearance::Primary),
                )
                .clicked()
            {
                download = Some(item.id.clone());
            }
        });
    }
    if !error.is_empty() {
        ui.add_space(8.0);
        ui.colored_label(egui::Color32::from_rgb(196, 43, 28), error);
    }
    download
}

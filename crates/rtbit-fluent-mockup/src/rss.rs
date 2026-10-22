//! The RSS page: Items, Feeds and Rules tabs, each a DataGrid, with Add Feed and Add Rule
//! dialogs. Mirrors `webui/src/components/RSSPage.tsx`.

use egui::{Context, RichText, Ui};
use fluent2_egui::controls::{Appearance, Button, ButtonSize};
use fluent2_egui::data::data_grid::{Align, Column, ColumnWidth};
use fluent2_egui::{
    Badge, BadgeColor, BadgeSize, Checkbox, DataGrid, DataGridSize, DataGridState, Dialog,
    DialogResult, Dropdown, Field, IconName, Input, RowId, SelectionMode, SpinButton, TabList,
};

use crate::data::Rng;

pub struct Feed {
    pub name: String,
    pub url: String,
    pub interval: f64,
    pub category: String,
    pub filter: String,
    pub enabled: bool,
    pub auto: bool,
}

pub struct Rule {
    pub name: String,
    pub pattern: String,
    pub feeds: Vec<bool>,
    pub category: String,
    pub priority: Option<usize>,
    pub enabled: bool,
}

pub struct Item {
    pub title: String,
    pub feed: usize,
    pub published: String,
    pub size: u64,
    pub done: bool,
}

/// Which editor dialog is open, and the draft it edits.
pub enum Editor {
    Feed(Option<usize>, Feed),
    Rule(Option<usize>, Rule),
}

pub struct RssState {
    pub tab: usize,
    pub feeds: Vec<Feed>,
    pub rules: Vec<Rule>,
    pub items: Vec<Item>,
    pub feed_filter: Option<usize>,
    pub editor: Option<Editor>,
    grids: [DataGridState; 3],
}

const PRIORITIES: [&str; 4] = ["Low", "Normal", "High", "Force"];

impl Default for RssState {
    fn default() -> Self {
        let feed =
            |name: &str, url: &str, interval: f64, category: &str, enabled: bool, auto: bool| {
                Feed {
                    name: name.to_owned(),
                    url: url.to_owned(),
                    interval,
                    category: category.to_owned(),
                    filter: String::new(),
                    enabled,
                    auto,
                }
            };
        let feeds = vec![
            feed(
                "Linux ISOs",
                "https://distrowatch.example/torrents.rss",
                900.0,
                "Linux ISOs",
                true,
                true,
            ),
            feed(
                "Blender Open Movies",
                "https://studio.example/open-movies/rss",
                3600.0,
                "Movies",
                true,
                false,
            ),
            feed(
                "Public Domain Audio",
                "https://archive.example/audio/rss",
                1800.0,
                "Music",
                true,
                true,
            ),
            feed(
                "Ebooks weekly",
                "https://books.example/weekly.xml",
                86_400.0,
                "Books",
                false,
                false,
            ),
        ];
        let rule =
            |name: &str, pattern: &str, feeds: Vec<bool>, category: &str, priority: usize| Rule {
                name: name.to_owned(),
                pattern: pattern.to_owned(),
                feeds,
                category: category.to_owned(),
                priority: Some(priority),
                enabled: true,
            };
        let rules = vec![
            rule(
                "Ubuntu LTS",
                r"ubuntu-\d\d\.04.*desktop",
                vec![true, false, false, false],
                "Linux ISOs",
                2,
            ),
            rule(
                "Debian netinst",
                r"debian-.*netinst",
                vec![true, false, false, false],
                "Linux ISOs",
                1,
            ),
            rule(
                "4K open movies",
                r"2160p.*HEVC",
                vec![false, true, false, false],
                "Movies",
                1,
            ),
        ];
        let mut rng = Rng::new(0x255);
        let titles = [
            "ubuntu-24.04.3-desktop-amd64.iso",
            "debian-13.1.0-amd64-netinst.iso",
            "Fedora-Workstation-Live-x86_64-43",
            "Sintel.2010.2160p.WEB-DL.HEVC",
            "Big.Buck.Bunny.2008.2160p.WEB-DL.HEVC",
            "Musopen - Chopin Nocturnes [FLAC]",
            "Kevin MacLeod - Royalty Free Collection [FLAC]",
            "archlinux-2026.10.01-x86_64.iso",
            "Project Gutenberg Top 100 (EPUB)",
            "Tears.of.Steel.2012.1080p.BluRay.x264",
        ];
        let items = (0..48)
            .map(|i| {
                let title = titles[i % titles.len()];
                let feed = match title {
                    t if t.contains(".iso") || t.contains("Fedora") => 0,
                    t if t.contains("FLAC") => 2,
                    t if t.contains("EPUB") => 3,
                    _ => 1,
                };
                Item {
                    title: format!(
                        "{title}{}",
                        if i >= titles.len() {
                            format!(" (r{})", i / titles.len())
                        } else {
                            String::new()
                        }
                    ),
                    feed,
                    published: format!(
                        "Oct {}, {:02}:{:02}",
                        22 - (i / 4).min(21),
                        rng.below(24),
                        rng.below(60)
                    ),
                    size: (rng.f32() * 6.0e9) as u64 + 50_000_000,
                    done: rng.chance(0.4),
                }
            })
            .collect();
        Self {
            tab: 0,
            feeds,
            rules,
            items,
            feed_filter: None,
            editor: None,
            grids: Default::default(),
        }
    }
}

/// Draws the page. Returns a toast message when something happened.
pub fn show(ui: &mut Ui, s: &mut RssState, size: DataGridSize) -> Option<String> {
    let mut toast = None;
    let feeds_label = format!("Feeds ({})", s.feeds.len());
    let rules_label = format!("Rules ({})", s.rules.len());
    ui.horizontal(|ui| {
        TabList::new("rss_tabs", &mut s.tab)
            .tab_icon(IconName::Mail, "Items")
            .tab_icon(IconName::Link, feeds_label)
            .tab_icon(IconName::Filter, rules_label)
            .show(ui);
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| match s.tab {
                0 => {
                    let mut names: Vec<&str> = vec!["All feeds"];
                    names.extend(s.feeds.iter().map(|f| f.name.as_str()));
                    ui.add(
                        Dropdown::new("rss_feed_filter", &mut s.feed_filter, &names)
                            .placeholder("All feeds")
                            .width(200.0),
                    );
                }
                1 => {
                    if ui
                        .add(
                            Button::new("Add Feed")
                                .icon(IconName::Add)
                                .appearance(Appearance::Primary),
                        )
                        .clicked()
                    {
                        s.editor = Some(Editor::Feed(
                            None,
                            Feed {
                                name: String::new(),
                                url: String::new(),
                                interval: 900.0,
                                category: String::new(),
                                filter: String::new(),
                                enabled: true,
                                auto: false,
                            },
                        ));
                    }
                }
                _ => {
                    if ui
                        .add(
                            Button::new("Add Rule")
                                .icon(IconName::Add)
                                .appearance(Appearance::Primary),
                        )
                        .clicked()
                    {
                        s.editor = Some(Editor::Rule(
                            None,
                            Rule {
                                name: String::new(),
                                pattern: String::new(),
                                feeds: vec![false; s.feeds.len()],
                                category: String::new(),
                                priority: Some(1),
                                enabled: true,
                            },
                        ));
                    }
                }
            },
        );
    });
    ui.add_space(6.0);
    match s.tab {
        0 => toast = items(ui, s, size),
        1 => toast = feeds(ui, s, size),
        _ => toast = rules(ui, s, size).or(toast),
    }
    if let Some(t) = editor(ui.ctx(), s) {
        toast = Some(t);
    }
    toast
}

fn items(ui: &mut Ui, s: &mut RssState, size: DataGridSize) -> Option<String> {
    let rows: Vec<usize> = (0..s.items.len())
        .filter(|&i| match s.feed_filter {
            None | Some(0) => true,
            Some(f) => s.items[i].feed == f - 1,
        })
        .collect();
    if rows.is_empty() {
        ui.label("No RSS items found. Add a feed to get started.");
        return None;
    }
    let cols = [
        Column::new("title", "Title")
            .width(ColumnWidth::Fraction(1.0))
            .min_width(260.0),
        Column::new("feed", "Feed").width(ColumnWidth::Fixed(180.0)),
        Column::new("published", "Published").width(ColumnWidth::Fixed(130.0)),
        Column::new("size", "Size")
            .width(ColumnWidth::Fixed(96.0))
            .align(Align::End),
        Column::new("status", "Status").width(ColumnWidth::Fixed(96.0)),
        Column::new("dl", "")
            .width(ColumnWidth::Fixed(48.0))
            .sortable(false)
            .align(Align::Center),
    ];
    let mut clicked = None;
    DataGrid::new("rss_items", &cols, rows.len())
        .label("RSS items")
        .size(size)
        .selection_mode(SelectionMode::Single)
        .row_id(|i| RowId(rows[i] as u64))
        .show(ui, &mut s.grids[0], |cell, ui| {
            let item = &s.items[rows[cell.row]];
            match cell.column {
                "title" => {
                    ui.add(egui::Label::new(&item.title).truncate());
                }
                "feed" => {
                    ui.label(&s.feeds[item.feed].name);
                }
                "published" => {
                    ui.label(&item.published);
                }
                "size" => {
                    ui.label(crate::data::bytes(item.size));
                }
                "status" => {
                    if item.done {
                        ui.label(
                            RichText::new("✓ Done")
                                .color(egui::Color32::from_rgb(0x10, 0x7c, 0x10)),
                        );
                    } else {
                        ui.label(RichText::new("Pending").weak());
                    }
                }
                _ => {
                    if ui
                        .add(
                            Button::icon_only(IconName::ArrowDownload, "Download")
                                .size(ButtonSize::Small)
                                .appearance(Appearance::Subtle)
                                .id_salt(("rss_dl", cell.row)),
                        )
                        .clicked()
                    {
                        clicked = Some(rows[cell.row]);
                    }
                }
            }
        });
    clicked.map(|i| {
        s.items[i].done = true;
        format!("Queued \"{}\"", s.items[i].title)
    })
}

fn status_badge(ui: &mut Ui, enabled: bool) {
    let (text, color) = if enabled {
        ("Active", BadgeColor::Success)
    } else {
        ("Disabled", BadgeColor::Subtle)
    };
    ui.add(Badge::new(text).color(color).size(BadgeSize::Small));
}

/// Edit and Delete buttons for a row. Returns (edit, delete).
fn row_actions(ui: &mut Ui, salt: &str, row: usize) -> (bool, bool) {
    let edit = ui
        .add(
            Button::icon_only(IconName::Edit, "Edit")
                .size(ButtonSize::Small)
                .appearance(Appearance::Subtle)
                .id_salt((salt, "e", row)),
        )
        .clicked();
    let delete = ui
        .add(
            Button::icon_only(IconName::Delete, "Delete")
                .size(ButtonSize::Small)
                .appearance(Appearance::Subtle)
                .id_salt((salt, "d", row)),
        )
        .clicked();
    (edit, delete)
}

fn feeds(ui: &mut Ui, s: &mut RssState, size: DataGridSize) -> Option<String> {
    if s.feeds.is_empty() {
        ui.label("No RSS feeds configured. Click \"Add Feed\" to add one.");
        return None;
    }
    let cols = [
        Column::new("name", "Name").width(ColumnWidth::Fixed(200.0)),
        Column::new("url", "URL").width(ColumnWidth::Fraction(1.0)),
        Column::new("interval", "Interval")
            .width(ColumnWidth::Fixed(90.0))
            .align(Align::End),
        Column::new("category", "Category").width(ColumnWidth::Fixed(120.0)),
        Column::new("status", "Status").width(ColumnWidth::Fixed(96.0)),
        Column::new("actions", "")
            .width(ColumnWidth::Fixed(84.0))
            .sortable(false),
    ];
    let (mut edit, mut delete) = (None, None);
    DataGrid::new("rss_feeds", &cols, s.feeds.len())
        .label("RSS feeds")
        .size(size)
        .show(ui, &mut s.grids[1], |cell, ui| {
            let f = &s.feeds[cell.row];
            match cell.column {
                "name" => {
                    ui.label(&f.name);
                }
                "url" => {
                    ui.add(egui::Label::new(RichText::new(&f.url).monospace()).truncate());
                }
                "interval" => {
                    ui.label(format!("{}m", f.interval as u64 / 60));
                }
                "category" => {
                    ui.label(if f.category.is_empty() {
                        "—"
                    } else {
                        &f.category
                    });
                }
                "status" => status_badge(ui, f.enabled),
                _ => {
                    ui.horizontal(|ui| {
                        let (e, d) = row_actions(ui, "feed", cell.row);
                        if e {
                            edit = Some(cell.row);
                        }
                        if d {
                            delete = Some(cell.row);
                        }
                    });
                }
            }
        });
    if let Some(i) = edit {
        let f = &s.feeds[i];
        s.editor = Some(Editor::Feed(
            Some(i),
            Feed {
                name: f.name.clone(),
                url: f.url.clone(),
                interval: f.interval,
                category: f.category.clone(),
                filter: f.filter.clone(),
                enabled: f.enabled,
                auto: f.auto,
            },
        ));
    }
    delete.map(|i| {
        let f = s.feeds.remove(i);
        for r in &mut s.rules {
            if i < r.feeds.len() {
                r.feeds.remove(i);
            }
        }
        s.items.retain(|it| it.feed != i);
        for it in &mut s.items {
            if it.feed > i {
                it.feed -= 1;
            }
        }
        format!("Deleted feed \"{}\"", f.name)
    })
}

fn rules(ui: &mut Ui, s: &mut RssState, size: DataGridSize) -> Option<String> {
    if s.rules.is_empty() {
        ui.label("No download rules. Click \"Add Rule\" to create one.");
        return None;
    }
    let cols = [
        Column::new("name", "Name").width(ColumnWidth::Fixed(180.0)),
        Column::new("pattern", "Pattern").width(ColumnWidth::Fraction(1.0)),
        Column::new("feeds", "Feeds").width(ColumnWidth::Fixed(200.0)),
        Column::new("category", "Category").width(ColumnWidth::Fixed(120.0)),
        Column::new("status", "Status").width(ColumnWidth::Fixed(96.0)),
        Column::new("actions", "")
            .width(ColumnWidth::Fixed(84.0))
            .sortable(false),
    ];
    let (mut edit, mut delete) = (None, None);
    DataGrid::new("rss_rules", &cols, s.rules.len())
        .label("RSS rules")
        .size(size)
        .show(ui, &mut s.grids[2], |cell, ui| {
            let r = &s.rules[cell.row];
            match cell.column {
                "name" => {
                    ui.label(&r.name);
                }
                "pattern" => {
                    ui.label(RichText::new(&r.pattern).monospace());
                }
                "feeds" => {
                    let names: Vec<&str> = r
                        .feeds
                        .iter()
                        .enumerate()
                        .filter(|(_, on)| **on)
                        .filter_map(|(i, _)| s.feeds.get(i).map(|f| f.name.as_str()))
                        .collect();
                    ui.add(egui::Label::new(names.join(", ")).truncate());
                }
                "category" => {
                    ui.label(if r.category.is_empty() {
                        "—"
                    } else {
                        &r.category
                    });
                }
                "status" => status_badge(ui, r.enabled),
                _ => {
                    ui.horizontal(|ui| {
                        let (e, d) = row_actions(ui, "rule", cell.row);
                        if e {
                            edit = Some(cell.row);
                        }
                        if d {
                            delete = Some(cell.row);
                        }
                    });
                }
            }
        });
    if let Some(i) = edit {
        let r = &s.rules[i];
        let mut feeds = r.feeds.clone();
        feeds.resize(s.feeds.len(), false);
        s.editor = Some(Editor::Rule(
            Some(i),
            Rule {
                name: r.name.clone(),
                pattern: r.pattern.clone(),
                feeds,
                category: r.category.clone(),
                priority: r.priority,
                enabled: r.enabled,
            },
        ));
    }
    delete.map(|i| format!("Deleted rule \"{}\"", s.rules.remove(i).name))
}

fn editor(ctx: &Context, s: &mut RssState) -> Option<String> {
    let mut open = s.editor.is_some();
    let feed_names: Vec<String> = s.feeds.iter().map(|f| f.name.clone()).collect();
    let editor = s.editor.as_mut()?;
    let title = match editor {
        Editor::Feed(None, _) => "Add Feed",
        Editor::Feed(Some(_), _) => "Edit Feed",
        Editor::Rule(None, _) => "Add Rule",
        Editor::Rule(Some(_), _) => "Edit Rule",
    };
    let result = Dialog::new(("rss_editor", title))
        .title(title)
        .width(480.0)
        .actions(|a| {
            a.primary("Save");
            a.secondary("Cancel");
        })
        .show(ctx, &mut open, |ui| match editor {
            Editor::Feed(_, f) => {
                Field::new("Name").required(true).show(ui, |ui| {
                    ui.add(
                        Input::new(&mut f.name)
                            .placeholder("My Feed")
                            .desired_width(440.0)
                            .id_salt("fn"),
                    )
                });
                Field::new("URL").required(true).show(ui, |ui| {
                    ui.add(
                        Input::new(&mut f.url)
                            .placeholder("https://example.com/rss")
                            .desired_width(440.0)
                            .id_salt("fu"),
                    )
                });
                ui.horizontal(|ui| {
                    Field::new("Poll Interval (sec)").show(ui, |ui| {
                        ui.add(
                            SpinButton::new(&mut f.interval)
                                .range(60.0..=86_400.0)
                                .step(60.0),
                        )
                    });
                    Field::new("Category").show(ui, |ui| {
                        ui.add(
                            Input::new(&mut f.category)
                                .placeholder("Optional")
                                .desired_width(200.0)
                                .id_salt("fc"),
                        )
                    });
                });
                Field::new("Filter Regex").show(ui, |ui| {
                    ui.add(
                        Input::new(&mut f.filter)
                            .placeholder("Optional title filter")
                            .desired_width(440.0)
                            .id_salt("ff"),
                    )
                });
                ui.add(Checkbox::bool(&mut f.enabled, "Enabled"));
                ui.add(Checkbox::bool(&mut f.auto, "Auto-download"));
            }
            Editor::Rule(_, r) => {
                Field::new("Name").required(true).show(ui, |ui| {
                    ui.add(
                        Input::new(&mut r.name)
                            .placeholder("Rule name")
                            .desired_width(440.0)
                            .id_salt("rn"),
                    )
                });
                Field::new("Match Regex").required(true).show(ui, |ui| {
                    ui.add(
                        Input::new(&mut r.pattern)
                            .placeholder("e.g. 1080p.*HEVC")
                            .desired_width(440.0)
                            .id_salt("rp"),
                    )
                });
                ui.label("Feeds");
                if feed_names.is_empty() {
                    ui.label(RichText::new("No feeds configured").weak());
                }
                for (name, on) in feed_names.iter().zip(r.feeds.iter_mut()) {
                    ui.add(Checkbox::bool(on, name.as_str()));
                }
                ui.horizontal(|ui| {
                    Field::new("Category").show(ui, |ui| {
                        ui.add(
                            Input::new(&mut r.category)
                                .placeholder("Optional")
                                .desired_width(200.0)
                                .id_salt("rc"),
                        )
                    });
                    Field::new("Priority").show(ui, |ui| {
                        ui.add(
                            Dropdown::new("rule_priority", &mut r.priority, &PRIORITIES)
                                .width(140.0),
                        )
                    });
                });
                ui.add(Checkbox::bool(&mut r.enabled, "Enabled"));
            }
        });
    if let DialogResult::Closed(fluent2_egui::CloseReason::Button("Save")) = result {
        let done = s.editor.take();
        return match done {
            Some(Editor::Feed(slot, f)) => {
                let msg = format!("Saved feed \"{}\"", f.name);
                match slot {
                    Some(i) => s.feeds[i] = f,
                    None => {
                        s.feeds.push(f);
                        for r in &mut s.rules {
                            r.feeds.push(false);
                        }
                    }
                }
                Some(msg)
            }
            Some(Editor::Rule(slot, r)) => {
                let msg = format!("Saved rule \"{}\"", r.name);
                match slot {
                    Some(i) => s.rules[i] = r,
                    None => s.rules.push(r),
                }
                Some(msg)
            }
            None => None,
        };
    }
    if !open || matches!(result, DialogResult::Closed(_)) {
        s.editor = None;
    }
    None
}

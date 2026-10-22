//! The settings dialog: a search box, ten tabs of forms, and Cancel / OK. Mirrors
//! `webui/src/components/config` (tab order, labels, help text and restart notes).

use egui::{Grid, RichText, ScrollArea, Ui};
use fluent2_egui::controls::{Appearance, Button};
use fluent2_egui::surfaces::{Intent, MessageBar};
use fluent2_egui::theme::{Theme, c};
use fluent2_egui::{
    Badge, BadgeColor, BadgeSize, Checkbox, Dropdown, Field, Icon, IconName, Input, SearchBox,
    SpinButton, TabList, TypeRamp, rich,
};

pub const TABS: [&str; 10] = [
    "General",
    "Folders",
    "Speed",
    "Connections",
    "Queue",
    "Seeding",
    "DHT",
    "Security",
    "HTTP API",
    "RSS",
];

/// Every searchable setting: label, tab index, restart required.
const REGISTRY: [(&str, usize, bool); 24] = [
    ("Version", 0, false),
    ("Web Seeds", 0, true),
    ("Default download folder", 1, false),
    ("Completed folder", 1, false),
    ("Data directory", 1, true),
    ("Download rate limit", 2, false),
    ("Upload rate limit", 2, false),
    ("Peer limit", 2, false),
    ("Concurrent init limit", 2, false),
    ("Alternative speed limits", 2, false),
    ("Speed limit schedule", 2, false),
    ("Encryption", 3, false),
    ("Listen port", 3, true),
    ("Bind address (peer listener)", 3, true),
    ("Max active downloads", 4, false),
    ("Max active uploads", 4, false),
    ("Max active total", 4, false),
    ("Seed ratio limit", 5, false),
    ("Seed time limit", 5, false),
    ("DHT enabled", 6, true),
    ("Change password", 7, false),
    ("API URL", 8, false),
    ("HTTP API bind address", 8, true),
    ("Feed History Limit", 9, false),
];

#[derive(Clone)]
pub struct Settings {
    pub tab: usize,
    pub search: String,
    pub download_folder: String,
    pub completed_folder: String,
    pub browsing: Option<usize>,
    pub down_limit: f64,
    pub up_limit: f64,
    pub peer_limit: f64,
    pub init_limit: f64,
    pub alt_enabled: bool,
    pub alt_down: f64,
    pub alt_up: f64,
    pub schedule: bool,
    pub schedule_from: String,
    pub schedule_to: String,
    pub days: [bool; 7],
    pub encryption: Option<usize>,
    pub max_downloads: f64,
    pub max_uploads: f64,
    pub max_total: f64,
    pub ratio_enabled: bool,
    pub ratio: f64,
    pub time_enabled: bool,
    pub hours: f64,
    pub current_password: String,
    pub new_username: String,
    pub new_password: String,
    pub confirm_password: String,
    pub credentials_message: Option<(bool, &'static str)>,
    pub feed_history: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tab: 0,
            search: String::new(),
            download_folder: "/srv/downloads".to_owned(),
            completed_folder: String::new(),
            browsing: None,
            down_limit: 0.0,
            up_limit: 0.0,
            peer_limit: 0.0,
            init_limit: 3.0,
            alt_enabled: false,
            alt_down: 512.0,
            alt_up: 128.0,
            schedule: false,
            schedule_from: "08:00".to_owned(),
            schedule_to: "18:00".to_owned(),
            days: [true, true, true, true, true, false, false],
            encryption: Some(1),
            max_downloads: 5.0,
            max_uploads: 10.0,
            max_total: 15.0,
            ratio_enabled: false,
            ratio: 2.0,
            time_enabled: false,
            hours: 24.0,
            current_password: String::new(),
            new_username: String::new(),
            new_password: String::new(),
            confirm_password: String::new(),
            credentials_message: None,
            feed_history: 500.0,
        }
    }
}

/// Global numbers the General tab reports.
pub struct Stats {
    pub uptime: String,
    pub downloaded: String,
    pub uploaded: String,
    pub live_peers: u32,
    pub down: String,
    pub up: String,
    pub feeds: usize,
    pub rules: usize,
    pub items: usize,
}

fn restart(ui: &mut Ui) {
    ui.add(
        Badge::new("Restart required")
            .color(BadgeColor::Warning)
            .size(BadgeSize::Small),
    );
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(rich(title, TypeRamp::Subtitle2));
    ui.add_space(4.0);
}

fn number(ui: &mut Ui, label: &str, hint: &str, value: &mut f64, suffix: &str) {
    Field::new(label).hint(hint).show(ui, |ui| {
        ui.add(
            SpinButton::new(value)
                .range(0.0..=1.0e12)
                .step(1.0)
                .suffix(suffix),
        );
    });
    ui.add_space(4.0);
}

fn note(ui: &mut Ui, label: &str, flag: &str) {
    ui.horizontal(|ui| {
        restart(ui);
        ui.label(format!("{label} — set with"));
        ui.label(RichText::new(flag).monospace());
    });
}

fn kv(ui: &mut Ui, id: &str, rows: &[(&str, String)]) {
    let muted = c(Theme::get(ui.ctx()).color.neutral_foreground_3);
    Grid::new(id)
        .num_columns(2)
        .spacing([32.0, 8.0])
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(RichText::new(*k).color(muted));
                ui.label(v);
                ui.end_row();
            }
        });
}

/// The body of the dialog. Returns nothing; Cancel / OK are dialog actions.
pub fn body(ui: &mut Ui, s: &mut Settings, stats: &Stats) {
    ui.add(
        SearchBox::new(&mut s.search)
            .placeholder("Search settings")
            .desired_width(ui.available_width()),
    );
    let query = s.search.trim().to_lowercase();
    if !query.is_empty() {
        let hits: Vec<_> = REGISTRY
            .iter()
            .filter(|(l, _, _)| l.to_lowercase().contains(&query))
            .collect();
        ui.add_space(6.0);
        if hits.is_empty() {
            ui.label(format!("No settings match \"{}\"", s.search.trim()));
        }
        for (label, tab, needs_restart) in hits {
            ui.horizontal(|ui| {
                if ui.link(*label).clicked() {
                    s.tab = *tab;
                    s.search.clear();
                }
                ui.label(RichText::new(TABS[*tab]).weak());
                if *needs_restart {
                    restart(ui);
                }
            });
        }
        ui.separator();
    }
    ui.add_space(6.0);
    let mut tabs = TabList::new("settings_tabs", &mut s.tab);
    for t in TABS {
        tabs = tabs.tab(t);
    }
    tabs.show(ui);
    ui.separator();
    ScrollArea::vertical()
        .id_salt("settings_body")
        .max_height(440.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_height(440.0);
            match s.tab {
                0 => general(ui, stats),
                1 => folders(ui, s),
                2 => speed(ui, s),
                3 => connections(ui, s),
                4 => queue(ui, s),
                5 => seeding(ui, s),
                6 => dht(ui),
                7 => security(ui, s),
                8 => http_api(ui),
                _ => rss(ui, s, stats),
            }
        });
}

fn general(ui: &mut Ui, st: &Stats) {
    section(ui, "About this session");
    kv(
        ui,
        "general",
        &[
            ("Version", "0.2.0-beta.2".to_owned()),
            ("Uptime", st.uptime.clone()),
            ("Total Downloaded", st.downloaded.clone()),
            ("Total Uploaded", st.uploaded.clone()),
            ("Live Peers", st.live_peers.to_string()),
            ("Download Speed", st.down.clone()),
            ("Upload Speed", st.up.clone()),
            ("Web Seeds", "Enabled".to_owned()),
        ],
    );
    ui.add_space(8.0);
    note(ui, "Web seeds", "--disable-webseeds");
}

fn folder_input(
    ui: &mut Ui,
    s: &mut Settings,
    which: usize,
    label: &str,
    hint: &str,
    placeholder: &str,
) {
    Field::new(label).hint(hint).show(ui, |ui| {
        ui.horizontal(|ui| {
            let value = if which == 0 {
                &mut s.download_folder
            } else {
                &mut s.completed_folder
            };
            ui.add(
                Input::new(value)
                    .placeholder(placeholder)
                    .desired_width(380.0)
                    .id_salt(("folder", which)),
            );
            if ui
                .add(Button::icon_only(IconName::FolderOpen, "Browse").id_salt(("browse", which)))
                .clicked()
            {
                s.browsing = Some(which);
            }
        });
    });
    if s.browsing == Some(which) {
        let theme = Theme::get(ui.ctx());
        egui::Frame::new()
            .fill(c(theme.color.neutral_background_2))
            .corner_radius(4.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("/srv").monospace());
                    if ui
                        .add(
                            Button::new("Select")
                                .appearance(Appearance::Primary)
                                .size(fluent2_egui::controls::ButtonSize::Small),
                        )
                        .clicked()
                    {
                        s.browsing = None;
                    }
                    if ui
                        .add(Button::new("Cancel").size(fluent2_egui::controls::ButtonSize::Small))
                        .clicked()
                    {
                        s.browsing = None;
                    }
                });
                for dir in ["..", "downloads", "completed", "media", "backups"] {
                    ui.horizontal(|ui| {
                        ui.add(Icon::new(IconName::Folder).size(fluent2_egui::IconSize::S16));
                        if ui.selectable_label(false, dir).clicked() && dir != ".." {
                            let target = format!("/srv/{dir}");
                            if which == 0 {
                                s.download_folder = target;
                            } else {
                                s.completed_folder = target;
                            }
                            s.browsing = None;
                        }
                    });
                }
            });
    }
}

fn folders(ui: &mut Ui, s: &mut Settings) {
    section(ui, "Download Location");
    folder_input(
        ui,
        s,
        0,
        "Default download folder",
        "New torrents will be saved to this folder by default",
        "/path/to/downloads",
    );
    section(ui, "Move on Completion");
    folder_input(
        ui,
        s,
        1,
        "Completed folder",
        "Finished torrents are moved here. Leave empty to disable",
        "/path/to/completed (optional)",
    );
    ui.add_space(10.0);
    note(ui, "Data directory", "--persistence-location");
}

fn limit_hint(v: f64) -> String {
    if v <= 0.0 {
        "0 = unlimited (currently disabled)".to_owned()
    } else {
        format!("current {} per second", crate::data::bytes(v as u64))
    }
}

fn speed(ui: &mut Ui, s: &mut Settings) {
    section(ui, "Speed Limits");
    let (d, u) = (limit_hint(s.down_limit), limit_hint(s.up_limit));
    number(
        ui,
        "Download rate limit (bytes/s)",
        &d,
        &mut s.down_limit,
        " B/s",
    );
    number(
        ui,
        "Upload rate limit (bytes/s)",
        &u,
        &mut s.up_limit,
        " B/s",
    );
    number(ui, "Peer limit", "current: default", &mut s.peer_limit, "");
    number(
        ui,
        "Concurrent init limit",
        "Torrents initialised at the same time",
        &mut s.init_limit,
        "",
    );
    section(ui, "Alternative Speed Limits");
    ui.add(Checkbox::bool(
        &mut s.alt_enabled,
        "Enable alternative speed limits",
    ));
    if s.alt_enabled {
        ui.indent("alt", |ui| {
            number(
                ui,
                "Alt download limit (KB/s)",
                "0 or empty = unlimited",
                &mut s.alt_down,
                " KB/s",
            );
            number(
                ui,
                "Alt upload limit (KB/s)",
                "0 or empty = unlimited",
                &mut s.alt_up,
                " KB/s",
            );
        });
    }
    ui.add(Checkbox::bool(&mut s.schedule, "Enable schedule"));
    if s.schedule {
        ui.indent("sched", |ui| {
            ui.horizontal(|ui| {
                Field::new("Start time").show(ui, |ui| {
                    ui.add(
                        Input::new(&mut s.schedule_from)
                            .desired_width(80.0)
                            .id_salt("from"),
                    )
                });
                Field::new("End time").show(ui, |ui| {
                    ui.add(
                        Input::new(&mut s.schedule_to)
                            .desired_width(80.0)
                            .id_salt("to"),
                    )
                });
            });
            ui.label("Days");
            ui.horizontal(|ui| {
                for (day, on) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                    .iter()
                    .zip(s.days.iter_mut())
                {
                    ui.add(Checkbox::bool(on, *day));
                }
            });
        });
    }
}

fn connections(ui: &mut Ui, s: &mut Settings) {
    ui.label("Connection statistics from the current session.");
    ui.add_space(6.0);
    Grid::new("conn_stats")
        .striped(true)
        .num_columns(5)
        .spacing([28.0, 6.0])
        .show(ui, |ui| {
            for h in ["Protocol", "Family", "Attempts", "Successes", "Errors"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for (p, f, a, ok, e) in [
                ("TCP", "IPv4", 18_204, 6_115, 12_089),
                ("TCP", "IPv6", 2_310, 901, 1_409),
                ("uTP", "IPv4", 9_877, 4_432, 5_445),
                ("uTP", "IPv6", 1_022, 388, 634),
            ] {
                ui.label(p);
                ui.label(f);
                ui.label(a.to_string());
                ui.label(ok.to_string());
                ui.label(e.to_string());
                ui.end_row();
            }
        });
    section(ui, "Encryption");
    ui.add_enabled(
        false,
        Dropdown::new(
            "encryption",
            &mut s.encryption,
            &[
                "Disabled",
                "Enabled (prefer encrypted)",
                "Forced (require encrypted)",
            ],
        )
        .width(280.0),
    );
    ui.label(
        RichText::new("Encrypted connections are preferred; plaintext peers are still accepted.")
            .weak(),
    );
    ui.add_space(10.0);
    note(ui, "Listen port", "--listen-port");
    note(ui, "Bind address (peer listener)", "--listen-ip");
    ui.label(RichText::new("These are set via CLI flags when the server starts.").weak());
}

fn queue(ui: &mut Ui, s: &mut Settings) {
    section(ui, "Queue Limits");
    number(
        ui,
        "Max active downloads",
        "0 or empty = unlimited",
        &mut s.max_downloads,
        "",
    );
    number(
        ui,
        "Max active uploads",
        "0 or empty = unlimited",
        &mut s.max_uploads,
        "",
    );
    number(
        ui,
        "Max active total",
        "0 or empty = unlimited",
        &mut s.max_total,
        "",
    );
}

fn seeding(ui: &mut Ui, s: &mut Settings) {
    section(ui, "Seed Ratio Limit");
    ui.add(Checkbox::bool(
        &mut s.ratio_enabled,
        "Limit seed ratio globally",
    ));
    if s.ratio_enabled {
        Field::new("Ratio limit")
            .hint("e.g. 2.0 means upload 2x the torrent's size, then stop")
            .show(ui, |ui| {
                ui.add(
                    SpinButton::new(&mut s.ratio)
                        .range(0.0..=100.0)
                        .step(0.1)
                        .decimals(1),
                );
            });
    }
    section(ui, "Seed Time Limit");
    ui.add(Checkbox::bool(
        &mut s.time_enabled,
        "Limit seeding time globally",
    ));
    if s.time_enabled {
        Field::new("Time limit (hours)").show(ui, |ui| {
            ui.add(
                SpinButton::new(&mut s.hours)
                    .range(0.0..=10_000.0)
                    .step(1.0)
                    .suffix(" h"),
            );
        });
    }
    ui.add_space(10.0);
    MessageBar::new(
        Intent::Info,
        "Super-seeding can be enabled per-torrent from the context menu.",
    )
    .show(ui);
}

fn dht(ui: &mut Ui) {
    section(ui, "DHT");
    kv(
        ui,
        "dht",
        &[
            ("Id", "4f1c…a93e".to_owned()),
            ("Outstanding Requests", "12".to_owned()),
            ("Routing Table Size", "312".to_owned()),
            ("Routing Table Size V6", "88".to_owned()),
            ("Bootstrapped", "true".to_owned()),
            ("Seen Peers", "14,208".to_owned()),
        ],
    );
    ui.add_space(8.0);
    note(ui, "DHT enabled", "--disable-dht");
}

fn security(ui: &mut Ui, s: &mut Settings) {
    section(ui, "Change credentials");
    if let Some((ok, text)) = s.credentials_message {
        MessageBar::new(if ok { Intent::Success } else { Intent::Error }, text).show(ui);
        ui.add_space(6.0);
    }
    Field::new("Current Password")
        .required(true)
        .show(ui, |ui| {
            ui.add(
                Input::new(&mut s.current_password)
                    .password()
                    .placeholder("Enter current password")
                    .desired_width(320.0)
                    .id_salt("cur"),
            );
        });
    ui.separator();
    Field::new("New Username").show(ui, |ui| {
        ui.add(
            Input::new(&mut s.new_username)
                .placeholder("Leave blank to keep current")
                .desired_width(320.0)
                .id_salt("nu"),
        );
    });
    Field::new("New Password").show(ui, |ui| {
        ui.add(
            Input::new(&mut s.new_password)
                .password()
                .placeholder("Leave blank to keep current")
                .desired_width(320.0)
                .id_salt("np"),
        );
    });
    let typed = !s.new_password.is_empty();
    ui.add_enabled_ui(typed, |ui| {
        Field::new("Confirm New Password").show(ui, |ui| {
            ui.add(
                Input::new(&mut s.confirm_password)
                    .password()
                    .desired_width(320.0)
                    .id_salt("cp"),
            );
        });
    });
    ui.add_space(6.0);
    if ui
        .add(Button::new("Update Credentials").appearance(Appearance::Primary))
        .clicked()
    {
        s.credentials_message = Some(if s.current_password.is_empty() {
            (false, "Enter your current password.")
        } else if s.new_password != s.confirm_password {
            (false, "The new passwords do not match.")
        } else {
            (true, "Credentials updated.")
        });
    }
}

fn http_api(ui: &mut Ui) {
    section(ui, "HTTP API");
    kv(
        ui,
        "http",
        &[
            ("API URL", "http://127.0.0.1:3030".to_owned()),
            ("Swagger UI", "http://127.0.0.1:3030/swagger/".to_owned()),
        ],
    );
    ui.add_space(8.0);
    note(ui, "HTTP API bind address", "--http-api-listen-addr");
}

fn rss(ui: &mut Ui, s: &mut Settings, st: &Stats) {
    section(ui, "RSS STATUS");
    ui.horizontal(|ui| {
        for (label, n) in [
            ("Feeds", st.feeds),
            ("Rules", st.rules),
            ("Items", st.items),
        ] {
            crate::widgets::card(ui, |ui| {
                ui.set_min_width(120.0);
                ui.vertical(|ui| {
                    ui.label(rich(n.to_string(), TypeRamp::Title2));
                    ui.label(RichText::new(label).weak());
                });
            });
        }
    });
    section(ui, "SETTINGS");
    number(
        ui,
        "Feed History Limit",
        "Items kept per feed. Empty = unlimited. Default: 500.",
        &mut s.feed_history,
        "",
    );
}

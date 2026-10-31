//! Settings, five tabs: General, Connection, Speed, Folders, Seeding.
//!
//! Bodies match the daemon handlers the web settings dialog already calls.
//! Listen address and port, UPnP, DHT and local discovery are saved for the
//! next start (`POST /session/network`) and never change the running process,
//! so the Connection tab marks them restart-required and shows the saved values
//! next to the running ones. Peer exchange has no switch and stays read-only.
//! A proxy password is never drawn from a log; the field starts empty and a
//! save sends it only when the user typed one.

use fluent2_egui::controls::{Appearance, Badge, BadgeColor, Button, Checkbox, Input};
use fluent2_egui::surfaces::{Intent, MessageBar};
use rtbit_client::{
    AdvancedConfig, AltSpeedConfig, AltSpeedSchedule, DownloadsConfig, FoldersConfig,
    IpFilterConfig, NetworkConfig, NetworkSettings, ProxyConfig, RateLimits, SavedNetworkStatus,
    SeedLimits, SettingsSnapshot,
};

pub use rtbit_ui_core::SettingsChanges;

/// What the settings screen asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    /// Post every edited section.
    Save,
    /// Forget the saved network values, so the next start uses the startup flags.
    UseStartupFlags,
}

/// Which of the five tabs is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Connection,
    Speed,
    Folders,
    Seeding,
}

impl SettingsTab {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Connection,
        Self::Speed,
        Self::Folders,
        Self::Seeding,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Connection => "Connection",
            Self::Speed => "Speed",
            Self::Folders => "Folders",
            Self::Seeding => "Seeding",
        }
    }
}

/// Editable copy of the snapshot. Strings so a half-typed number is not lost.
/// Debug masks a password typed into the proxy field.
#[derive(Clone, Default, PartialEq)]
pub struct SettingsDraft {
    pub incomplete_suffix: bool,
    pub connect_timeout: String,
    pub read_write_timeout: String,
    pub keep_alive: String,
    pub listen_ip: String,
    pub listen_port: String,
    pub dht: bool,
    pub pex: bool,
    pub lsd: bool,
    pub upnp: bool,
    pub http_listen: String,
    /// Shown under the network form after a network save. Set by the app.
    pub network_notice: String,
    /// Masked URL from the daemon. Not posted back: it may contain `***`.
    pub proxy_shown: String,
    /// A new proxy URL. Empty means "leave the saved proxy alone".
    pub proxy_edit: String,
    pub clear_proxy: bool,
    pub ip_filter: String,
    pub download_folder: String,
    pub completed_folder: String,
    pub download_bps: String,
    pub upload_bps: String,
    pub peer_limit: String,
    pub init_limit: String,
    pub alt_enabled: bool,
    pub alt_down: String,
    pub alt_up: String,
    pub schedule_enabled: bool,
    pub schedule_start: String,
    pub schedule_end: String,
    pub schedule_days: String,
    pub ratio_limit: String,
    pub seed_minutes: String,
    /// Loaded values, so a save posts only a section the user changed.
    baseline: Option<SettingsSnapshot>,
}

impl std::fmt::Debug for SettingsDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsDraft")
            .field("incomplete_suffix", &self.incomplete_suffix)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_write_timeout", &self.read_write_timeout)
            .field("keep_alive", &self.keep_alive)
            .field("listen_ip", &self.listen_ip)
            .field("listen_port", &self.listen_port)
            .field("dht", &self.dht)
            .field("pex", &self.pex)
            .field("lsd", &self.lsd)
            .field("upnp", &self.upnp)
            .field("http_listen", &self.http_listen)
            .field("network_notice", &self.network_notice)
            .field("proxy_shown", &mask_proxy(&self.proxy_shown))
            .field("proxy_edit", &mask_proxy(&self.proxy_edit))
            .field("clear_proxy", &self.clear_proxy)
            .field("ip_filter", &self.ip_filter)
            .field("download_folder", &self.download_folder)
            .field("completed_folder", &self.completed_folder)
            .field("download_bps", &self.download_bps)
            .field("upload_bps", &self.upload_bps)
            .field("peer_limit", &self.peer_limit)
            .field("init_limit", &self.init_limit)
            .field("alt_enabled", &self.alt_enabled)
            .field("alt_down", &self.alt_down)
            .field("alt_up", &self.alt_up)
            .field("schedule_enabled", &self.schedule_enabled)
            .field("schedule_start", &self.schedule_start)
            .field("schedule_end", &self.schedule_end)
            .field("schedule_days", &self.schedule_days)
            .field("ratio_limit", &self.ratio_limit)
            .field("seed_minutes", &self.seed_minutes)
            .finish()
    }
}

/// `scheme://user:secret@host` becomes `scheme://user:***@host`.
fn mask_proxy(url: &str) -> String {
    let Some(scheme) = url.find("://") else {
        return url.to_owned();
    };
    let after = scheme + 3;
    let Some(at) = url[after..].find('@') else {
        return url.to_owned();
    };
    let userinfo = &url[after..after + at];
    let Some((user, secret)) = userinfo.split_once(':') else {
        return url.to_owned();
    };
    if secret.is_empty() {
        return url.to_owned();
    }
    format!("{}{user}:***{}", &url[..after], &url[after + at..])
}

impl SettingsDraft {
    /// Fill the fields from a daemon read. A blank number means "engine default" or "off".
    pub fn from_snapshot(snap: &SettingsSnapshot) -> Self {
        let net = NetworkStart::of(&snap.network);
        Self {
            incomplete_suffix: snap.downloads.incomplete_file_suffix,
            connect_timeout: opt_num(snap.advanced.connect_timeout_secs),
            read_write_timeout: opt_num(snap.advanced.read_write_timeout_secs),
            keep_alive: opt_num(snap.advanced.keep_alive_secs),
            listen_ip: net.listen_ip,
            listen_port: net.listen_port,
            dht: net.dht,
            pex: snap.network.pex_enabled,
            lsd: net.lsd,
            upnp: net.upnp,
            http_listen: snap.network.http_api_listen.clone(),
            proxy_shown: snap.proxy.proxy_url.clone().unwrap_or_default(),
            ip_filter: snap.ip_filter.filter.clone(),
            download_folder: snap.folders.download_folder.clone(),
            completed_folder: snap.folders.completed_folder.clone().unwrap_or_default(),
            download_bps: opt_num(snap.limits.download_bps),
            upload_bps: opt_num(snap.limits.upload_bps),
            peer_limit: opt_num(snap.limits.peer_limit),
            init_limit: opt_num(snap.limits.concurrent_init_limit),
            alt_enabled: snap.alt.enabled,
            alt_down: opt_num(snap.alt.config.download_rate),
            alt_up: opt_num(snap.alt.config.upload_rate),
            schedule_enabled: snap.schedule.enabled,
            schedule_start: snap.schedule.start_minutes.to_string(),
            schedule_end: snap.schedule.end_minutes.to_string(),
            schedule_days: snap.schedule.days.to_string(),
            ratio_limit: snap
                .seed_limits
                .ratio_limit
                .map(|n| n.to_string())
                .unwrap_or_default(),
            seed_minutes: snap
                .seed_limits
                .time_limit_secs
                .map(|secs| (secs / 60).to_string())
                .unwrap_or_default(),
            baseline: Some(snap.clone()),
            ..Self::default()
        }
    }

    /// Posts implied by edits since the last load. Empty when nothing changed.
    pub fn changes(&self) -> Result<SettingsChanges, String> {
        let base = self
            .baseline
            .as_ref()
            .ok_or_else(|| "Settings have not loaded yet.".to_owned())?;
        let mut out = SettingsChanges::default();

        let downloads = DownloadsConfig {
            incomplete_file_suffix: self.incomplete_suffix,
        };
        if downloads != base.downloads {
            out.downloads = Some(downloads);
        }

        let advanced = AdvancedConfig {
            connect_timeout_secs: parse_opt(&self.connect_timeout, "Connect timeout")?,
            read_write_timeout_secs: parse_opt(&self.read_write_timeout, "Read/write timeout")?,
            keep_alive_secs: parse_opt(&self.keep_alive, "Keep-alive")?,
        };
        if advanced != base.advanced {
            out.advanced = Some(advanced);
        }

        if base.network.can_save {
            let shown = NetworkStart::of(&base.network);
            let edited = NetworkStart {
                listen_ip: self.listen_ip.clone(),
                listen_port: self.listen_port.clone(),
                upnp: self.upnp,
                dht: self.dht,
                lsd: self.lsd,
            };
            if edited != shown {
                out.network = Some(network_body(&edited, &base.network)?);
            }
        }

        if self.clear_proxy {
            out.proxy = Some(ProxyConfig { proxy_url: None });
        } else if !self.proxy_edit.trim().is_empty() {
            out.proxy = Some(ProxyConfig {
                proxy_url: Some(self.proxy_edit.trim().to_owned()),
            });
        }

        let filter = IpFilterConfig {
            filter: self.ip_filter.clone(),
        };
        if filter != base.ip_filter {
            out.ip_filter = Some(filter);
        }

        let completed = {
            let trimmed = self.completed_folder.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        };
        let folders = FoldersConfig {
            download_folder: self.download_folder.trim().to_owned(),
            completed_folder: completed,
        };
        if folders.download_folder.is_empty() {
            return Err("Download folder is required.".into());
        }
        if folders != base.folders {
            out.folders = Some(folders);
        }

        let limits = RateLimits {
            download_bps: parse_opt(&self.download_bps, "Download limit")?,
            upload_bps: parse_opt(&self.upload_bps, "Upload limit")?,
            peer_limit: parse_opt(&self.peer_limit, "Peer limit")?,
            concurrent_init_limit: parse_opt(&self.init_limit, "Connection limit")?,
        };
        if limits != base.limits {
            out.limits = Some(limits);
        }

        if self.alt_enabled != base.alt.enabled {
            out.alt_enabled = Some(self.alt_enabled);
        }
        let alt = AltSpeedConfig {
            download_rate: parse_opt(&self.alt_down, "Alternative download")?,
            upload_rate: parse_opt(&self.alt_up, "Alternative upload")?,
        };
        if alt != base.alt.config {
            out.alt_config = Some(alt);
        }
        let schedule = AltSpeedSchedule {
            enabled: self.schedule_enabled,
            start_minutes: parse_req(&self.schedule_start, "Schedule start")?,
            end_minutes: parse_req(&self.schedule_end, "Schedule end")?,
            days: parse_req(&self.schedule_days, "Schedule days")?,
        };
        if schedule != base.schedule {
            out.schedule = Some(schedule);
        }

        let seed = SeedLimits {
            ratio_limit: parse_opt_f64(&self.ratio_limit)?,
            time_limit_secs: parse_opt::<u64>(&self.seed_minutes, "Seed time")?.map(|m| m * 60),
        };
        if seed != base.seed_limits {
            out.seed_limits = Some(seed);
        }
        Ok(out)
    }
}

/// The network form's fields as they start: a saved value, else what is running.
#[derive(Clone, Debug, PartialEq, Eq)]
struct NetworkStart {
    listen_ip: String,
    listen_port: String,
    upnp: bool,
    dht: bool,
    lsd: bool,
}

impl NetworkStart {
    fn of(net: &NetworkConfig) -> Self {
        let pending = net.pending.clone().unwrap_or_default();
        Self {
            listen_ip: pending
                .listen_ip
                .or_else(|| net.listen_ip.clone())
                .unwrap_or_default(),
            listen_port: pending
                .listen_port
                .or(net.listen_port)
                .map(|p| p.to_string())
                .unwrap_or_default(),
            upnp: pending.upnp_port_forward.unwrap_or(net.upnp_port_forward),
            dht: pending.dht_enabled.unwrap_or(net.dht_enabled),
            lsd: pending.lsd_enabled.unwrap_or(net.lsd_enabled),
        }
    }
}

/// The POST body for an edited network form. An empty IP clears a saved one,
/// so the startup flag applies again; the other fields are sent with it.
fn network_body(edited: &NetworkStart, net: &NetworkConfig) -> Result<NetworkSettings, String> {
    if let Some(err) = listen_ip_error(&edited.listen_ip) {
        return Err(err.to_owned());
    }
    let port = parse_port(&edited.listen_port).ok_or_else(|| PORT_ERROR.to_owned())?;
    let ip = edited.listen_ip.trim();
    let saved_ip = net.pending.as_ref().and_then(|p| p.listen_ip.as_ref());
    Ok(NetworkSettings {
        listen_ip: (!ip.is_empty()).then(|| ip.to_owned()),
        listen_port: Some(port),
        dht_enabled: Some(edited.dht),
        lsd_enabled: Some(edited.lsd),
        upnp_port_forward: Some(edited.upnp),
        reset: (ip.is_empty() && saved_ip.is_some()).then_some(true),
    })
}

const PORT_ERROR: &str = "Enter a port from 1 to 65535.";

fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|p| *p >= 1)
}

/// A blocking problem with the typed listen IP. Empty keeps the startup flag.
pub fn listen_ip_error(ip: &str) -> Option<&'static str> {
    let value = ip.trim();
    if value.is_empty() {
        return None;
    }
    if value.contains('%') {
        return Some(
            "IPv6 zones (%interface) are not supported. Leave the zone off, or pick the interface with --bind-device.",
        );
    }
    let bare = value.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<std::net::IpAddr>() {
        Ok(_) => None,
        Err(_) => Some("Enter a valid IPv4 or IPv6 address."),
    }
}

/// Legal values that are often a mistake. Shown, never blocking.
pub fn network_warnings(ip: &str, port: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if parse_port(port).is_some_and(|p| p < 1024) {
        out.push(
            "Ports below 1024 need root or CAP_NET_BIND_SERVICE. If the daemon cannot bind it, it falls back to the startup flags.",
        );
    }
    match ip.trim().trim_start_matches('[').trim_end_matches(']') {
        "::" => out.push(":: listens for peers on every interface, IPv4 and IPv6."),
        "0.0.0.0" => {
            out.push("0.0.0.0 listens for peers on every IPv4 interface and none on IPv6.")
        }
        _ => {}
    }
    out
}

/// One saved value next to what the process runs with now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedRow {
    pub label: &'static str,
    pub running: String,
    pub saved: String,
}

impl SavedRow {
    pub fn differs(&self) -> bool {
        self.running != self.saved
    }
}

/// A row for each saved value. Empty when nothing is saved.
pub fn saved_vs_running(net: &NetworkConfig) -> Vec<SavedRow> {
    let Some(p) = &net.pending else {
        return Vec::new();
    };
    let on_off = |v: bool| if v { "On" } else { "Off" }.to_owned();
    let mut rows = Vec::new();
    if let Some(ip) = &p.listen_ip {
        rows.push(SavedRow {
            label: "Listen IP",
            running: net.listen_ip.clone().unwrap_or_else(|| "off".into()),
            saved: ip.clone(),
        });
    }
    if let Some(port) = p.listen_port {
        rows.push(SavedRow {
            label: "Listen port",
            running: net
                .listen_port
                .map_or_else(|| "off".into(), |p| p.to_string()),
            saved: port.to_string(),
        });
    }
    for (label, running, saved) in [
        (
            "UPnP port forward",
            net.upnp_port_forward,
            p.upnp_port_forward,
        ),
        ("DHT", net.dht_enabled, p.dht_enabled),
        ("Local discovery", net.lsd_enabled, p.lsd_enabled),
    ] {
        if let Some(saved) = saved {
            rows.push(SavedRow {
                label,
                running: on_off(running),
                saved: on_off(saved),
            });
        }
    }
    rows
}

fn opt_num<T: ToString>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn parse_opt<T: std::str::FromStr>(text: &str, label: &str) -> Result<Option<T>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse()
        .map(Some)
        .map_err(|_| format!("{label} must be a whole number."))
}

fn parse_req<T: std::str::FromStr>(text: &str, label: &str) -> Result<T, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("{label} must be a whole number."))
}

fn parse_opt_f64(text: &str) -> Result<Option<f64>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse()
        .map(Some)
        .map_err(|_| "Ratio limit must be a number.".to_owned())
}

/// Draw the open tab and say which button was clicked.
pub fn draw_settings(
    ui: &mut egui::Ui,
    tab: &mut SettingsTab,
    draft: &mut SettingsDraft,
    busy: bool,
    error: &str,
) -> SettingsAction {
    let mut action = SettingsAction::None;
    ui.heading("Settings");
    ui.horizontal(|ui| {
        for candidate in SettingsTab::ALL {
            let selected = *tab == candidate;
            let appearance = if selected {
                Appearance::Primary
            } else {
                Appearance::Secondary
            };
            if ui
                .add_enabled(!busy, Button::new(candidate.label()).appearance(appearance))
                .clicked()
            {
                *tab = candidate;
            }
        }
    });
    ui.add_space(8.0);
    match *tab {
        SettingsTab::General => general(ui, draft),
        SettingsTab::Connection => {
            if connection(ui, draft, busy) {
                action = SettingsAction::UseStartupFlags;
            }
        }
        SettingsTab::Speed => speed(ui, draft),
        SettingsTab::Folders => folders(ui, draft),
        SettingsTab::Seeding => seeding(ui, draft),
    }
    ui.add_space(12.0);
    if ui
        .add_enabled(!busy, Button::new("Save").appearance(Appearance::Primary))
        .clicked()
    {
        action = SettingsAction::Save;
    }
    if !error.is_empty() {
        ui.add_space(8.0);
        ui.colored_label(ERROR_RED, error);
    }
    action
}

const ERROR_RED: egui::Color32 = egui::Color32::from_rgb(196, 43, 28);
const WARNING_AMBER: egui::Color32 = egui::Color32::from_rgb(188, 75, 9);

fn general(ui: &mut egui::Ui, draft: &mut SettingsDraft) {
    ui.add(Checkbox::bool(
        &mut draft.incomplete_suffix,
        "Append .part to incomplete files",
    ));
    ui.add_space(8.0);
    ui.label("Peer timeouts, in seconds. Blank uses the engine default. The daemon refuses 0 and values over its cap.");
    number(ui, "Connect timeout", &mut draft.connect_timeout);
    number(ui, "Read/write timeout", &mut draft.read_write_timeout);
    number(ui, "Keep-alive", &mut draft.keep_alive);
}

/// Returns true when "Use startup flags" was clicked.
fn connection(ui: &mut egui::Ui, draft: &mut SettingsDraft, busy: bool) -> bool {
    let reset = network(ui, draft, busy);
    ui.add_enabled(false, Checkbox::bool(&mut draft.pex, "Peer exchange"));
    ui.label("Peer exchange is always on.");
    if !draft.http_listen.is_empty() {
        ui.label(format!("HTTP listen {}", draft.http_listen));
    }
    ui.add_space(8.0);
    if !draft.proxy_shown.is_empty() {
        ui.label(format!("Saved proxy {}", draft.proxy_shown));
    }
    ui.add(
        Input::new(&mut draft.proxy_edit)
            .label("Proxy URL")
            .id_salt("proxy_url"),
    );
    ui.add(Checkbox::bool(&mut draft.clear_proxy, "Clear proxy"));
    ui.add_space(8.0);
    ui.label("IP filter, one range per line.");
    ui.add(
        Input::new(&mut draft.ip_filter)
            .label("IP filter")
            .id_salt("ip_filter"),
    );
    reset
}

/// The startup-only network form. Returns true when "Use startup flags" was clicked.
fn network(ui: &mut egui::Ui, draft: &mut SettingsDraft, busy: bool) -> bool {
    let Some(net) = draft.baseline.as_ref().map(|b| b.network.clone()) else {
        return false;
    };
    ui.horizontal(|ui| {
        ui.strong("Peer listener and discovery");
        ui.add(Badge::new("Restart required").color(BadgeColor::Warning));
    });
    if !net.can_save {
        ui.label(format!(
            "Listen {} {}",
            net.listen_ip.as_deref().unwrap_or("any"),
            net.listen_port.map(|p| p.to_string()).unwrap_or_default()
        ));
        let (mut upnp, mut dht, mut lsd) =
            (net.upnp_port_forward, net.dht_enabled, net.lsd_enabled);
        ui.add_enabled(false, Checkbox::bool(&mut upnp, "UPnP port forward"));
        ui.add_enabled(false, Checkbox::bool(&mut dht, "DHT"));
        ui.add_enabled(false, Checkbox::bool(&mut lsd, "Local discovery"));
        ui.label(
            "Session persistence is off, so these settings cannot be saved here. Set them with the startup flags.",
        );
        ui.add_space(8.0);
        return false;
    }
    match net.pending_status {
        Some(SavedNetworkStatus::FellBack) => {
            MessageBar::new(
                Intent::Error,
                "The saved peer listener settings could not be used at the last start, so the startup flags are running. Fix the values below, or use the startup flags.",
            )
            .id_salt("network_fell_back")
            .show(ui);
        }
        Some(SavedNetworkStatus::Ignored) => {
            MessageBar::new(
                Intent::Info,
                "Started with --ignore-saved-network: the saved values below are not in use.",
            )
            .id_salt("network_ignored")
            .show(ui);
        }
        _ => {}
    }
    let rows = saved_vs_running(&net);
    if !rows.is_empty() {
        egui::Grid::new("network_saved_vs_running")
            .num_columns(3)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.weak("Setting");
                ui.weak("Running now");
                ui.weak("Saved");
                ui.end_row();
                for row in &rows {
                    ui.label(row.label);
                    ui.label(&row.running);
                    if row.differs() {
                        ui.strong(format!("{} (next start)", row.saved));
                    } else {
                        ui.label(&row.saved);
                    }
                    ui.end_row();
                }
            });
    }
    ui.add(
        Input::new(&mut draft.listen_ip)
            .label("Listen IP")
            .placeholder("::")
            .id_salt("listen_ip"),
    );
    ui.label("IPv4 or IPv6 address for incoming peers. :: listens on every interface. Blank uses the startup flag.");
    ui.add(
        Input::new(&mut draft.listen_port)
            .label("Listen port")
            .id_salt("listen_port"),
    );
    ui.add(Checkbox::bool(&mut draft.upnp, "UPnP port forward"));
    ui.add(Checkbox::bool(&mut draft.dht, "DHT"));
    ui.add(Checkbox::bool(&mut draft.lsd, "Local discovery"));
    if let Some(err) = listen_ip_error(&draft.listen_ip) {
        ui.colored_label(ERROR_RED, err);
    }
    if parse_port(&draft.listen_port).is_none() {
        ui.colored_label(ERROR_RED, PORT_ERROR);
    }
    for warning in network_warnings(&draft.listen_ip, &draft.listen_port) {
        ui.colored_label(WARNING_AMBER, warning);
    }
    let mut reset = false;
    if net.pending.is_some()
        && ui
            .add_enabled(!busy, Button::new("Use startup flags"))
            .clicked()
    {
        reset = true;
    }
    if !draft.network_notice.is_empty() {
        ui.label(&draft.network_notice);
    }
    ui.add_space(8.0);
    reset
}

fn speed(ui: &mut egui::Ui, draft: &mut SettingsDraft) {
    ui.label("Blank is unlimited. Rates are bytes per second.");
    number(ui, "Download limit", &mut draft.download_bps);
    number(ui, "Upload limit", &mut draft.upload_bps);
    number(ui, "Peer limit", &mut draft.peer_limit);
    number(ui, "Connection limit", &mut draft.init_limit);
    ui.add_space(8.0);
    ui.add(Checkbox::bool(
        &mut draft.alt_enabled,
        "Alternative speed limits",
    ));
    number(ui, "Alternative download", &mut draft.alt_down);
    number(ui, "Alternative upload", &mut draft.alt_up);
    ui.add(Checkbox::bool(
        &mut draft.schedule_enabled,
        "Schedule alternative limits",
    ));
    number(ui, "Schedule start", &mut draft.schedule_start);
    number(ui, "Schedule end", &mut draft.schedule_end);
    number(ui, "Schedule days", &mut draft.schedule_days);
}

fn folders(ui: &mut egui::Ui, draft: &mut SettingsDraft) {
    ui.add(
        Input::new(&mut draft.download_folder)
            .label("Download folder")
            .id_salt("download_folder"),
    );
    ui.add_space(8.0);
    ui.add(
        Input::new(&mut draft.completed_folder)
            .label("Completed folder")
            .id_salt("completed_folder"),
    );
}

fn seeding(ui: &mut egui::Ui, draft: &mut SettingsDraft) {
    ui.label("Blank turns the limit off. Seed time is minutes.");
    ui.add(
        Input::new(&mut draft.ratio_limit)
            .label("Ratio limit")
            .id_salt("ratio_limit"),
    );
    number(ui, "Seed time", &mut draft.seed_minutes);
}

fn number(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.add_space(4.0);
    ui.add(Input::new(value).label(label));
}

#[cfg(test)]
mod draft_debug {
    use super::SettingsDraft;

    #[test]
    fn debug_masks_the_proxy_password() {
        let draft = SettingsDraft {
            proxy_edit: "socks5://ada:s3cret-pass@127.0.0.1:1080".into(),
            proxy_shown: "socks5://ada:***@127.0.0.1:1080".into(),
            ..SettingsDraft::default()
        };
        let shown = format!("{draft:?}");
        assert!(shown.contains("socks5://ada:***@127.0.0.1:1080"));
        assert!(!shown.contains("s3cret-pass"));
    }
}

#[cfg(test)]
mod network_rules {
    use super::{listen_ip_error, network_warnings, saved_vs_running};
    use rtbit_client::{NetworkConfig, NetworkSettings};

    #[test]
    fn listen_ip_accepts_blank_v4_v6_and_brackets() {
        for ok in ["", "  ", "10.0.0.1", "::", "[::1]", "2001:db8::5"] {
            assert_eq!(listen_ip_error(ok), None, "{ok}");
        }
        assert!(listen_ip_error("fe80::1%eth0").unwrap().contains("zones"));
        assert!(listen_ip_error("300.1.1.1").unwrap().contains("valid"));
        assert!(listen_ip_error("host.lan").unwrap().contains("valid"));
    }

    #[test]
    fn warnings_cover_privileged_ports_and_all_interfaces() {
        assert!(network_warnings("", "6881").is_empty());
        assert_eq!(network_warnings("", "443").len(), 1);
        assert!(network_warnings("0.0.0.0", "6881")[0].contains("none on IPv6"));
        assert!(network_warnings("[::]", "80").len() == 2);
    }

    #[test]
    fn rows_list_only_saved_values() {
        let mut net = NetworkConfig {
            listen_port: Some(6881),
            dht_enabled: true,
            ..NetworkConfig::default()
        };
        assert!(saved_vs_running(&net).is_empty());
        net.pending = Some(NetworkSettings {
            listen_port: Some(6881),
            dht_enabled: Some(false),
            ..NetworkSettings::default()
        });
        let rows = saved_vs_running(&net);
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].differs());
        assert_eq!(
            (rows[1].running.as_str(), rows[1].saved.as_str()),
            ("On", "Off")
        );
        assert!(rows[1].differs());
    }
}

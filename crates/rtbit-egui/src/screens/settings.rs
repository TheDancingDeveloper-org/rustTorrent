//! Settings, five tabs: General, Connection, Speed, Folders, Seeding.
//!
//! Bodies match the daemon handlers the web settings dialog already calls.
//! Connection listen, DHT, peer exchange, discovery and port forwarding are
//! fixed at startup, so this screen shows them and does not post them.
//! A proxy password is never drawn from a log; the field starts empty and a
//! save sends it only when the user typed one.

use fluent2_egui::controls::{Appearance, Button, Checkbox, Input};
use rtbit_client::{
    AdvancedConfig, AltSpeedConfig, AltSpeedSchedule, DownloadsConfig, FoldersConfig,
    IpFilterConfig, ProxyConfig, RateLimits, SeedLimits, SettingsSnapshot,
};

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
        Self {
            incomplete_suffix: snap.downloads.incomplete_file_suffix,
            connect_timeout: opt_num(snap.advanced.connect_timeout_secs),
            read_write_timeout: opt_num(snap.advanced.read_write_timeout_secs),
            keep_alive: opt_num(snap.advanced.keep_alive_secs),
            listen_ip: snap.network.listen_ip.clone().unwrap_or_default(),
            listen_port: snap
                .network
                .listen_port
                .map(|p| p.to_string())
                .unwrap_or_default(),
            dht: snap.network.dht_enabled,
            pex: snap.network.pex_enabled,
            lsd: snap.network.lsd_enabled,
            upnp: snap.network.upnp_port_forward,
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

/// One section to post. `None` means that section was not edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsChanges {
    pub downloads: Option<DownloadsConfig>,
    pub advanced: Option<AdvancedConfig>,
    pub proxy: Option<ProxyConfig>,
    pub ip_filter: Option<IpFilterConfig>,
    pub folders: Option<FoldersConfig>,
    pub limits: Option<RateLimits>,
    pub alt_enabled: Option<bool>,
    pub alt_config: Option<AltSpeedConfig>,
    pub schedule: Option<AltSpeedSchedule>,
    pub seed_limits: Option<SeedLimits>,
}

impl SettingsChanges {
    pub fn is_empty(&self) -> bool {
        self.downloads.is_none()
            && self.advanced.is_none()
            && self.proxy.is_none()
            && self.ip_filter.is_none()
            && self.folders.is_none()
            && self.limits.is_none()
            && self.alt_enabled.is_none()
            && self.alt_config.is_none()
            && self.schedule.is_none()
            && self.seed_limits.is_none()
    }
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

/// Draw the open tab. Returns true when Save was clicked.
pub fn draw_settings(
    ui: &mut egui::Ui,
    tab: &mut SettingsTab,
    draft: &mut SettingsDraft,
    busy: bool,
    error: &str,
) -> bool {
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
        SettingsTab::Connection => connection(ui, draft),
        SettingsTab::Speed => speed(ui, draft),
        SettingsTab::Folders => folders(ui, draft),
        SettingsTab::Seeding => seeding(ui, draft),
    }
    ui.add_space(12.0);
    let save = ui
        .add_enabled(!busy, Button::new("Save").appearance(Appearance::Primary))
        .clicked();
    if !error.is_empty() {
        ui.add_space(8.0);
        ui.colored_label(egui::Color32::from_rgb(196, 43, 28), error);
    }
    save
}

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

fn connection(ui: &mut egui::Ui, draft: &mut SettingsDraft) {
    ui.label("Listen address, DHT, peer exchange, local discovery and port forwarding are fixed at startup.");
    ui.label(format!(
        "Listen {} {}",
        if draft.listen_ip.is_empty() {
            "any".to_owned()
        } else {
            draft.listen_ip.clone()
        },
        draft.listen_port
    ));
    ui.add_enabled(false, Checkbox::bool(&mut draft.dht, "DHT"));
    ui.add_enabled(false, Checkbox::bool(&mut draft.pex, "Peer exchange"));
    ui.add_enabled(false, Checkbox::bool(&mut draft.lsd, "Local discovery"));
    ui.add_enabled(false, Checkbox::bool(&mut draft.upnp, "Port forwarding"));
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

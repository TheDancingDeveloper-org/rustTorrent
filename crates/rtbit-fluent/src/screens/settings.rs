//! Settings screen: six tabs (General, Connection, Speed, Folders, Seeding,
//! Advanced) behind a `SelectableList` nav, bound to a [`SettingsDraft`] that mirrors
//! the daemon's `SettingsSnapshot`.
//!
//! Load and save run on the bridge's tokio runtime; the UI thread only sets a
//! flag and awaits the reply in a gpui task. [`SettingsDraft::changes`] builds
//! the [`SettingsChanges`] that carries only the sections the user edited.
//! Listen address, port and the DHT/LSD/UPnP switches are saved for the next
//! start and never change the running process; the screen says so after a save.
//! The proxy section, read-only peer exchange and "Use startup flags" reset
//! follow the settings behaviour of the previous client.

use std::collections::BTreeMap;
use std::str::FromStr;

use fluent_layout::{MessageBar, MessageIntent, SelectableList, SelectableListItem};
use fluent_primitives::{
    Button, ButtonAppearance, Field, FieldValidationState, Label, Switch, TextInput,
};
use gpui::{
    App, AsyncApp, ClickEvent, Context, Entity, IntoElement, Render, Task, WeakEntity, Window, div,
    prelude::*, px,
};
use rtbit_client::{
    AdvancedConfig, AltSpeedConfig, AltSpeedSchedule, CONCURRENT_INIT_LIMIT_MAX,
    CONCURRENT_INIT_LIMIT_MIN, CONNECT_TIMEOUT_SECS_MAX, CONNECT_TIMEOUT_SECS_MIN, Client,
    DownloadsConfig, Error, FASTRESUME_VALIDATION_DENOM_MAX, FASTRESUME_VALIDATION_DENOM_MIN,
    FoldersConfig, IpFilterConfig, KEEP_ALIVE_SECS_MAX, KEEP_ALIVE_SECS_MIN,
    MAGNET_RESOLUTION_TIMEOUT_SECS_MAX, MAGNET_RESOLUTION_TIMEOUT_SECS_MIN,
    MAX_CONCURRENT_HANDSHAKES_MAX, MAX_CONCURRENT_HANDSHAKES_MIN, NetworkConfig, NetworkSettings,
    PEER_LIMIT_MAX, PEER_LIMIT_MIN, ProxyConfig, READ_WRITE_TIMEOUT_SECS_MAX,
    READ_WRITE_TIMEOUT_SECS_MIN, RateLimits, SeedLimits, SettingsSnapshot, UPLOAD_SLOTS_MAX,
    UPLOAD_SLOTS_MIN,
};
use rtbit_ui_core::{SettingsChanges, map_error};

use crate::bridge::{Bridge, SharedClient};

/// Which tab is open. Ids are the nav item ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Connection,
    Speed,
    Folders,
    Seeding,
    Advanced,
}

impl SettingsTab {
    const ALL: [(Self, &'static str, &'static str); 6] = [
        (Self::General, "general", "General"),
        (Self::Connection, "connection", "Connection"),
        (Self::Speed, "speed", "Speed"),
        (Self::Folders, "folders", "Folders"),
        (Self::Seeding, "seeding", "Seeding"),
        (Self::Advanced, "advanced", "Advanced"),
    ];

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, nav, _)| *nav == id)
            .map(|(tab, _, _)| *tab)
    }

    fn id(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(tab, _, _)| *tab == self)
            .map_or("general", |(_, id, _)| id)
    }
}

const PORT_ERROR: &str = "Enter a port from 1 to 65535.";
const FOLDER_ERROR: &str = "Download folder is required.";
const RESTART_NOTICE: &str = "Saved. Restart to apply.";

/// The caption under an Advanced field: its caps, and what blank means.
fn range_caption(min: u64, max: u64) -> String {
    format!("Allowed {min}-{max}, blank = default")
}

/// Editable copy of the settings. Strings so a half-typed number is not lost.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsDraft {
    pub incomplete_suffix: bool,
    pub connect_timeout: String,
    pub read_write_timeout: String,
    pub keep_alive: String,
    pub upload_slots: String,
    pub magnet_timeout: String,
    pub max_handshakes: String,
    pub fastresume_denom: String,
    pub listen_ip: String,
    pub listen_port: String,
    pub dht: bool,
    pub lsd: bool,
    pub upnp: bool,
    /// Read-only: peer exchange is always on in the daemon.
    pub pex: bool,
    /// The saved proxy URL, shown read-only. The daemon has already masked the password.
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
            upload_slots: opt_num(snap.advanced.upload_slots),
            magnet_timeout: opt_num(snap.advanced.magnet_resolution_timeout_secs),
            max_handshakes: opt_num(snap.advanced.max_concurrent_handshakes),
            fastresume_denom: opt_num(snap.advanced.fastresume_validation_denom),
            listen_ip: net.listen_ip,
            listen_port: net.listen_port,
            dht: net.dht,
            lsd: net.lsd,
            upnp: net.upnp,
            pex: snap.network.pex_enabled,
            proxy_shown: snap.proxy.proxy_url.clone().unwrap_or_default(),
            proxy_edit: String::new(),
            clear_proxy: false,
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
        }
    }

    /// Per-field validation messages, keyed by field name. Empty when the draft is valid.
    pub fn errors(&self) -> BTreeMap<&'static str, String> {
        let checks: [(&'static str, Option<String>); 22] = [
            (
                "connect_timeout",
                range_error(
                    &self.connect_timeout,
                    "Connect timeout",
                    CONNECT_TIMEOUT_SECS_MIN,
                    CONNECT_TIMEOUT_SECS_MAX,
                ),
            ),
            (
                "read_write_timeout",
                range_error(
                    &self.read_write_timeout,
                    "Read/write timeout",
                    READ_WRITE_TIMEOUT_SECS_MIN,
                    READ_WRITE_TIMEOUT_SECS_MAX,
                ),
            ),
            (
                "keep_alive",
                range_error(
                    &self.keep_alive,
                    "Keep-alive",
                    KEEP_ALIVE_SECS_MIN,
                    KEEP_ALIVE_SECS_MAX,
                ),
            ),
            (
                "upload_slots",
                range_error(
                    &self.upload_slots,
                    "Upload slots",
                    UPLOAD_SLOTS_MIN,
                    UPLOAD_SLOTS_MAX,
                ),
            ),
            (
                "magnet_timeout",
                range_error(
                    &self.magnet_timeout,
                    "Magnet resolution timeout",
                    MAGNET_RESOLUTION_TIMEOUT_SECS_MIN,
                    MAGNET_RESOLUTION_TIMEOUT_SECS_MAX,
                ),
            ),
            (
                "max_handshakes",
                range_error(
                    &self.max_handshakes,
                    "Concurrent handshakes",
                    MAX_CONCURRENT_HANDSHAKES_MIN,
                    MAX_CONCURRENT_HANDSHAKES_MAX,
                ),
            ),
            (
                "fastresume_denom",
                range_error(
                    &self.fastresume_denom,
                    "Fastresume validation",
                    FASTRESUME_VALIDATION_DENOM_MIN,
                    FASTRESUME_VALIDATION_DENOM_MAX,
                ),
            ),
            (
                "listen_ip",
                listen_ip_error(&self.listen_ip).map(str::to_owned),
            ),
            (
                "listen_port",
                parse_port(&self.listen_port)
                    .is_none()
                    .then(|| PORT_ERROR.to_owned()),
            ),
            (
                "download_folder",
                self.download_folder
                    .trim()
                    .is_empty()
                    .then(|| FOLDER_ERROR.to_owned()),
            ),
            (
                "download_bps",
                parse_error::<u32>(&self.download_bps, "Download limit"),
            ),
            (
                "upload_bps",
                parse_error::<u32>(&self.upload_bps, "Upload limit"),
            ),
            (
                "peer_limit",
                range_error(
                    &self.peer_limit,
                    "Peer limit",
                    PEER_LIMIT_MIN,
                    PEER_LIMIT_MAX,
                ),
            ),
            (
                "init_limit",
                range_error(
                    &self.init_limit,
                    "Connection limit",
                    CONCURRENT_INIT_LIMIT_MIN,
                    CONCURRENT_INIT_LIMIT_MAX,
                ),
            ),
            (
                "alt_down",
                parse_error::<u64>(&self.alt_down, "Alternative download"),
            ),
            (
                "alt_up",
                parse_error::<u64>(&self.alt_up, "Alternative upload"),
            ),
            (
                "schedule_start",
                parse_error::<u32>(&self.schedule_start, "Schedule start"),
            ),
            (
                "schedule_end",
                parse_error::<u32>(&self.schedule_end, "Schedule end"),
            ),
            (
                "schedule_days",
                parse_error::<u8>(&self.schedule_days, "Schedule days"),
            ),
            ("ratio_limit", parse_opt_f64(&self.ratio_limit).err()),
            (
                "seed_minutes",
                parse_error::<u64>(&self.seed_minutes, "Seed time"),
            ),
            ("schedule_enabled", None),
        ];
        checks
            .into_iter()
            .filter_map(|(key, err)| err.map(|msg| (key, msg)))
            .collect()
    }

    /// The sections that differ from `base`, ready to post. Empty when nothing changed.
    pub fn changes(&self, base: &SettingsSnapshot) -> Result<SettingsChanges, String> {
        if let Some((_, msg)) = self.errors().into_iter().next() {
            return Err(msg);
        }
        let mut out = SettingsChanges::default();

        let downloads = DownloadsConfig {
            incomplete_file_suffix: self.incomplete_suffix,
        };
        if downloads != base.downloads {
            out.downloads = Some(downloads);
        }

        // Peer limit and connection limit are edited on this tab but belong to the
        // limits route (POST /torrents/limits), so this body never sets them: both
        // sides are built from the fields this screen owns plus the default.
        let advanced = AdvancedConfig {
            connect_timeout_secs: parse_opt(&self.connect_timeout, "Connect timeout")?,
            read_write_timeout_secs: parse_opt(&self.read_write_timeout, "Read/write timeout")?,
            keep_alive_secs: parse_opt(&self.keep_alive, "Keep-alive")?,
            upload_slots: parse_opt(&self.upload_slots, "Upload slots")?,
            magnet_resolution_timeout_secs: parse_opt(
                &self.magnet_timeout,
                "Magnet resolution timeout",
            )?,
            max_concurrent_handshakes: parse_opt(&self.max_handshakes, "Concurrent handshakes")?,
            fastresume_validation_denom: parse_opt(
                &self.fastresume_denom,
                "Fastresume validation",
            )?,
        };
        let shown = AdvancedConfig {
            connect_timeout_secs: base.advanced.connect_timeout_secs,
            read_write_timeout_secs: base.advanced.read_write_timeout_secs,
            keep_alive_secs: base.advanced.keep_alive_secs,
            upload_slots: base.advanced.upload_slots,
            magnet_resolution_timeout_secs: base.advanced.magnet_resolution_timeout_secs,
            max_concurrent_handshakes: base.advanced.max_concurrent_handshakes,
            fastresume_validation_denom: base.advanced.fastresume_validation_denom,
        };
        if advanced != shown {
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

        let folders = FoldersConfig {
            download_folder: self.download_folder.trim().to_owned(),
            completed_folder: non_empty(&self.completed_folder),
        };
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

fn opt_num<T: ToString>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn parse_error<T: FromStr>(text: &str, label: &str) -> Option<String> {
    parse_opt::<T>(text, label).err()
}

/// A whole number inside `min..=max`, or blank. Blocks save with a field message otherwise.
fn range_error(text: &str, label: &str, min: u64, max: u64) -> Option<String> {
    match parse_opt::<u64>(text, label) {
        Err(message) => Some(message),
        Ok(Some(n)) if !(min..=max).contains(&n) => {
            Some(format!("{label} must be from {min} to {max}."))
        }
        Ok(_) => None,
    }
}

fn parse_opt<T: FromStr>(text: &str, label: &str) -> Result<Option<T>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse()
        .map(Some)
        .map_err(|_| format!("{label} must be a whole number."))
}

fn parse_req<T: FromStr>(text: &str, label: &str) -> Result<T, String> {
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

/// The text inputs, one entity per draft string field.
struct Form {
    connect_timeout: Entity<TextInput>,
    read_write_timeout: Entity<TextInput>,
    keep_alive: Entity<TextInput>,
    upload_slots: Entity<TextInput>,
    magnet_timeout: Entity<TextInput>,
    max_handshakes: Entity<TextInput>,
    fastresume_denom: Entity<TextInput>,
    listen_ip: Entity<TextInput>,
    listen_port: Entity<TextInput>,
    ip_filter: Entity<TextInput>,
    proxy_edit: Entity<TextInput>,
    download_folder: Entity<TextInput>,
    completed_folder: Entity<TextInput>,
    download_bps: Entity<TextInput>,
    upload_bps: Entity<TextInput>,
    peer_limit: Entity<TextInput>,
    init_limit: Entity<TextInput>,
    alt_down: Entity<TextInput>,
    alt_up: Entity<TextInput>,
    schedule_start: Entity<TextInput>,
    schedule_end: Entity<TextInput>,
    schedule_days: Entity<TextInput>,
    ratio_limit: Entity<TextInput>,
    seed_minutes: Entity<TextInput>,
}

impl Form {
    fn new(cx: &mut Context<SettingsScreen>) -> Self {
        let mut input = || cx.new(|_| TextInput::new());
        Self {
            connect_timeout: input(),
            read_write_timeout: input(),
            keep_alive: input(),
            upload_slots: input(),
            magnet_timeout: input(),
            max_handshakes: input(),
            fastresume_denom: input(),
            listen_ip: input(),
            listen_port: input(),
            ip_filter: input(),
            proxy_edit: input(),
            download_folder: input(),
            completed_folder: input(),
            download_bps: input(),
            upload_bps: input(),
            peer_limit: input(),
            init_limit: input(),
            alt_down: input(),
            alt_up: input(),
            schedule_start: input(),
            schedule_end: input(),
            schedule_days: input(),
            ratio_limit: input(),
            seed_minutes: input(),
        }
    }

    /// Copy a draft into the inputs (a load).
    fn write(&self, draft: &SettingsDraft, cx: &mut Context<SettingsScreen>) {
        let pairs: [(&Entity<TextInput>, &str); 24] = [
            (&self.connect_timeout, &draft.connect_timeout),
            (&self.read_write_timeout, &draft.read_write_timeout),
            (&self.keep_alive, &draft.keep_alive),
            (&self.upload_slots, &draft.upload_slots),
            (&self.magnet_timeout, &draft.magnet_timeout),
            (&self.max_handshakes, &draft.max_handshakes),
            (&self.fastresume_denom, &draft.fastresume_denom),
            (&self.listen_ip, &draft.listen_ip),
            (&self.listen_port, &draft.listen_port),
            (&self.ip_filter, &draft.ip_filter),
            (&self.download_folder, &draft.download_folder),
            (&self.completed_folder, &draft.completed_folder),
            (&self.download_bps, &draft.download_bps),
            (&self.upload_bps, &draft.upload_bps),
            (&self.peer_limit, &draft.peer_limit),
            (&self.init_limit, &draft.init_limit),
            (&self.alt_down, &draft.alt_down),
            (&self.alt_up, &draft.alt_up),
            (&self.schedule_start, &draft.schedule_start),
            (&self.schedule_end, &draft.schedule_end),
            (&self.schedule_days, &draft.schedule_days),
            (&self.ratio_limit, &draft.ratio_limit),
            (&self.seed_minutes, &draft.seed_minutes),
            (&self.proxy_edit, &draft.proxy_edit),
        ];
        for (entity, value) in pairs {
            entity.update(cx, |field, cx| field.set_value(value, cx));
        }
    }
}

/// Screen state. Text lives in the inputs; switches and the nav live here.
pub struct SettingsScreen {
    tab: SettingsTab,
    form: Form,
    incomplete_suffix: bool,
    dht: bool,
    lsd: bool,
    upnp: bool,
    alt_enabled: bool,
    schedule_enabled: bool,
    /// Saved proxy, shown read-only. Set by the last load.
    proxy_shown: String,
    clear_proxy: bool,
    /// Peer exchange is shown read-only; the daemon decides whether it is on.
    pex: bool,
    /// The last daemon read. A save diffs against it.
    baseline: Option<SettingsSnapshot>,
    loading: bool,
    pending: bool,
    notice: Option<String>,
    error: Option<String>,
    _load: Option<Task<()>>,
    _save: Option<Task<()>>,
}

impl SettingsScreen {
    #[must_use]
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tab: SettingsTab::default(),
            form: Form::new(cx),
            incomplete_suffix: false,
            dht: false,
            lsd: false,
            upnp: false,
            proxy_shown: String::new(),
            clear_proxy: false,
            pex: false,
            alt_enabled: false,
            schedule_enabled: false,
            baseline: None,
            loading: false,
            pending: false,
            notice: None,
            error: None,
            _load: None,
            _save: None,
        }
    }

    /// The tab that is open.
    #[must_use]
    pub fn tab(&self) -> SettingsTab {
        self.tab
    }

    /// The message shown in the error bar, if the last load or save failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `true` while a load or save is in flight.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.loading || self.pending
    }

    /// The draft as the user has left it on screen.
    pub fn draft(&self, cx: &App) -> SettingsDraft {
        let text = |entity: &Entity<TextInput>| entity.read(cx).text().to_owned();
        SettingsDraft {
            incomplete_suffix: self.incomplete_suffix,
            connect_timeout: text(&self.form.connect_timeout),
            read_write_timeout: text(&self.form.read_write_timeout),
            keep_alive: text(&self.form.keep_alive),
            upload_slots: text(&self.form.upload_slots),
            magnet_timeout: text(&self.form.magnet_timeout),
            max_handshakes: text(&self.form.max_handshakes),
            fastresume_denom: text(&self.form.fastresume_denom),
            listen_ip: text(&self.form.listen_ip),
            listen_port: text(&self.form.listen_port),
            dht: self.dht,
            lsd: self.lsd,
            upnp: self.upnp,
            pex: self.pex,
            proxy_shown: self.proxy_shown.clone(),
            proxy_edit: text(&self.form.proxy_edit),
            clear_proxy: self.clear_proxy,
            ip_filter: text(&self.form.ip_filter),
            download_folder: text(&self.form.download_folder),
            completed_folder: text(&self.form.completed_folder),
            download_bps: text(&self.form.download_bps),
            upload_bps: text(&self.form.upload_bps),
            peer_limit: text(&self.form.peer_limit),
            init_limit: text(&self.form.init_limit),
            alt_enabled: self.alt_enabled,
            alt_down: text(&self.form.alt_down),
            alt_up: text(&self.form.alt_up),
            schedule_enabled: self.schedule_enabled,
            schedule_start: text(&self.form.schedule_start),
            schedule_end: text(&self.form.schedule_end),
            schedule_days: text(&self.form.schedule_days),
            ratio_limit: text(&self.form.ratio_limit),
            seed_minutes: text(&self.form.seed_minutes),
        }
    }

    /// Read the six settings tabs from the daemon.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        cx.notify();
        let rx = cx
            .global::<Bridge>()
            .call(|client: SharedClient| async move {
                let mut client = client.lock().await;
                client.settings().await
            });
        self._load = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(reply) => reply,
                    Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
                };
                let _ = this.update(cx, |this, cx| this.apply_loaded(reply, cx));
            }),
        );
    }

    fn apply_loaded(&mut self, reply: Result<SettingsSnapshot, Error>, cx: &mut Context<Self>) {
        self.loading = false;
        match reply {
            Ok(snap) => {
                let draft = SettingsDraft::from_snapshot(&snap);
                self.incomplete_suffix = draft.incomplete_suffix;
                self.dht = draft.dht;
                self.lsd = draft.lsd;
                self.upnp = draft.upnp;
                self.pex = draft.pex;
                self.proxy_shown = draft.proxy_shown.clone();
                self.clear_proxy = false;
                self.alt_enabled = draft.alt_enabled;
                self.schedule_enabled = draft.schedule_enabled;
                self.form.write(&draft, cx);
                self.baseline = Some(snap);
                self.error = None;
            }
            Err(err) => self.error = Some(map_error(&err)),
        }
        cx.notify();
    }

    /// Post every section the user changed, then reload so the baseline is the saved state.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        let Some(base) = self.baseline.clone() else {
            self.error = Some("Settings have not loaded yet.".to_owned());
            cx.notify();
            return;
        };
        let draft = self.draft(cx);
        let changes = match draft.changes(&base) {
            Ok(changes) => changes,
            Err(message) => {
                self.error = Some(message);
                self.notice = None;
                cx.notify();
                return;
            }
        };
        if changes.is_empty() {
            self.error = None;
            self.notice = Some("Nothing to save.".to_owned());
            cx.notify();
            return;
        }
        self.pending = true;
        self.error = None;
        self.notice = None;
        // Network values and the two restart knobs only apply on the next start.
        let restart = changes.network.is_some()
            || changes.advanced.as_ref().is_some_and(|adv| {
                adv.max_concurrent_handshakes != base.advanced.max_concurrent_handshakes
                    || adv.fastresume_validation_denom != base.advanced.fastresume_validation_denom
            });
        let rx = cx
            .global::<Bridge>()
            .call(move |client: SharedClient| async move {
                let mut client = client.lock().await;
                post_changes(&mut client, &changes).await
            });
        self._save = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(reply) => reply,
                    Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
                };
                let _ = this.update(cx, |this, cx| this.apply_saved(reply, restart, cx));
            }),
        );
        cx.notify();
    }

    /// "Use startup flags": forget the saved network values so the next start uses the flags.
    pub fn reset_startup_flags(&mut self, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        self.pending = true;
        self.error = None;
        self.notice = None;
        let rx = cx
            .global::<Bridge>()
            .call(move |client: SharedClient| async move {
                let mut client = client.lock().await;
                client.reset_network().await.map(|()| false)
            });
        self._save = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(reply) => reply,
                    Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
                };
                let _ = this.update(cx, |this, cx| this.apply_saved(reply, true, cx));
            }),
        );
        cx.notify();
    }

    /// `daemon_restart` is the restart flag the daemon returned for an advanced save.
    fn apply_saved(&mut self, reply: Result<bool, Error>, restart: bool, cx: &mut Context<Self>) {
        self.pending = false;
        match reply {
            Ok(daemon_restart) => {
                self.notice = Some(if restart || daemon_restart {
                    RESTART_NOTICE.to_owned()
                } else {
                    "Saved.".to_owned()
                });
                self.load(cx);
            }
            Err(err) => self.error = Some(map_error(&err)),
        }
        cx.notify();
    }

    fn select_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(tab) = SettingsTab::from_id(id) {
            self.tab = tab;
            cx.notify();
        }
    }
}

/// Post the edited sections in the daemon's order. `true` when the daemon says a
/// saved advanced value needs a restart.
async fn post_changes(client: &mut Client, changes: &SettingsChanges) -> Result<bool, Error> {
    let mut restart_required = false;
    if let Some(body) = &changes.network {
        client.set_network(body).await?;
    }
    if let Some(body) = &changes.downloads {
        client.set_downloads(body).await?;
    }
    if let Some(body) = &changes.advanced {
        restart_required = client.set_advanced(body).await?.restart_required;
    }
    if let Some(body) = &changes.proxy {
        client.set_proxy(body).await?;
    }
    if let Some(body) = &changes.ip_filter {
        client.set_ip_filter(body).await?;
    }
    if let Some(body) = &changes.folders {
        client.set_folders(body).await?;
    }
    if let Some(body) = &changes.limits {
        client.set_limits(body).await?;
    }
    if let Some(enabled) = changes.alt_enabled {
        client.set_alt_enabled(enabled).await?;
    }
    if let Some(body) = &changes.alt_config {
        client.set_alt_config(body).await?;
    }
    if let Some(body) = &changes.schedule {
        client.set_schedule(body).await?;
    }
    if let Some(body) = &changes.seed_limits {
        client.set_seed_limits(body).await?;
    }
    Ok(restart_required)
}

/// A text field with its validation message, if the draft has one for `key`.
fn text_field(
    label: &'static str,
    input: &Entity<TextInput>,
    errors: &BTreeMap<&'static str, String>,
    key: &'static str,
) -> Field {
    let field = Field::new(input.clone()).label(label);
    match errors.get(key) {
        Some(message) => field.validation(FieldValidationState::Error, message.clone()),
        None => field,
    }
}

/// A numeric field with its caps shown as a caption.
fn range_field(
    label: &'static str,
    input: &Entity<TextInput>,
    errors: &BTreeMap<&'static str, String>,
    key: &'static str,
    (min, max): (u64, u64),
) -> Field {
    text_field(label, input, errors, key).helper_text(range_caption(min, max))
}

/// "Use startup flags": drops the saved network values; the next start uses the flags.
fn reset_button(me: &Entity<SettingsScreen>, busy: bool) -> impl IntoElement {
    let me = me.clone();
    Button::new("settings-startup-flags")
        .label("Use startup flags")
        .disabled(busy)
        .on_click(move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
            me.update(cx, |screen, cx| screen.reset_startup_flags(cx));
        })
}

fn switch_row(
    id: &'static str,
    label: &'static str,
    on: bool,
    me: &Entity<SettingsScreen>,
    set: fn(&mut SettingsScreen, bool),
) -> impl IntoElement {
    let me = me.clone();
    div()
        .flex()
        .gap_2()
        .items_center()
        .child(Switch::new(id).on(on).on_click(move |next, _, _, cx| {
            me.update(cx, |screen, cx| {
                set(screen, next);
                cx.notify();
            });
        }))
        .child(Label::new(label))
}

impl Render for SettingsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me = cx.entity();
        let errors = self.draft(cx).errors();
        let nav_me = me.clone();
        let nav = SelectableList::new("settings-nav")
            .items(
                SettingsTab::ALL
                    .iter()
                    .map(|(_, id, title)| SelectableListItem::new(*id, *title)),
            )
            .selected(self.tab.id())
            .on_select(move |id, _, cx| {
                nav_me.update(cx, |screen, cx| screen.select_tab(&id, cx));
            });

        let body = match self.tab {
            SettingsTab::General => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(switch_row(
                    "incomplete-suffix",
                    "Use .part suffix for unfinished files",
                    self.incomplete_suffix,
                    &me,
                    |s, v| s.incomplete_suffix = v,
                ))
                .child(text_field(
                    "IP filter",
                    &self.form.ip_filter,
                    &errors,
                    "ip_filter",
                )),
            SettingsTab::Connection => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(text_field(
                    "Listen address",
                    &self.form.listen_ip,
                    &errors,
                    "listen_ip",
                ))
                .child(text_field(
                    "Listen port",
                    &self.form.listen_port,
                    &errors,
                    "listen_port",
                ))
                .child(switch_row(
                    "upnp",
                    "UPnP port forward",
                    self.upnp,
                    &me,
                    |s, v| s.upnp = v,
                ))
                .child(switch_row("dht", "DHT", self.dht, &me, |s, v| s.dht = v))
                .child(switch_row(
                    "lsd",
                    "Local discovery",
                    self.lsd,
                    &me,
                    |s, v| s.lsd = v,
                ))
                .child(Label::new(format!(
                    "Peer exchange is always on (read-only{}).",
                    if self.pex { "" } else { ", off in the daemon" }
                )))
                .child(Label::new(if self.proxy_shown.is_empty() {
                    "No proxy saved.".to_owned()
                } else {
                    format!("Saved proxy {}", self.proxy_shown)
                }))
                .child(text_field(
                    "Proxy URL",
                    &self.form.proxy_edit,
                    &errors,
                    "proxy_edit",
                ))
                .child(switch_row(
                    "clear-proxy",
                    "Clear proxy",
                    self.clear_proxy,
                    &me,
                    |s, v| s.clear_proxy = v,
                ))
                .child(Label::new(
                    "Address, port and discovery switches apply on the next start.",
                ))
                .child(reset_button(&me, self.is_busy())),
            SettingsTab::Speed => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(text_field(
                    "Download limit (B/s)",
                    &self.form.download_bps,
                    &errors,
                    "download_bps",
                ))
                .child(text_field(
                    "Upload limit (B/s)",
                    &self.form.upload_bps,
                    &errors,
                    "upload_bps",
                ))
                .child(switch_row(
                    "alt-enabled",
                    "Alternative speed limits",
                    self.alt_enabled,
                    &me,
                    |s, v| s.alt_enabled = v,
                ))
                .child(text_field(
                    "Alternative download (B/s)",
                    &self.form.alt_down,
                    &errors,
                    "alt_down",
                ))
                .child(text_field(
                    "Alternative upload (B/s)",
                    &self.form.alt_up,
                    &errors,
                    "alt_up",
                ))
                .child(switch_row(
                    "schedule-enabled",
                    "Schedule alternative limits",
                    self.schedule_enabled,
                    &me,
                    |s, v| s.schedule_enabled = v,
                ))
                .child(text_field(
                    "Schedule start (minutes)",
                    &self.form.schedule_start,
                    &errors,
                    "schedule_start",
                ))
                .child(text_field(
                    "Schedule end (minutes)",
                    &self.form.schedule_end,
                    &errors,
                    "schedule_end",
                ))
                .child(text_field(
                    "Schedule days (bitmask)",
                    &self.form.schedule_days,
                    &errors,
                    "schedule_days",
                )),
            SettingsTab::Folders => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(text_field(
                    "Download folder",
                    &self.form.download_folder,
                    &errors,
                    "download_folder",
                ))
                .child(text_field(
                    "Completed folder",
                    &self.form.completed_folder,
                    &errors,
                    "completed_folder",
                )),
            SettingsTab::Seeding => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(Label::new(
                    "Blank turns the limit off. Seed time is minutes.",
                ))
                .child(text_field(
                    "Ratio limit",
                    &self.form.ratio_limit,
                    &errors,
                    "ratio_limit",
                ))
                .child(text_field(
                    "Seed time (minutes)",
                    &self.form.seed_minutes,
                    &errors,
                    "seed_minutes",
                )),
            SettingsTab::Advanced => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(Label::new("Timeouts"))
                .child(range_field(
                    "Connect timeout (s)",
                    &self.form.connect_timeout,
                    &errors,
                    "connect_timeout",
                    (CONNECT_TIMEOUT_SECS_MIN, CONNECT_TIMEOUT_SECS_MAX),
                ))
                .child(range_field(
                    "Read/write timeout (s)",
                    &self.form.read_write_timeout,
                    &errors,
                    "read_write_timeout",
                    (READ_WRITE_TIMEOUT_SECS_MIN, READ_WRITE_TIMEOUT_SECS_MAX),
                ))
                .child(range_field(
                    "Keep-alive (s)",
                    &self.form.keep_alive,
                    &errors,
                    "keep_alive",
                    (KEEP_ALIVE_SECS_MIN, KEEP_ALIVE_SECS_MAX),
                ))
                .child(range_field(
                    "Magnet resolution timeout (s)",
                    &self.form.magnet_timeout,
                    &errors,
                    "magnet_timeout",
                    (
                        MAGNET_RESOLUTION_TIMEOUT_SECS_MIN,
                        MAGNET_RESOLUTION_TIMEOUT_SECS_MAX,
                    ),
                ))
                .child(Label::new("Limits"))
                .child(range_field(
                    "Peer limit (per torrent)",
                    &self.form.peer_limit,
                    &errors,
                    "peer_limit",
                    (PEER_LIMIT_MIN, PEER_LIMIT_MAX),
                ))
                .child(range_field(
                    "Connection limit (torrent starts at once)",
                    &self.form.init_limit,
                    &errors,
                    "init_limit",
                    (CONCURRENT_INIT_LIMIT_MIN, CONCURRENT_INIT_LIMIT_MAX),
                ))
                .child(range_field(
                    "Upload slots (per torrent)",
                    &self.form.upload_slots,
                    &errors,
                    "upload_slots",
                    (UPLOAD_SLOTS_MIN, UPLOAD_SLOTS_MAX),
                ))
                .child(Label::new("Needs restart"))
                .child(range_field(
                    "Concurrent handshakes",
                    &self.form.max_handshakes,
                    &errors,
                    "max_handshakes",
                    (MAX_CONCURRENT_HANDSHAKES_MIN, MAX_CONCURRENT_HANDSHAKES_MAX),
                ))
                .child(range_field(
                    "Fastresume validation (0 = skip)",
                    &self.form.fastresume_denom,
                    &errors,
                    "fastresume_denom",
                    (
                        FASTRESUME_VALIDATION_DENOM_MIN,
                        FASTRESUME_VALIDATION_DENOM_MAX,
                    ),
                ))
                .child(Label::new(
                    "Handshake and fastresume values apply after a restart.",
                )),
        };

        let save = cx.listener(|this, _: &ClickEvent, _window, cx| this.save(cx));
        let mut panel = div().flex().flex_col().gap_3().w(px(420.0)).child(body);
        if let Some(message) = &self.error {
            panel = panel.child(MessageBar::new(message.clone()).intent(MessageIntent::Error));
        }
        if let Some(message) = &self.notice {
            panel = panel.child(Label::new(message.clone()));
        }
        panel = panel.child(
            Button::new("settings-save")
                .label(if self.pending { "Saving…" } else { "Save" })
                .appearance(ButtonAppearance::Accent)
                .disabled(self.is_busy() || self.baseline.is_none())
                .on_click(save),
        );

        div()
            .flex()
            .gap_4()
            .child(div().w(px(200.0)).child(nav))
            .child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            folders: FoldersConfig {
                download_folder: "/downloads".into(),
                completed_folder: None,
            },
            network: NetworkConfig {
                listen_port: Some(6881),
                ..NetworkConfig::default()
            },
            ..SettingsSnapshot::default()
        }
    }

    #[test]
    fn untouched_draft_posts_nothing() {
        let snap = snapshot();
        let draft = SettingsDraft::from_snapshot(&snap);
        assert!(draft.changes(&snap).unwrap().is_empty());
    }

    #[test]
    fn edited_folder_posts_only_folders() {
        let snap = snapshot();
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.download_folder = "/data".into();
        let changes = draft.changes(&snap).unwrap();
        assert_eq!(
            changes.folders,
            Some(FoldersConfig {
                download_folder: "/data".into(),
                completed_folder: None,
            })
        );
        assert!(changes.limits.is_none());
        assert!(changes.network.is_none());
    }

    #[test]
    fn blank_download_folder_is_a_validation_error() {
        let snap = snapshot();
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.download_folder = "   ".into();
        assert_eq!(
            draft.errors().get("download_folder").map(String::as_str),
            Some(FOLDER_ERROR)
        );
        assert_eq!(draft.changes(&snap), Err(FOLDER_ERROR.to_owned()));
    }

    #[test]
    fn a_half_typed_number_is_kept_and_flagged() {
        let snap = snapshot();
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.upload_bps = "12x".into();
        assert_eq!(
            draft.errors().get("upload_bps").map(String::as_str),
            Some("Upload limit must be a whole number.")
        );
        assert!(draft.changes(&snap).is_err());
    }

    #[test]
    fn bad_listen_ip_and_port_are_flagged() {
        let snap = snapshot();
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.listen_ip = "300.1.1.1".into();
        draft.listen_port = "0".into();
        let errors = draft.errors();
        assert!(errors.get("listen_ip").unwrap().contains("valid"));
        assert_eq!(
            errors.get("listen_port").map(String::as_str),
            Some(PORT_ERROR)
        );
    }

    #[test]
    fn over_cap_and_under_min_advanced_values_block_changes() {
        let snap = snapshot();
        type Setter = fn(&mut SettingsDraft, String);
        let cases: [(&str, Setter); 8] = [
            ("connect_timeout", |d, v| d.connect_timeout = v),
            ("read_write_timeout", |d, v| d.read_write_timeout = v),
            ("keep_alive", |d, v| d.keep_alive = v),
            ("upload_slots", |d, v| d.upload_slots = v),
            ("magnet_timeout", |d, v| d.magnet_timeout = v),
            ("max_handshakes", |d, v| d.max_handshakes = v),
            ("peer_limit", |d, v| d.peer_limit = v),
            ("init_limit", |d, v| d.init_limit = v),
        ];
        let bounds = [
            (CONNECT_TIMEOUT_SECS_MIN, CONNECT_TIMEOUT_SECS_MAX),
            (READ_WRITE_TIMEOUT_SECS_MIN, READ_WRITE_TIMEOUT_SECS_MAX),
            (KEEP_ALIVE_SECS_MIN, KEEP_ALIVE_SECS_MAX),
            (UPLOAD_SLOTS_MIN, UPLOAD_SLOTS_MAX),
            (
                MAGNET_RESOLUTION_TIMEOUT_SECS_MIN,
                MAGNET_RESOLUTION_TIMEOUT_SECS_MAX,
            ),
            (MAX_CONCURRENT_HANDSHAKES_MIN, MAX_CONCURRENT_HANDSHAKES_MAX),
            (PEER_LIMIT_MIN, PEER_LIMIT_MAX),
            (CONCURRENT_INIT_LIMIT_MIN, CONCURRENT_INIT_LIMIT_MAX),
        ];
        for ((key, set), (min, max)) in cases.into_iter().zip(bounds) {
            for ok in [min, max] {
                let mut draft = SettingsDraft::from_snapshot(&snap);
                set(&mut draft, ok.to_string());
                assert!(!draft.errors().contains_key(key), "{key}={ok} is allowed");
                assert!(draft.changes(&snap).is_ok(), "{key}={ok} saves");
            }
            for bad in [max + 1, min.wrapping_sub(1)] {
                if bad == u64::MAX {
                    continue; // a minimum of 0 has no under-min value
                }
                let mut draft = SettingsDraft::from_snapshot(&snap);
                set(&mut draft, bad.to_string());
                let message = draft.errors().get(key).cloned();
                assert!(message.is_some(), "{key}={bad} is refused");
                assert_eq!(draft.changes(&snap), Err(message.unwrap()));
            }
        }
    }

    #[test]
    fn fastresume_zero_is_allowed_and_one_over_is_not() {
        let snap = snapshot();
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.fastresume_denom = "0".into();
        let body = draft.changes(&snap).unwrap().advanced.unwrap();
        assert_eq!(body.fastresume_validation_denom, Some(0));
        draft.fastresume_denom = (FASTRESUME_VALIDATION_DENOM_MAX + 1).to_string();
        assert!(draft.changes(&snap).is_err());
    }

    #[test]
    fn blank_advanced_fields_mean_default_and_post_nothing() {
        let snap = snapshot();
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.upload_slots = String::new();
        assert!(draft.changes(&snap).unwrap().is_empty());
    }

    #[test]
    fn the_advanced_tab_is_listed() {
        assert_eq!(
            SettingsTab::from_id("advanced"),
            Some(SettingsTab::Advanced)
        );
        assert!(SettingsTab::ALL.contains(&(SettingsTab::Advanced, "advanced", "Advanced")));
    }

    #[test]
    fn clearing_the_suffix_switch_posts_downloads() {
        let mut snap = snapshot();
        snap.downloads.incomplete_file_suffix = true;
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.incomplete_suffix = false;
        let changes = draft.changes(&snap).unwrap();
        assert_eq!(
            changes.downloads,
            Some(DownloadsConfig {
                incomplete_file_suffix: false
            })
        );
    }

    #[test]
    fn seed_minutes_post_as_seconds() {
        let snap = snapshot();
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.seed_minutes = "30".into();
        let changes = draft.changes(&snap).unwrap();
        assert_eq!(
            changes.seed_limits,
            Some(SeedLimits {
                ratio_limit: None,
                time_limit_secs: Some(1800)
            })
        );
    }

    #[test]
    fn listen_ip_blank_clears_a_saved_ip() {
        let mut snap = snapshot();
        snap.network.listen_ip = Some("10.0.0.1".into());
        snap.network.listen_port = Some(6881);
        snap.network.can_save = true;
        snap.network.pending = Some(NetworkSettings {
            listen_ip: Some("10.0.0.1".into()),
            ..NetworkSettings::default()
        });
        let mut draft = SettingsDraft::from_snapshot(&snap);
        draft.listen_ip = String::new();
        let body = draft.changes(&snap).unwrap().network.unwrap();
        assert_eq!(body.reset, Some(true));
        assert_eq!(body.listen_ip, None);
        assert_eq!(body.listen_port, Some(6881));
    }

    /// Serve login and every settings GET from the client fixtures, plus a seed-limits POST.
    fn daemon(cx: &mut gpui::TestAppContext) -> (tokio::runtime::Runtime, wiremock::MockServer) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        const FIXTURES: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../rtbit-client/tests/fixtures"
        );
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            let json = |name: &str| {
                let body = std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap();
                ResponseTemplate::new(200).set_body_raw(body, "application/json")
            };
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(json("login.json"))
                .mount(&server)
                .await;
            for (route, file) in [
                ("/session/downloads", "downloads.json"),
                ("/session/advanced", "advanced.json"),
                ("/session/network", "network.json"),
                ("/session/proxy", "proxy.json"),
                ("/session/ip-filter", "ip-filter.json"),
                ("/session/folders", "folders.json"),
                ("/torrents/limits", "limits.json"),
                ("/speed/alt", "alt.json"),
                ("/speed/schedule", "schedule.json"),
                ("/torrents/seed_limits", "seed.json"),
            ] {
                Mock::given(method("GET"))
                    .and(path(route))
                    .respond_with(json(file))
                    .mount(&server)
                    .await;
            }
            Mock::given(method("POST"))
                .and(path("/torrents/seed_limits"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/session/advanced"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(r#"{"restart_required":true}"#, "application/json"),
                )
                .mount(&server)
                .await;
            server
        });
        let bridge = Bridge::new(&server.uri()).unwrap();
        let login =
            bridge.call(|client| async move { client.lock().await.login("admin", "secret").await });
        bridge.runtime().block_on(login).unwrap().unwrap();
        cx.update(|cx| bridge.install(cx));
        (runtime, server)
    }

    fn drive(
        cx: &mut gpui::TestAppContext,
        mut done: impl FnMut(&mut gpui::TestAppContext) -> bool,
    ) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("daemon reply never arrived");
    }

    #[gpui::test]
    fn a_save_posts_only_the_changed_ratio(cx: &mut gpui::TestAppContext) {
        let (runtime, server) = daemon(cx);
        let screen = cx.new(SettingsScreen::new);
        screen.update(cx, |screen, cx| screen.load(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        screen.read_with(cx, |s, _| assert!(s.error().is_none(), "{:?}", s.error()));
        let ratio = screen.read_with(cx, |s, _| s.form.ratio_limit.clone());
        ratio.update(cx, |field, cx| field.set_value("3.5", cx));
        screen.update(cx, |screen, cx| screen.save(cx));
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| !s.is_busy() && s.notice.is_some())
        });
        screen.read_with(cx, |s, _| {
            assert!(s.error().is_none(), "{:?}", s.error());
            assert_eq!(s.notice.as_deref(), Some("Saved."));
        });
        let posts: Vec<_> = runtime
            .block_on(server.received_requests())
            .unwrap()
            .into_iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() != "/auth/login")
            .collect();
        assert_eq!(posts.len(), 1, "only the seed limits change is posted");
        assert_eq!(posts[0].url.path(), "/torrents/seed_limits");
        let body: serde_json::Value = serde_json::from_slice(&posts[0].body).unwrap();
        assert_eq!(body["ratio_limit"], 3.5);
    }

    #[gpui::test]
    fn an_advanced_save_posts_the_edited_knob_and_shows_the_restart_notice(
        cx: &mut gpui::TestAppContext,
    ) {
        let (runtime, server) = daemon(cx);
        let screen = cx.new(SettingsScreen::new);
        screen.update(cx, |screen, cx| screen.load(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        screen.read_with(cx, |s, cx| {
            assert!(s.error().is_none(), "{:?}", s.error());
            let draft = s.draft(cx);
            assert_eq!(draft.upload_slots, "4");
            assert_eq!(draft.max_handshakes, "64");
        });
        let slots = screen.read_with(cx, |s, _| s.form.upload_slots.clone());
        slots.update(cx, |field, cx| field.set_value("8", cx));
        screen.update(cx, |screen, cx| screen.save(cx));
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| !s.is_busy() && s.notice.is_some())
        });
        screen.read_with(cx, |s, _| {
            assert!(s.error().is_none(), "{:?}", s.error());
            assert_eq!(s.notice.as_deref(), Some(RESTART_NOTICE));
        });
        let posts: Vec<_> = runtime
            .block_on(server.received_requests())
            .unwrap()
            .into_iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() != "/auth/login")
            .collect();
        assert_eq!(posts.len(), 1, "only the advanced section is posted");
        assert_eq!(posts[0].url.path(), "/session/advanced");
        let body: serde_json::Value = serde_json::from_slice(&posts[0].body).unwrap();
        assert_eq!(body["upload_slots"], 8);
        assert_eq!(body["connect_timeout_secs"], 10);
        assert_eq!(body["max_concurrent_handshakes"], 64);
        assert!(body.get("peer_limit").is_none(), "limits post separately");
    }

    #[gpui::test]
    fn an_over_cap_advanced_field_blocks_the_save(cx: &mut gpui::TestAppContext) {
        let (runtime, server) = daemon(cx);
        let screen = cx.new(SettingsScreen::new);
        screen.update(cx, |screen, cx| screen.load(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        let slots = screen.read_with(cx, |s, _| s.form.upload_slots.clone());
        slots.update(cx, |field, cx| {
            field.set_value((UPLOAD_SLOTS_MAX + 1).to_string(), cx);
        });
        screen.update(cx, |screen, cx| screen.save(cx));
        screen.read_with(cx, |s, _| {
            assert!(s.error().unwrap().contains("Upload slots"));
            assert!(!s.is_busy());
        });
        let posts = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() != "/auth/login")
            .count();
        assert_eq!(posts, 0);
    }

    #[gpui::test]
    fn an_unchanged_form_saves_nothing(cx: &mut gpui::TestAppContext) {
        let (runtime, server) = daemon(cx);
        let screen = cx.new(SettingsScreen::new);
        screen.update(cx, |screen, cx| screen.load(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        screen.update(cx, |screen, cx| screen.save(cx));
        screen.read_with(cx, |s, _| {
            assert_eq!(s.notice.as_deref(), Some("Nothing to save."));
        });
        let posts = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() != "/auth/login")
            .count();
        assert_eq!(posts, 0);
    }

    #[gpui::test]
    fn load_fills_the_form_from_the_daemon(cx: &mut gpui::TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let screen = cx.new(SettingsScreen::new);
        screen.update(cx, |screen, cx| screen.load(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        screen.read_with(cx, |s, cx| {
            assert!(s.error().is_none(), "{:?}", s.error());
            let draft = s.draft(cx);
            assert_eq!(draft.download_folder, "/data/in");
            assert_eq!(draft.download_bps, "1000000");
            assert_eq!(draft.peer_limit, "50");
            assert_eq!(draft.listen_port, "6881");
            assert!(draft.dht);
            assert!(!draft.lsd);
            let baseline = s.baseline.as_ref().expect("baseline kept for save");
            assert!(draft.changes(baseline).unwrap().is_empty());
        });
    }

    #[gpui::test]
    fn refused_save_shows_the_error_bar(cx: &mut gpui::TestAppContext) {
        let (runtime, server) = daemon(cx);
        runtime.block_on(async {
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/torrents/seed_limits"))
                .respond_with(wiremock::ResponseTemplate::new(500))
                .up_to_n_times(1)
                .with_priority(1)
                .mount(&server)
                .await;
        });
        let screen = cx.new(SettingsScreen::new);
        screen.update(cx, |screen, cx| screen.load(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        let ratio = screen.read_with(cx, |s, _| s.form.ratio_limit.clone());
        ratio.update(cx, |field, cx| field.set_value("3.5", cx));
        screen.update(cx, |screen, cx| screen.save(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        screen.read_with(cx, |s, _| {
            assert!(s.error().is_some(), "refused save must show the error bar");
            assert!(s.notice.is_none());
        });
    }
}

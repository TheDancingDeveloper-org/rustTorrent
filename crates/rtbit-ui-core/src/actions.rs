//! What a screen asks the daemon to do. The shell posts these through
//! [`crate::LoginAttempt`].

use rtbit_client::{
    AdvancedConfig, AltSpeedConfig, AltSpeedSchedule, DownloadsConfig, FoldersConfig,
    IpFilterConfig, NetworkSettings, ProxyConfig, RateLimits, SeedLimits,
};

/// A torrent the user wants the daemon to add.
///
/// Either a magnet link or the bytes of a `.torrent` file, plus the save path,
/// an optional category and whether to start paused. The dialog has already
/// checked the magnet and the path; the daemon checks them again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddRequest {
    pub source: AddSource,
    pub save_path: String,
    pub category: String,
    pub start_paused: bool,
}

/// Where the torrent comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddSource {
    /// A magnet link, already checked as `magnet:?xt=urn:btih:`.
    Magnet(String),
    /// Bytes of a `.torrent` file the user picked.
    File { name: String, bytes: Vec<u8> },
}

/// A row command the daemon understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Start,
    Pause,
    Delete,
}

/// A change the pane asked for. The shell posts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailAction {
    /// Toggle one file in or out of the download selection.
    SetIncluded { index: usize, included: bool },
    /// Set every listed file the same way. A folder checkbox uses this so the
    /// files under it move together.
    SetIncludedMany { indices: Vec<usize>, included: bool },
    /// Attach one announce URL.
    AddTracker(String),
}

/// One section to post. `None` means that section was not edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsChanges {
    pub downloads: Option<DownloadsConfig>,
    /// Saved for the next start. Includes `reset` for "Use startup flags".
    pub network: Option<NetworkSettings>,
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
            && self.network.is_none()
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

impl SettingsChanges {
    /// Only the network reset: forget every saved network value.
    pub fn use_startup_flags() -> Self {
        Self {
            network: Some(NetworkSettings {
                reset: Some(true),
                ..NetworkSettings::default()
            }),
            ..Self::default()
        }
    }
}

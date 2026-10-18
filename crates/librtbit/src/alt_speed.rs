/// Alternative ("turtle mode") speed limits, qBittorrent-style.
///
/// When enabled, the alternative rates temporarily replace the session's
/// normal rate limits; disabling restores them. An optional weekly schedule
/// toggles the alternative limits automatically.
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, debug_span, info};

use crate::session::Session;

#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug)]
pub struct AltSpeedConfig {
    /// Alternative download limit in bytes/sec (None = unlimited).
    pub download_rate: Option<u64>,
    /// Alternative upload limit in bytes/sec (None = unlimited).
    pub upload_rate: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug)]
pub struct AltSpeedSchedule {
    pub enabled: bool,
    /// Minutes from local midnight.
    #[serde(default)]
    pub start_minutes: u32,
    #[serde(default)]
    pub end_minutes: u32,
    /// Bitmask: 1=Mon, 2=Tue, 4=Wed, 8=Thu, 16=Fri, 32=Sat, 64=Sun.
    #[serde(default)]
    pub days: u8,
}

#[derive(Serialize)]
pub struct AltSpeedStatus {
    pub enabled: bool,
    pub config: AltSpeedConfig,
    pub schedule: Option<AltSpeedSchedule>,
}

#[derive(Default)]
pub struct AltSpeedState {
    enabled: AtomicBool,
    config: RwLock<AltSpeedConfig>,
    schedule: RwLock<AltSpeedSchedule>,
    /// Normal (down, up) limits saved while alternative limits are active.
    saved_normal: RwLock<Option<(Option<NonZeroU32>, Option<NonZeroU32>)>>,
    /// Last decision made by the scheduler, so manual toggles between window
    /// boundaries aren't constantly overridden.
    last_schedule_decision: RwLock<Option<bool>>,
}

fn to_bps(rate: Option<u64>) -> Option<NonZeroU32> {
    rate.and_then(|r| NonZeroU32::new(u32::try_from(r).unwrap_or(u32::MAX)))
}

impl Session {
    pub fn alt_speed_status(&self) -> AltSpeedStatus {
        AltSpeedStatus {
            enabled: self.alt_speed.enabled.load(Ordering::SeqCst),
            config: *self.alt_speed.config.read(),
            schedule: Some(*self.alt_speed.schedule.read()),
        }
    }

    pub fn alt_speed_enabled(&self) -> bool {
        self.alt_speed.enabled.load(Ordering::SeqCst)
    }

    pub fn set_alt_speed_enabled(&self, enabled: bool) {
        let was = self.alt_speed.enabled.swap(enabled, Ordering::SeqCst);
        if was == enabled {
            return;
        }
        if enabled {
            let normal = self.ratelimits.get_config();
            *self.alt_speed.saved_normal.write() = Some((normal.download_bps, normal.upload_bps));
            let alt = *self.alt_speed.config.read();
            self.ratelimits.set_download_bps(to_bps(alt.download_rate));
            self.ratelimits.set_upload_bps(to_bps(alt.upload_rate));
            info!(?alt, "alternative speed limits enabled");
        } else if let Some((down, up)) = self.alt_speed.saved_normal.write().take() {
            self.ratelimits.set_download_bps(down);
            self.ratelimits.set_upload_bps(up);
            info!("alternative speed limits disabled, normal limits restored");
        }
    }

    pub fn set_alt_speed_config(&self, config: AltSpeedConfig) {
        *self.alt_speed.config.write() = config;
        if self.alt_speed_enabled() {
            self.ratelimits
                .set_download_bps(to_bps(config.download_rate));
            self.ratelimits.set_upload_bps(to_bps(config.upload_rate));
        }
    }

    pub fn alt_speed_schedule(&self) -> AltSpeedSchedule {
        *self.alt_speed.schedule.read()
    }

    pub fn set_alt_speed_schedule(&self, schedule: AltSpeedSchedule) {
        *self.alt_speed.schedule.write() = schedule;
        // Re-evaluate immediately on next scheduler tick.
        *self.alt_speed.last_schedule_decision.write() = None;
    }

    /// Set normal session rate limits. While alternative limits are active,
    /// updates the saved normal limits instead of the live limiter so they
    /// take effect once alternative limits are disabled.
    pub fn set_normal_rate_limits(
        &self,
        download_bps: Option<NonZeroU32>,
        upload_bps: Option<NonZeroU32>,
    ) {
        if self.alt_speed_enabled() {
            *self.alt_speed.saved_normal.write() = Some((download_bps, upload_bps));
        } else {
            self.ratelimits.set_download_bps(download_bps);
            self.ratelimits.set_upload_bps(upload_bps);
        }
    }

    /// Normal session rate limits as configured, whether or not alternative
    /// limits are currently replacing them in the live limiter.
    pub fn normal_rate_limits(&self) -> (Option<NonZeroU32>, Option<NonZeroU32>) {
        if let Some(saved) = *self.alt_speed.saved_normal.read() {
            return saved;
        }
        let live = self.ratelimits.get_config();
        (live.download_bps, live.upload_bps)
    }

    fn alt_speed_schedule_wants_active(schedule: &AltSpeedSchedule) -> bool {
        use chrono::{Datelike, Local, Timelike};
        let now = Local::now();
        let day_bit = 1u8 << now.weekday().num_days_from_monday();
        if schedule.days & day_bit == 0 {
            return false;
        }
        let minutes = now.hour() * 60 + now.minute();
        let (start, end) = (schedule.start_minutes, schedule.end_minutes);
        if start <= end {
            minutes >= start && minutes < end
        } else {
            // Overnight window (e.g. 22:00 -> 06:00)
            minutes >= start || minutes < end
        }
    }

    pub(crate) fn spawn_alt_speed_scheduler(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        self.spawn(
            debug_span!(parent: self.rs(), "alt_speed_scheduler"),
            "alt_speed_scheduler",
            async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    let session = match weak.upgrade() {
                        Some(s) => s,
                        None => return Ok(()),
                    };
                    let schedule = session.alt_speed_schedule();
                    if !schedule.enabled {
                        *session.alt_speed.last_schedule_decision.write() = None;
                        continue;
                    }
                    let should = Self::alt_speed_schedule_wants_active(&schedule);
                    let mut last = session.alt_speed.last_schedule_decision.write();
                    if *last != Some(should) {
                        *last = Some(should);
                        drop(last);
                        debug!(should, "alt speed scheduler toggling");
                        session.set_alt_speed_enabled(should);
                    }
                }
            },
        );
    }
}

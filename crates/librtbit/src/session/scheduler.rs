//! Periodic policy pass for the session: share-ratio and seeding-time limits,
//! download/seed queueing, and the low-disk guard.
//!
//! Each pass looks at every torrent, decides what should be running, and then
//! pauses or resumes torrents to match. The decisions themselves are pure
//! functions so they can be tested without a session.

use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tracing::{debug, debug_span, info, warn};

use super::{ManagedTorrentHandle, Session, TorrentId};
use crate::torrent_state::ManagedTorrentShared;

/// How often the policy pass runs.
const POLICY_INTERVAL: Duration = Duration::from_secs(5);

/// Why the policy pass paused a torrent. Stored as flags on the torrent so the
/// pass knows what it may resume, and persisted so the reason survives a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyPause {
    /// Moved back into a queue to make room for another torrent.
    Queued,
    /// Held by the low-disk guard until free space recovers.
    DiskHold,
    /// Reached a share limit.
    ShareLimit,
}

/// The policy state kept on a torrent, as stored in the shared state and in
/// session persistence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PolicyFlags {
    pub queued: bool,
    pub held_for_disk: bool,
    pub share_paused: bool,
}

impl PolicyFlags {
    pub(crate) fn read(shared: &ManagedTorrentShared) -> Self {
        Self {
            queued: shared.queued.load(Ordering::Relaxed),
            held_for_disk: shared.held_for_disk.load(Ordering::Relaxed),
            share_paused: shared.share_paused.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn store(self, shared: &ManagedTorrentShared) {
        shared.queued.store(self.queued, Ordering::Relaxed);
        shared
            .held_for_disk
            .store(self.held_for_disk, Ordering::Relaxed);
        shared
            .share_paused
            .store(self.share_paused, Ordering::Relaxed);
    }

    pub(crate) fn for_pause(reason: PolicyPause) -> Self {
        match reason {
            PolicyPause::Queued => Self {
                queued: true,
                ..Default::default()
            },
            PolicyPause::DiskHold => Self {
                held_for_disk: true,
                ..Default::default()
            },
            PolicyPause::ShareLimit => Self {
                share_paused: true,
                ..Default::default()
            },
        }
    }
}

/// Per-torrent limits after which a seeding torrent is paused. Any one limit
/// alone is enough; `None` disables that limit.
///
/// The ratio is `uploaded / total size`, counting uploads from earlier runs.
/// Seeding time counts from completion. Inactive seeding time counts from the
/// last pass that saw the upload counter move; that clock is not persisted, so
/// it restarts with the process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ShareLimits {
    pub max_ratio: Option<f64>,
    pub max_seeding_time: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_inactive_seeding_time: Option<Duration>,
}

impl ShareLimits {
    /// True when no limit is set. Used to leave the limits out of state.
    pub(crate) fn is_unset(&self) -> bool {
        self.max_ratio.is_none()
            && self.max_seeding_time.is_none()
            && self.max_inactive_seeding_time.is_none()
    }
}

/// Why a seeding torrent hit its share limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShareLimitReason {
    Ratio,
    SeedingTime,
    InactiveSeedingTime,
}

/// Decide whether a finished torrent has reached its share limits.
///
/// `seeding_secs` is zero when the completion time is unknown, which disables
/// the seeding-time check rather than tripping it. `inactive_secs` is the time
/// since the upload counter last moved, and zero likewise disables its check.
pub(crate) fn effective_share_limits(own: ShareLimits, global: ShareLimits) -> ShareLimits {
    if own.is_unset() { global } else { own }
}

pub(crate) fn share_limit_reached(
    limits: &ShareLimits,
    uploaded_bytes: u64,
    total_bytes: u64,
    seeding_secs: u64,
    inactive_secs: u64,
) -> Option<ShareLimitReason> {
    if let Some(max_ratio) = limits.max_ratio
        && total_bytes > 0
        && (uploaded_bytes as f64 / total_bytes as f64) >= max_ratio
    {
        return Some(ShareLimitReason::Ratio);
    }
    if let Some(max_time) = limits.max_seeding_time
        && seeding_secs > 0
        && seeding_secs >= max_time.as_secs()
    {
        return Some(ShareLimitReason::SeedingTime);
    }
    if let Some(max_idle) = limits.max_inactive_seeding_time
        && inactive_secs > 0
        && inactive_secs >= max_idle.as_secs()
    {
        return Some(ShareLimitReason::InactiveSeedingTime);
    }
    None
}

/// Seconds since a seed's upload counter last moved, updating the remembered
/// `(uploaded_bytes, seen_at)` pair when it has. A fresh pair starts the clock.
pub(crate) fn inactive_seconds(activity: &mut (u64, u64), uploaded_bytes: u64, now: u64) -> u64 {
    if activity.1 == 0 || activity.0 != uploaded_bytes {
        *activity = (uploaded_bytes, now);
    }
    now.saturating_sub(activity.1)
}

/// Result of a queue decision for one kind (downloads or seeds).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct QueuePlan {
    /// Running torrents to pause and move back into the queue.
    pub demote: Vec<TorrentId>,
    /// Waiting torrents to start.
    pub promote: Vec<TorrentId>,
}

/// A torrent's place in a queue: its queue position, then its id to break ties.
pub(crate) type QueueKey = (i64, TorrentId);

/// Decide which torrents to run given a limit.
///
/// The torrents at the front of the queue (lowest position) hold the slots.
/// When the queue is full, a waiting torrent ahead of a running one takes its
/// slot, so moving a torrent up the queue starts it. `limit == None` means
/// unlimited: nothing is demoted and every waiting torrent is promoted.
pub(crate) fn plan_queue(
    running: &[QueueKey],
    waiting: &[QueueKey],
    limit: Option<usize>,
) -> QueuePlan {
    let Some(limit) = limit else {
        let mut waiting = waiting.to_vec();
        waiting.sort_unstable();
        return QueuePlan {
            demote: Vec::new(),
            promote: waiting.into_iter().map(|(_, id)| id).collect(),
        };
    };

    let mut all: Vec<(QueueKey, bool)> = running
        .iter()
        .map(|&k| (k, true))
        .chain(waiting.iter().map(|&k| (k, false)))
        .collect();
    all.sort_unstable();
    let mut plan = QueuePlan::default();
    for (rank, ((_, id), is_running)) in all.into_iter().enumerate() {
        match (rank < limit, is_running) {
            (true, false) => plan.promote.push(id),
            (false, true) => plan.demote.push(id),
            _ => {}
        }
    }
    plan
}

/// A move within the queue, as the WebUI API's priority endpoints request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMove {
    Top,
    Bottom,
    Up,
    Down,
}

/// Apply `mv` to the torrents in `selected` within `order` (every torrent's
/// id, front of the queue first). Returns the new order. Selected torrents
/// keep their relative order; `Up`/`Down` move each one past one neighbour
/// that is not selected.
pub(crate) fn reorder_queue(
    order: &[TorrentId],
    selected: &std::collections::HashSet<TorrentId>,
    mv: QueueMove,
) -> Vec<TorrentId> {
    let (picked, rest): (Vec<_>, Vec<_>) = order.iter().partition(|id| selected.contains(id));
    match mv {
        QueueMove::Top => picked.into_iter().chain(rest).collect(),
        QueueMove::Bottom => rest.into_iter().chain(picked).collect(),
        QueueMove::Up => {
            let mut v = order.to_vec();
            for i in 1..v.len() {
                if selected.contains(&v[i]) && !selected.contains(&v[i - 1]) {
                    v.swap(i, i - 1);
                }
            }
            v
        }
        QueueMove::Down => {
            let mut v = order.to_vec();
            for i in (0..v.len().saturating_sub(1)).rev() {
                if selected.contains(&v[i]) && !selected.contains(&v[i + 1]) {
                    v.swap(i, i + 1);
                }
            }
            v
        }
    }
}

/// What the low-disk guard should do for a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiskAction {
    /// Pause the download: free space is below the threshold.
    Hold,
    /// Resume a download held by this guard: space has recovered.
    Release,
    NoChange,
}

/// Decide the low-disk action from free space, the threshold and whether the
/// torrent is already held by the guard.
///
/// Holding starts below `min_free`. Release waits until free space reaches
/// twice `min_free`, so a download does not flap on and off at the threshold.
pub(crate) fn disk_guard_action(free: u64, min_free: u64, held: bool) -> DiskAction {
    if !held && free < min_free {
        DiskAction::Hold
    } else if held && free >= min_free.saturating_mul(2) {
        DiskAction::Release
    } else {
        DiskAction::NoChange
    }
}

/// Free bytes available to unprivileged users on the filesystem holding `path`.
/// Walks up to the nearest existing ancestor, since a torrent's folder may not
/// exist yet. Returns `None` where it cannot be measured.
#[cfg(unix)]
pub(crate) fn free_disk_space(path: &Path) -> Option<u64> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    let stat = nix::sys::statvfs::statvfs(probe).ok()?;
    Some(stat.blocks_available() as u64 * stat.fragment_size() as u64)
}

#[cfg(not(unix))]
pub(crate) fn free_disk_space(_path: &Path) -> Option<u64> {
    None
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Session {
    /// Set the share limits for one torrent. Enforced by the next policy pass, and
    /// saved right away so they survive a restart.
    pub async fn set_share_limits(&self, handle: &ManagedTorrentHandle, limits: ShareLimits) {
        *handle.shared().share_limits.write() = limits;
        self.try_update_persistence_metadata(handle).await;
    }

    /// Get the share limits for one torrent.
    pub fn share_limits(&self, handle: &ManagedTorrentHandle) -> ShareLimits {
        *handle.shared().share_limits.read()
    }

    /// Global seed limits. Applied to torrents that have no limits of their own.
    pub fn seed_limits(&self) -> ShareLimits {
        *self.seed_limits.read()
    }

    /// Set the global seed limits and save them with the session settings.
    /// The next policy pass applies them to torrents without their own limits.
    pub async fn set_seed_limits(&self, limits: ShareLimits) -> anyhow::Result<()> {
        *self.seed_limits.write() = limits;
        self.persist_session_settings().await
    }

    /// Maximum concurrently downloading torrents. `None` means unlimited.
    pub fn max_active_downloads(&self) -> Option<usize> {
        decode_limit(self.max_active_downloads.load(Ordering::Relaxed))
    }

    pub fn set_max_active_downloads(&self, limit: Option<usize>) {
        self.max_active_downloads
            .store(encode_limit(limit), Ordering::Relaxed);
    }

    /// Maximum concurrently seeding torrents. `None` means unlimited.
    pub fn max_active_seeds(&self) -> Option<usize> {
        decode_limit(self.max_active_seeds.load(Ordering::Relaxed))
    }

    pub fn set_max_active_seeds(&self, limit: Option<usize>) {
        self.max_active_seeds
            .store(encode_limit(limit), Ordering::Relaxed);
    }

    /// Set both queue limits and save them with the session settings.
    pub async fn set_queue_limits(
        &self,
        max_active_downloads: Option<usize>,
        max_active_seeds: Option<usize>,
    ) -> anyhow::Result<()> {
        self.set_max_active_downloads(max_active_downloads);
        self.set_max_active_seeds(max_active_seeds);
        self.persist_session_settings().await
    }

    /// True when either queue limit is set.
    pub fn queueing_enabled(&self) -> bool {
        self.max_active_downloads().is_some() || self.max_active_seeds().is_some()
    }

    /// Every torrent id, front of the queue first.
    pub fn queue_order(&self) -> Vec<TorrentId> {
        let mut keyed: Vec<QueueKey> =
            self.with_torrents(|it| it.map(|(id, h)| (h.queue_position(), id)).collect());
        keyed.sort_unstable();
        keyed.into_iter().map(|(_, id)| id).collect()
    }

    /// Move torrents within the queue. Positions are renumbered densely, and
    /// changed ones are saved. The next policy pass applies the new order.
    pub async fn move_in_queue(&self, ids: &[TorrentId], mv: QueueMove) {
        let selected = ids.iter().copied().collect();
        let order = reorder_queue(&self.queue_order(), &selected, mv);
        for (position, id) in order.into_iter().enumerate() {
            let Some(handle) = self.get(id.into()) else {
                continue;
            };
            let previous = handle
                .shared()
                .queue_position
                .swap(position as i64, Ordering::Relaxed);
            if previous != position as i64 {
                self.try_update_persistence_metadata(&handle).await;
            }
        }
    }

    /// Turn force-start on or off. Turning it on starts the torrent at once,
    /// outside the queue limits; turning it off hands it back to the queue.
    pub async fn set_force_start(
        self: &Arc<Self>,
        handle: &ManagedTorrentHandle,
        enabled: bool,
    ) -> anyhow::Result<()> {
        handle
            .shared()
            .force_start
            .store(enabled, Ordering::Relaxed);
        if enabled && (handle.is_paused() || handle.is_queued()) {
            // unpause clears the policy flags and saves the torrent.
            return self.unpause(handle).await;
        }
        self.try_update_persistence_metadata(handle).await;
        Ok(())
    }

    /// Free-space threshold in bytes below which downloads are held. `None`
    /// disables the guard.
    pub fn min_free_disk_space(&self) -> Option<u64> {
        match self.min_free_disk_space.load(Ordering::Relaxed) {
            0 => None,
            bytes => Some(bytes),
        }
    }

    pub fn set_min_free_disk_space(&self, bytes: Option<u64>) {
        self.min_free_disk_space
            .store(bytes.unwrap_or(0), Ordering::Relaxed);
    }

    pub(crate) fn spawn_policy_scheduler(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        self.spawn(
            debug_span!(parent: self.rs(), "policy_scheduler"),
            "policy_scheduler",
            async move {
                loop {
                    tokio::time::sleep(POLICY_INTERVAL).await;
                    let session = match weak.upgrade() {
                        Some(s) => s,
                        None => return Ok(()),
                    };
                    session.policy_pass().await;
                }
            },
        );
    }

    /// Pause a torrent on the session's behalf and record why, so the policy
    /// pass knows whether it may resume it or should leave it alone.
    async fn policy_pause(&self, handle: &ManagedTorrentHandle, reason: PolicyPause) {
        if let Err(e) = handle.pause() {
            debug!(id = handle.id(), "policy pause skipped: {e:#}");
            return;
        }
        PolicyFlags::for_pause(reason).store(handle.shared());
        if reason == PolicyPause::ShareLimit {
            handle.shared().force_start.store(false, Ordering::Relaxed);
        }
        self.try_update_persistence_metadata(handle).await;
    }

    /// Run one policy pass: share limits, then the low-disk guard, then the queues.
    pub(crate) async fn policy_pass(self: &Arc<Self>) {
        let handles: Vec<ManagedTorrentHandle> =
            self.with_torrents(|it| it.map(|(_, h)| h.clone()).collect());
        let now = unix_now();

        // Downloads and seeds that are running or waiting for a slot, per kind.
        let mut running_dl = Vec::new();
        let mut running_seed = Vec::new();
        let mut waiting_dl = Vec::new();
        let mut waiting_seed = Vec::new();
        let mut downloads_by_id = std::collections::HashMap::new();
        let mut seeds_by_id = std::collections::HashMap::new();
        // Torrents the low-disk guard may hold or release, with their downloading state.
        let mut disk_candidates = Vec::new();

        for handle in &handles {
            let stats = handle.stats();
            let id = handle.id();
            let is_live_or_init = matches!(
                stats.state,
                crate::torrent_state::stats::TorrentStatsState::Live
                    | crate::torrent_state::stats::TorrentStatsState::Initializing
            );
            let held = handle.shared().held_for_disk.load(Ordering::Relaxed);
            if held || (is_live_or_init && !stats.finished && !handle.is_paused()) {
                disk_candidates.push((handle.clone(), held));
            }

            // Only a live, unpaused seed can hit a share limit.
            if matches!(
                stats.state,
                crate::torrent_state::stats::TorrentStatsState::Live
            ) && stats.finished
                && !handle.is_paused()
            {
                let own = *handle.shared().share_limits.read();
                let limits = effective_share_limits(own, *self.seed_limits.read());
                let completed_at = handle.shared().completion_on.load(Ordering::Relaxed);
                let seeding_secs = if completed_at > 0 {
                    now.saturating_sub(completed_at)
                } else {
                    0
                };
                let inactive_secs = inactive_seconds(
                    &mut handle.shared().upload_activity.lock(),
                    stats.uploaded_bytes,
                    now,
                );
                if let Some(reason) = share_limit_reached(
                    &limits,
                    stats.uploaded_bytes,
                    stats.total_bytes,
                    seeding_secs,
                    inactive_secs,
                ) {
                    info!(id, ?reason, "share limit reached, pausing torrent");
                    self.policy_pause(handle, PolicyPause::ShareLimit).await;
                    continue;
                }
            }

            if handle.shared().held_for_disk.load(Ordering::Relaxed) {
                // Handled by the low-disk guard below; not part of the queues.
                continue;
            }

            // A force-started torrent runs outside the queue and takes no slot.
            if handle.is_force_started() {
                continue;
            }

            let queued = handle.shared().queued.load(Ordering::Relaxed);
            let is_seed = stats.finished;
            let is_running = !handle.is_paused() && is_live_or_init;
            let bucket = match (is_seed, queued, is_running) {
                (true, true, _) => &mut waiting_seed,
                (false, true, _) => &mut waiting_dl,
                (true, false, true) => &mut running_seed,
                (false, false, true) => &mut running_dl,
                _ => continue,
            };
            bucket.push((handle.queue_position(), id));
            if is_seed {
                seeds_by_id.insert(id, handle.clone());
            } else {
                downloads_by_id.insert(id, handle.clone());
            }
        }

        if let Some(min_free) = self.min_free_disk_space() {
            for (handle, held) in &disk_candidates {
                let Some(free) = free_disk_space(&handle.output_folder()) else {
                    continue;
                };
                match disk_guard_action(free, min_free, *held) {
                    DiskAction::Hold => {
                        warn!(
                            id = handle.id(),
                            free, min_free, "low disk space, holding download"
                        );
                        self.policy_pause(handle, PolicyPause::DiskHold).await;
                    }
                    DiskAction::Release => {
                        info!(
                            id = handle.id(),
                            free, "disk space recovered, resuming download"
                        );
                        if let Err(e) = self.unpause(handle).await {
                            warn!(id = handle.id(), "error resuming held download: {e:#}");
                        }
                    }
                    DiskAction::NoChange => {}
                }
            }
        }

        let dl_plan = plan_queue(&running_dl, &waiting_dl, self.max_active_downloads());
        let seed_plan = plan_queue(&running_seed, &waiting_seed, self.max_active_seeds());
        for (plan, by_id) in [(dl_plan, &downloads_by_id), (seed_plan, &seeds_by_id)] {
            for id in plan.demote {
                if let Some(handle) = by_id.get(&id) {
                    debug!(id, "queue full, moving torrent back to the queue");
                    self.policy_pause(handle, PolicyPause::Queued).await;
                }
            }
            for id in plan.promote {
                if let Some(handle) = by_id.get(&id) {
                    debug!(id, "queue slot free, starting queued torrent");
                    if let Err(e) = self.unpause(handle).await {
                        warn!(id, "error starting queued torrent: {e:#}");
                    }
                }
            }
        }
    }
}

pub(crate) fn encode_limit(limit: Option<usize>) -> usize {
    // 0 is never a useful limit, so it stands for "unlimited".
    limit.filter(|&n| n > 0).unwrap_or(0)
}

fn decode_limit(raw: usize) -> Option<usize> {
    (raw > 0).then_some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(ratio: Option<f64>, secs: Option<u64>) -> ShareLimits {
        ShareLimits {
            max_ratio: ratio,
            max_seeding_time: secs.map(Duration::from_secs),
            max_inactive_seeding_time: None,
        }
    }

    #[test]
    fn global_share_limits_apply_only_without_per_torrent_limits() {
        let global = limits(Some(2.0), Some(3600));
        assert_eq!(
            effective_share_limits(ShareLimits::default(), global),
            global
        );

        let own = limits(Some(1.5), None);
        assert_eq!(effective_share_limits(own, global), own);
    }

    #[test]
    fn no_limits_never_triggers() {
        assert_eq!(
            share_limit_reached(&ShareLimits::default(), u64::MAX, 1, u64::MAX, 0),
            None
        );
    }

    #[test]
    fn ratio_triggers_at_threshold() {
        let l = limits(Some(1.5), None);
        assert_eq!(share_limit_reached(&l, 149, 100, 0, 0), None);
        assert_eq!(
            share_limit_reached(&l, 150, 100, 0, 0),
            Some(ShareLimitReason::Ratio)
        );
    }

    #[test]
    fn ratio_ignores_zero_size_torrent() {
        assert_eq!(
            share_limit_reached(&limits(Some(0.0), None), 10, 0, 0, 0),
            None
        );
    }

    #[test]
    fn seeding_time_triggers_and_unknown_completion_does_not() {
        let l = limits(None, Some(3600));
        assert_eq!(share_limit_reached(&l, 0, 100, 3599, 0), None);
        assert_eq!(
            share_limit_reached(&l, 0, 100, 3600, 0),
            Some(ShareLimitReason::SeedingTime)
        );
        assert_eq!(share_limit_reached(&l, 0, 100, 0, 0), None);
    }

    #[test]
    fn inactive_seeding_time_triggers_and_zero_does_not() {
        let l = ShareLimits {
            max_inactive_seeding_time: Some(Duration::from_secs(600)),
            ..Default::default()
        };
        assert_eq!(share_limit_reached(&l, 0, 100, 5000, 599), None);
        assert_eq!(
            share_limit_reached(&l, 0, 100, 5000, 600),
            Some(ShareLimitReason::InactiveSeedingTime)
        );
        assert_eq!(share_limit_reached(&l, 0, 100, 5000, 0), None);
    }

    #[test]
    fn inactivity_clock_resets_when_uploads_move() {
        let mut activity = (0, 0);
        assert_eq!(inactive_seconds(&mut activity, 10, 1000), 0);
        assert_eq!(inactive_seconds(&mut activity, 10, 1300), 300);
        assert_eq!(inactive_seconds(&mut activity, 11, 1400), 0);
        assert_eq!(inactive_seconds(&mut activity, 11, 1450), 50);
    }

    /// Keys whose position is the id, as for torrents never moved in the queue.
    fn keys(ids: &[TorrentId]) -> Vec<QueueKey> {
        ids.iter().map(|&id| (id as i64, id)).collect()
    }

    #[test]
    fn plan_queue_unlimited_promotes_everything_waiting() {
        let plan = plan_queue(&keys(&[3, 1]), &keys(&[5, 4]), None);
        assert_eq!(plan.demote, Vec::<TorrentId>::new());
        assert_eq!(plan.promote, vec![4, 5]);
    }

    #[test]
    fn plan_queue_keeps_oldest_running_and_demotes_newest() {
        let plan = plan_queue(&keys(&[9, 2, 5]), &[], Some(2));
        assert_eq!(plan.demote, vec![9]);
        assert!(plan.promote.is_empty());
    }

    #[test]
    fn plan_queue_fills_free_slots_in_id_order() {
        let plan = plan_queue(&keys(&[1]), &keys(&[7, 3, 8]), Some(3));
        assert_eq!(plan.demote, Vec::<TorrentId>::new());
        assert_eq!(plan.promote, vec![3, 7]);
    }

    #[test]
    fn plan_queue_full_leaves_waiting_alone() {
        let plan = plan_queue(&keys(&[1, 2]), &keys(&[3]), Some(2));
        assert_eq!(plan, QueuePlan::default());
    }

    #[test]
    fn plan_queue_zero_limit_holds_everything() {
        let plan = plan_queue(&keys(&[1]), &keys(&[2]), Some(0));
        assert_eq!(plan.demote, vec![1]);
        assert!(plan.promote.is_empty());
    }

    #[test]
    fn plan_queue_follows_positions_not_ids() {
        // Torrent 9 was moved to the front; it takes the slot from torrent 1.
        let plan = plan_queue(&[(1, 1)], &[(-1, 9)], Some(1));
        assert_eq!(plan.promote, vec![9]);
        assert_eq!(plan.demote, vec![1]);
    }

    #[test]
    fn reorder_queue_moves() {
        let order = [1, 2, 3, 4, 5];
        let sel = |ids: &[TorrentId]| ids.iter().copied().collect();
        assert_eq!(
            reorder_queue(&order, &sel(&[4, 2]), QueueMove::Top),
            vec![2, 4, 1, 3, 5]
        );
        assert_eq!(
            reorder_queue(&order, &sel(&[1, 3]), QueueMove::Bottom),
            vec![2, 4, 5, 1, 3]
        );
        assert_eq!(
            reorder_queue(&order, &sel(&[1, 3]), QueueMove::Up),
            vec![1, 3, 2, 4, 5]
        );
        assert_eq!(
            reorder_queue(&order, &sel(&[3, 5]), QueueMove::Down),
            vec![1, 2, 4, 3, 5]
        );
        assert_eq!(
            reorder_queue(&order, &sel(&[4, 5]), QueueMove::Up),
            vec![1, 2, 4, 5, 3]
        );
    }

    #[test]
    fn disk_guard_holds_below_threshold_only() {
        assert_eq!(disk_guard_action(99, 100, false), DiskAction::Hold);
        assert_eq!(disk_guard_action(100, 100, false), DiskAction::NoChange);
    }

    #[test]
    fn disk_guard_releases_only_after_hysteresis_margin() {
        assert_eq!(disk_guard_action(150, 100, true), DiskAction::NoChange);
        assert_eq!(disk_guard_action(200, 100, true), DiskAction::Release);
    }

    #[test]
    fn limit_encoding_round_trips() {
        assert_eq!(decode_limit(encode_limit(None)), None);
        assert_eq!(decode_limit(encode_limit(Some(0))), None);
        assert_eq!(decode_limit(encode_limit(Some(4))), Some(4));
    }

    #[cfg(unix)]
    #[test]
    fn free_space_is_reported_for_missing_child_of_existing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let free = free_disk_space(&dir.path().join("not/yet/created"));
        // A full disk reports 0; the test is that the value is reported, not how much.
        assert!(free.is_some());
    }
}

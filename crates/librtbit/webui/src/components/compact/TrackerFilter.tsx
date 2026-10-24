import { useContext, useEffect, useMemo, useState } from "react";
import {
  BsBroadcast,
  BsBroadcastPin,
  BsExclamationTriangle,
  BsXOctagon,
} from "react-icons/bs";
import { APIContext } from "../../context";
import { useUIStore } from "../../stores/uiStore";
import { useTorrentStore } from "../../stores/torrentStore";
import {
  TrackerHealth,
  useTrackerHealthStore,
} from "../../stores/trackerHealthStore";
import {
  TRACKER_FILTER_ERROR,
  TRACKER_FILTER_WARNING,
  torrentTrackerHosts,
} from "../../helper/torrentFilters";

/** How often tracker health is re-checked. The list API has no tracker
 * state, so this costs one status request per torrent with trackers. */
const HEALTH_POLL_MS = 30_000;

/** Sidebar section: filter torrents by tracker host, qBittorrent-style. */
export const TrackerFilter: React.FC = () => {
  const torrents = useTorrentStore((state) => state.torrents);
  const trackerFilter = useUIStore((state) => state.trackerFilter);
  const setTrackerFilter = useUIStore((state) => state.setTrackerFilter);
  const API = useContext(APIContext);
  const health = useTrackerHealthStore((state) => state.health);
  const setHealth = useTrackerHealthStore((state) => state.setHealth);
  const [healthChecked, setHealthChecked] = useState(false);

  // Ids of torrents with trackers; a stable string so polling doesn't restart
  // on every list refresh.
  const trackedIds = useMemo(
    () =>
      (torrents ?? [])
        .filter((t) => (t.trackers?.length ?? 0) > 0)
        .map((t) => t.id)
        .join(","),
    [torrents],
  );

  useEffect(() => {
    const ids = trackedIds ? trackedIds.split(",").map(Number) : [];
    let cancelled = false;
    const check = async () => {
      const next: Record<number, TrackerHealth> = {};
      await Promise.all(
        ids.map(async (id) => {
          try {
            const { trackers } = await API.getTrackerStatus(id);
            const failing = trackers.filter((tr) => tr.state === "error");
            if (failing.length === 0) return;
            next[id] = failing.length === trackers.length ? "error" : "warning";
          } catch {
            // Torrent removed or status unavailable: treat as healthy.
          }
        }),
      );
      if (!cancelled) {
        setHealth(next);
        setHealthChecked(true);
      }
    };
    check();
    const timer = setInterval(check, HEALTH_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [API, trackedIds, setHealth]);

  const healthCounts = useMemo(() => {
    let error = 0;
    let warning = 0;
    for (const h of Object.values(health)) {
      if (h === "error") error++;
      else warning++;
    }
    return { error, warning };
  }, [health]);

  // A health group with no torrents has no sidebar item, so a selected (or
  // persisted) one would leave an empty list with nothing selected: fall back
  // to "All" once health has been checked.
  useEffect(() => {
    if (!healthChecked) return;
    if (
      (trackerFilter === TRACKER_FILTER_ERROR && healthCounts.error === 0) ||
      (trackerFilter === TRACKER_FILTER_WARNING && healthCounts.warning === 0)
    ) {
      setTrackerFilter(null);
    }
  }, [healthChecked, trackerFilter, healthCounts, setTrackerFilter]);

  const trackerCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    let trackerless = 0;
    let total = 0;
    if (torrents) {
      for (const t of torrents) {
        total++;
        const hosts = torrentTrackerHosts(t);
        if (hosts.length === 0) trackerless++;
        for (const host of hosts) {
          counts[host] = (counts[host] || 0) + 1;
        }
      }
    }
    return { counts, trackerless, total };
  }, [torrents]);

  const hosts = useMemo(
    () => Object.keys(trackerCounts.counts).sort((a, b) => a.localeCompare(b)),
    [trackerCounts.counts],
  );

  const activeItemClass = "bg-primary/10 text-primary font-medium";
  const inactiveItemClass =
    "text-secondary hover:bg-surface-sunken hover:text-primary";
  const iconClass = "w-3.5 h-3.5 shrink-0";

  const item = (
    key: string | null,
    label: string,
    count: number,
    icon: React.ReactNode,
  ) => (
    <button
      key={key ?? "__all__"}
      onClick={() => setTrackerFilter(key)}
      aria-pressed={trackerFilter === key}
      aria-label={`${label}, ${count} ${count === 1 ? "torrent" : "torrents"}`}
      className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded text-sm cursor-pointer transition-colors focus-ring ${
        trackerFilter === key ? activeItemClass : inactiveItemClass
      }`}
    >
      {icon}
      <span className="flex-1 text-left truncate" title={label}>
        {label}
      </span>
      <span
        className={`text-xs tabular-nums ${
          trackerFilter === key ? "text-primary" : "text-tertiary"
        }`}
      >
        {count}
      </span>
    </button>
  );

  // Hide the whole section when the server doesn't report trackers
  if (hosts.length === 0 && trackerCounts.trackerless === 0) {
    return null;
  }

  return (
    <div>
      <div className="px-3 pt-3 pb-1">
        <h2 className="text-xs font-semibold text-tertiary uppercase tracking-wider">
          Trackers
        </h2>
      </div>
      <div className="px-1.5 pb-2">
        {item(
          null,
          "All",
          trackerCounts.total,
          <BsBroadcast className={iconClass} />,
        )}
        {trackerCounts.trackerless > 0 &&
          item(
            "",
            "Trackerless",
            trackerCounts.trackerless,
            <BsBroadcast className={iconClass} />,
          )}
        {healthCounts.error > 0 &&
          item(
            TRACKER_FILTER_ERROR,
            "Tracker error",
            healthCounts.error,
            <BsXOctagon className={`${iconClass} text-error`} />,
          )}
        {healthCounts.warning > 0 &&
          item(
            TRACKER_FILTER_WARNING,
            "Tracker warning",
            healthCounts.warning,
            <BsExclamationTriangle className={`${iconClass} text-warning`} />,
          )}
        {hosts.map((host) =>
          item(
            host,
            host,
            trackerCounts.counts[host],
            <BsBroadcastPin className={iconClass} />,
          ),
        )}
      </div>
    </div>
  );
};

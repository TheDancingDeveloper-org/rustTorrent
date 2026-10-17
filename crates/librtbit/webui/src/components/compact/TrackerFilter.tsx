import { useMemo } from "react";
import { BsBroadcast, BsBroadcastPin } from "react-icons/bs";
import { useUIStore } from "../../stores/uiStore";
import { useTorrentStore } from "../../stores/torrentStore";
import { torrentTrackerHosts } from "../../helper/torrentFilters";

/** Sidebar section: filter torrents by tracker host, qBittorrent-style. */
export const TrackerFilter: React.FC = () => {
  const torrents = useTorrentStore((state) => state.torrents);
  const trackerFilter = useUIStore((state) => state.trackerFilter);
  const setTrackerFilter = useUIStore((state) => state.setTrackerFilter);

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

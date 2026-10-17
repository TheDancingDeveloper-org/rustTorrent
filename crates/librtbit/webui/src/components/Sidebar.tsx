import { useMemo } from "react";
import {
  FaDownload,
  FaUpload,
  FaPause,
  FaExclamationTriangle,
} from "react-icons/fa";
import { BsChevronLeft, BsChevronRight, BsCollection } from "react-icons/bs";
import { MdQueue } from "react-icons/md";
import { useUIStore, CurrentPage } from "../stores/uiStore";
import { useTorrentStore } from "../stores/torrentStore";
import { StatusFilter } from "../helper/torrentFilters";
import { CategoryFilter } from "./compact/CategoryFilter";
import { TrackerFilter } from "./compact/TrackerFilter";

interface FilterItem {
  key: StatusFilter;
  label: string;
  icon: React.ReactNode;
  count: number;
}

/** `drawer` fills the mobile navigation drawer and has no collapse control. */
export const Sidebar: React.FC<{ variant?: "default" | "drawer" }> = ({
  variant = "default",
}) => {
  const torrents = useTorrentStore((state) => state.torrents);
  const statusFilter = useUIStore((state) => state.statusFilter);
  const setStatusFilter = useUIStore((state) => state.setStatusFilter);
  const setCurrentPage = useUIStore((state) => state.setCurrentPage);
  const sidebarCollapsed = useUIStore((state) => state.sidebarCollapsed);
  const toggleSidebar = useUIStore((state) => state.toggleSidebar);

  const handleFilterClick = (key: StatusFilter) => {
    setStatusFilter(key);
    setCurrentPage("torrents");
  };

  const statusCounts = useMemo(() => {
    if (!torrents)
      return {
        all: 0,
        downloading: 0,
        seeding: 0,
        paused: 0,
        queued: 0,
        initializing: 0,
        error: 0,
      };
    return {
      all: torrents.length,
      downloading: torrents.filter(
        (t) => t.stats?.state === "live" && !t.stats?.finished,
      ).length,
      seeding: torrents.filter(
        (t) => t.stats?.state === "live" && t.stats?.finished,
      ).length,
      paused: torrents.filter((t) => t.stats?.state === "paused").length,
      queued: torrents.filter((t) => t.stats?.queue_state === "Queued").length,
      initializing: torrents.filter((t) => t.stats?.state === "initializing")
        .length,
      error: torrents.filter((t) => t.stats?.state === "error").length,
    };
  }, [torrents]);

  const iconClass = "w-3.5 h-3.5 shrink-0";

  const filters: FilterItem[] = [
    {
      key: "all",
      label: "All",
      icon: <BsCollection className={iconClass} />,
      count: statusCounts.all,
    },
    {
      key: "downloading",
      label: "Downloading",
      icon: <FaDownload className={iconClass} />,
      count: statusCounts.downloading,
    },
    {
      key: "seeding",
      label: "Seeding",
      icon: <FaUpload className={iconClass} />,
      count: statusCounts.seeding,
    },
    {
      key: "paused",
      label: "Paused",
      icon: <FaPause className={iconClass} />,
      count: statusCounts.paused,
    },
    {
      key: "queued",
      label: "Queued",
      icon: <MdQueue className={iconClass} />,
      count: statusCounts.queued,
    },
    {
      key: "error",
      label: "Error",
      icon: <FaExclamationTriangle className={iconClass} />,
      count: statusCounts.error,
    },
  ];

  const activeItemClass = "bg-primary/10 text-primary font-medium";
  const inactiveItemClass =
    "text-secondary hover:bg-surface-sunken hover:text-primary";

  if (sidebarCollapsed && variant === "default") {
    return (
      <div className="w-12 bg-surface border-r border-divider flex flex-col shrink-0">
        <div className="flex-1 pt-2">
          {filters.map((f) => (
            <button
              key={f.key}
              onClick={() => handleFilterClick(f.key)}
              title={`${f.label} (${f.count})`}
              aria-label={`${f.label}, ${f.count} ${f.count === 1 ? "torrent" : "torrents"}`}
              aria-pressed={statusFilter === f.key}
              className={`w-full flex items-center justify-center py-2.5 cursor-pointer transition-colors focus-ring ${
                statusFilter === f.key ? activeItemClass : inactiveItemClass
              }`}
            >
              {f.icon}
            </button>
          ))}
        </div>
        <button
          onClick={toggleSidebar}
          className="p-2 text-tertiary hover:text-secondary cursor-pointer border-t border-divider flex items-center justify-center"
          title="Expand sidebar"
        >
          <BsChevronRight className="w-3.5 h-3.5" />
        </button>
      </div>
    );
  }

  const rootClass =
    variant === "drawer"
      ? "w-full h-full bg-surface flex flex-col"
      : "w-48 bg-surface border-r border-divider flex flex-col shrink-0";

  return (
    <div className={rootClass}>
      <div className="flex-1 overflow-y-auto">
        {/* Status section */}
        <div className="px-3 pt-3 pb-1">
          <h2 className="text-xs font-semibold text-tertiary uppercase tracking-wider">
            Status
          </h2>
        </div>
        <div className="px-1.5">
          {filters.map((f) => (
            <button
              key={f.key}
              onClick={() => handleFilterClick(f.key)}
              aria-pressed={statusFilter === f.key}
              aria-label={`${f.label}, ${f.count} ${f.count === 1 ? "torrent" : "torrents"}`}
              className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded text-sm cursor-pointer transition-colors focus-ring ${
                statusFilter === f.key ? activeItemClass : inactiveItemClass
              }`}
            >
              {f.icon}
              <span className="flex-1 text-left">{f.label}</span>
              <span
                className={`text-xs tabular-nums ${
                  statusFilter === f.key ? "text-primary" : "text-tertiary"
                }`}
              >
                {f.count}
              </span>
            </button>
          ))}
        </div>

        {/* Categories section */}
        <CategoryFilter />

        {/* Trackers section */}
        <TrackerFilter />
      </div>
      {variant === "default" && (
        <button
          onClick={toggleSidebar}
          className="p-2 text-tertiary hover:text-secondary cursor-pointer border-t border-divider flex items-center justify-center gap-1 text-xs focus-ring"
          title="Collapse sidebar"
        >
          <BsChevronLeft className="w-3 h-3" />
          <span>Collapse</span>
        </button>
      )}
    </div>
  );
};

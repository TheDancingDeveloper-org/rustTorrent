// Shared torrent filtering and sorting utilities

import type { TorrentListItem } from "../api-types";
import { useTrackerHealthStore } from "../stores/trackerHealthStore";
import { StatusFilter, matchesStatus } from "./statusFilter";

export type { StatusFilter } from "./statusFilter";
export {
  STATUS_FILTERS,
  STATUS_FILTER_LABELS,
  matchesStatus,
} from "./statusFilter";

// Sort types
export type TorrentSortColumn =
  | "id"
  | "name"
  | "size"
  | "progress"
  | "downSpeed"
  | "upSpeed"
  | "eta"
  | "category";

export type SortDirection = "asc" | "desc";

// Sort column display labels
export const SORT_COLUMN_LABELS: Record<TorrentSortColumn, string> = {
  id: "ID",
  name: "Name",
  size: "Size",
  progress: "Progress",
  downSpeed: "Down Speed",
  upSpeed: "Up Speed",
  eta: "ETA",
  category: "Category",
};

// Get sort value for a torrent
export function getSortValue(
  t: TorrentListItem,
  column: TorrentSortColumn,
): number | string {
  switch (column) {
    case "id":
      return t.id;
    case "name":
      return (t.name ?? "").toLowerCase();
    case "size":
      return t.stats?.total_bytes ?? 0;
    case "progress":
      return t.stats?.total_bytes
        ? (t.stats.progress_bytes ?? 0) / t.stats.total_bytes
        : 0;
    case "downSpeed":
      return t.stats?.live?.download_speed?.mbps ?? 0;
    case "upSpeed":
      return t.stats?.live?.upload_speed?.mbps ?? 0;
    case "eta": {
      if (!t.stats?.live) return Infinity;
      const remaining =
        (t.stats.total_bytes ?? 0) - (t.stats.progress_bytes ?? 0);
      const speed = t.stats.live.download_speed?.mbps ?? 0;
      if (speed <= 0 || remaining <= 0) return remaining <= 0 ? 0 : Infinity;
      return remaining / (speed * 1024 * 1024);
    }
    case "category":
      return (t.category ?? "").toLowerCase();
  }
}

// Compare function for sorting
export function compareTorrents(
  a: TorrentListItem,
  b: TorrentListItem,
  column: TorrentSortColumn,
  direction: SortDirection,
): number {
  const aVal = getSortValue(a, column);
  const bVal = getSortValue(b, column);
  const cmp =
    typeof aVal === "string"
      ? aVal.localeCompare(bVal as string)
      : (aVal as number) - (bVal as number);
  return direction === "asc" ? cmp : -cmp;
}

// Check if torrent matches search query
export function matchesSearch(name: string | null, query: string): boolean {
  if (!query) return true;
  return (name ?? "").toLowerCase().includes(query);
}

// Extract the host of a tracker announce URL ("udp://tr.example.com:80/ann" -> "tr.example.com")
export function trackerHost(url: string): string | null {
  const m = /^[a-z+]+:\/\/([^/:?#]+)/i.exec(url);
  return m ? m[1].toLowerCase() : null;
}

// Unique tracker hosts for a torrent
export function torrentTrackerHosts(t: TorrentListItem): string[] {
  const hosts = new Set<string>();
  for (const url of t.trackers ?? []) {
    const host = trackerHost(url);
    if (host) hosts.add(host);
  }
  return Array.from(hosts);
}

// Pseudo tracker filters for the tracker health groups (":" never appears in
// a host returned by trackerHost)
export const TRACKER_FILTER_ERROR = ":error";
export const TRACKER_FILTER_WARNING = ":warning";

// Check if torrent matches tracker filter (a tracker host, "" = trackerless,
// or one of the health groups above)
export function matchesTracker(
  t: TorrentListItem,
  trackerFilter: string | null,
): boolean {
  if (trackerFilter === null) return true;
  if (trackerFilter === TRACKER_FILTER_ERROR)
    return useTrackerHealthStore.getState().health[t.id] === "error";
  if (trackerFilter === TRACKER_FILTER_WARNING)
    return useTrackerHealthStore.getState().health[t.id] === "warning";
  const hosts = torrentTrackerHosts(t);
  if (trackerFilter === "") return hosts.length === 0;
  return hosts.includes(trackerFilter);
}

// Check if torrent matches category filter
export function matchesCategory(
  torrent: TorrentListItem,
  categoryFilter: string | null,
): boolean {
  if (categoryFilter === null) return true;
  if (categoryFilter === "") return !torrent.category; // uncategorized
  return torrent.category === categoryFilter;
}

// Combined visibility check
export function isTorrentVisible(
  t: TorrentListItem,
  searchQuery: string,
  statusFilter: StatusFilter,
  categoryFilter?: string | null,
  trackerFilter?: string | null,
): boolean {
  return (
    matchesSearch(t.name, searchQuery) &&
    matchesStatus(t, statusFilter) &&
    matchesCategory(t, categoryFilter ?? null) &&
    matchesTracker(t, trackerFilter ?? null)
  );
}

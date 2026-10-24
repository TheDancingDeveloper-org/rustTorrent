// Torrent status filters: kept free of store imports so node tests can load it

import type { TorrentListItem } from "../api-types";

// Status filter types
export type StatusFilter =
  | "all"
  | "downloading"
  | "seeding"
  | "completed"
  | "running"
  | "stopped"
  | "active"
  | "stalled_up"
  | "stalled_down"
  | "checking"
  | "paused"
  | "queued"
  | "error";

// Every status filter, in sidebar order. No "moving" filter: the API does not
// report storage moves in torrent state.
export const STATUS_FILTERS: readonly StatusFilter[] = [
  "all",
  "downloading",
  "seeding",
  "completed",
  "running",
  "stopped",
  "active",
  "stalled_up",
  "stalled_down",
  "checking",
  "paused",
  "queued",
  "error",
];

// Status filter display labels
export const STATUS_FILTER_LABELS: Record<StatusFilter, string> = {
  all: "All",
  downloading: "Downloading",
  seeding: "Seeding",
  completed: "Completed",
  running: "Running",
  stopped: "Stopped",
  active: "Active",
  stalled_up: "Stalled uploading",
  stalled_down: "Stalled downloading",
  checking: "Checking",
  paused: "Paused",
  queued: "Queued",
  error: "Error",
};

// Check if torrent matches status filter
export function matchesStatus(
  t: TorrentListItem,
  filter: StatusFilter,
): boolean {
  if (filter === "all") return true;

  const state = t.stats?.state;
  const finished = t.stats?.finished;
  const downMbps = t.stats?.live?.download_speed?.mbps ?? 0;
  const upMbps = t.stats?.live?.upload_speed?.mbps ?? 0;

  switch (filter) {
    case "downloading":
      return state === "live" && !finished;
    case "seeding":
      return state === "live" && !!finished;
    case "completed":
      return !!finished;
    case "running":
      return state === "live";
    case "stopped":
      return state === "paused" || state === "error";
    case "active":
      return state === "live" && (downMbps > 0 || upMbps > 0);
    case "stalled_up":
      return state === "live" && !!finished && upMbps <= 0;
    case "stalled_down":
      return state === "live" && !finished && downMbps <= 0;
    case "checking":
      return state === "initializing";
    case "paused":
      return state === "paused";
    case "queued":
      return t.stats?.queue_state === "Queued";
    case "error":
      return state === "error";
  }
}

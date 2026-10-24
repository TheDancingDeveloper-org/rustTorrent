import { create } from "zustand";

/**
 * Tracker health per torrent id, derived from the per-torrent tracker status
 * endpoint (the torrent list carries announce URLs only):
 * "error" = every tracker is failing, "warning" = some trackers are failing.
 * Torrents without an entry are healthy or not yet checked.
 */
export type TrackerHealth = "error" | "warning";

export interface TrackerHealthState {
  health: Record<number, TrackerHealth>;
  setHealth: (health: Record<number, TrackerHealth>) => void;
}

export const useTrackerHealthStore = create<TrackerHealthState>((set) => ({
  health: {},
  setHealth: (health) => set({ health }),
}));

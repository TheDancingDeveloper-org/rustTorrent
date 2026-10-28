import { create } from "zustand";
import { StatusFilter } from "../helper/torrentFilters";
import { CategoryInfo } from "../api-types";
import {
  loadViewPref,
  saveViewPref,
  isBoolean,
} from "../helper/viewPrefs";
import {
  DEFAULT_VIEW_QUERY,
  parseViewQuery,
  writeViewQuery,
  type ViewQuery,
} from "../helper/viewQuery";

const LARGE_SCREEN_BREAKPOINT = 1024;

const initialView: ViewQuery =
  typeof window === "undefined"
    ? DEFAULT_VIEW_QUERY
    : parseViewQuery(window.location.search);

function publishView(patch: Partial<ViewQuery>): void {
  const state = useUIStore.getState();
  writeViewQuery({
    search: state.searchQuery,
    status: state.statusFilter,
    category: state.categoryFilter,
    tracker: state.trackerFilter,
    tag: state.tagFilter,
    sortColumn: state.sortColumn,
    sortDirection: state.sortDirection,
    ...patch,
  });
}

function getDefaultViewMode(): "full" | "compact" {
  return window.innerWidth >= LARGE_SCREEN_BREAKPOINT ? "compact" : "full";
}

export type CurrentPage = "torrents" | "indexarr" | "rss" | "statistics";

export interface UIStore {
  currentPage: CurrentPage;
  setCurrentPage: (page: CurrentPage) => void;

  viewMode: "full" | "compact";
  setViewMode: (mode: "full" | "compact") => void;
  toggleViewMode: () => void;

  searchQuery: string;
  setSearchQuery: (query: string) => void;

  statusFilter: StatusFilter;
  setStatusFilter: (filter: StatusFilter) => void;

  selectedTorrentIds: Set<number>;
  lastSelectedId: number | null;
  selectTorrent: (id: number) => void;
  toggleSelection: (id: number) => void;
  selectRange: (id: number, orderedIds: number[]) => void;
  deselectTorrent: (id: number) => void;
  clearSelection: () => void;
  selectAll: (ids: number[]) => void;
  selectRelative: (direction: "up" | "down", orderedIds: number[]) => void;

  sidebarCollapsed: boolean;
  sidebarOpen: boolean; // for mobile drawer
  trackerFilter: string | null;
  categoryFilter: string | null; // null = show all, "" = uncategorized, string = filter to category
  tagFilter: string | null; // null = show all, "" = untagged, string = filter to that tag
  toggleSidebar: () => void;
  setSidebarOpen: (open: boolean) => void;
  setTrackerFilter: (tracker: string | null) => void;
  setCategoryFilter: (category: string | null) => void;
  setTagFilter: (tag: string | null) => void;

  sortColumn: ViewQuery["sortColumn"];
  sortDirection: ViewQuery["sortDirection"];
  setSort: (column: ViewQuery["sortColumn"], direction: ViewQuery["sortDirection"]) => void;

  // Category management
  categories: Record<string, CategoryInfo>;
  setCategories: (categories: Record<string, CategoryInfo>) => void;

  detailsModalTorrentId: number | null;
  openDetailsModal: (id: number) => void;
  closeDetailsModal: () => void;

  detailPaneRequestedTab: string | null;
  setDetailPaneTab: (tab: string | null) => void;

  // Keyboard shortcut reference overlay ("?")
  shortcutHelpOpen: boolean;
  setShortcutHelpOpen: (open: boolean) => void;

  // Text for the polite live region. seq changes on every call so a repeated
  // message is still announced.
  announcement: { text: string; seq: number };
  announce: (text: string) => void;
}

export const useUIStore = create<UIStore>((set, get) => ({
  currentPage: "torrents" as CurrentPage,
  setCurrentPage: (page) => set({ currentPage: page }),

  viewMode: getDefaultViewMode(),

  setViewMode: (mode) => {
    set({ viewMode: mode });
  },

  toggleViewMode: () => {
    const newMode = get().viewMode === "compact" ? "full" : "compact";
    set({ viewMode: newMode });
  },

  searchQuery: initialView.search,
  setSearchQuery: (query) => {
    publishView({ search: query });
    set({ searchQuery: query });
  },

  statusFilter: initialView.status,
  setStatusFilter: (filter) => {
    saveViewPref("statusFilter", filter);
    publishView({ status: filter });
    set({ statusFilter: filter });
  },

  selectedTorrentIds: new Set<number>(),
  lastSelectedId: null,

  selectTorrent: (id) => {
    set({ selectedTorrentIds: new Set([id]), lastSelectedId: id });
  },

  toggleSelection: (id) => {
    const current = get().selectedTorrentIds;
    const next = new Set(current);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    set({ selectedTorrentIds: next, lastSelectedId: id });
  },

  selectRange: (id, orderedIds) => {
    const { lastSelectedId, selectedTorrentIds } = get();
    if (lastSelectedId === null) {
      // No anchor, just select this one
      set({ selectedTorrentIds: new Set([id]), lastSelectedId: id });
      return;
    }

    if (selectedTorrentIds.has(id)) {
      const next = new Set(selectedTorrentIds);
      next.delete(id);
      set({ selectedTorrentIds: next });
      return;
    }

    const anchorIdx = orderedIds.indexOf(lastSelectedId);
    const targetIdx = orderedIds.indexOf(id);

    if (anchorIdx === -1 || targetIdx === -1) {
      // Fallback: just select the target
      set({ selectedTorrentIds: new Set([id]), lastSelectedId: id });
      return;
    }

    const startIdx = Math.min(anchorIdx, targetIdx);
    const endIdx = Math.max(anchorIdx, targetIdx);
    const rangeIds = orderedIds.slice(startIdx, endIdx + 1);

    // Extend selection with range
    const next = new Set(selectedTorrentIds);
    for (const rangeId of rangeIds) {
      next.add(rangeId);
    }
    set({ selectedTorrentIds: next });
  },

  deselectTorrent: (id) => {
    const current = get().selectedTorrentIds;
    if (current.has(id)) {
      const next = new Set(current);
      next.delete(id);
      set({ selectedTorrentIds: next });
    }
  },

  clearSelection: () => {
    set({ selectedTorrentIds: new Set(), lastSelectedId: null });
  },

  selectAll: (ids) => {
    set({ selectedTorrentIds: new Set(ids) });
  },

  selectRelative: (direction, orderedIds) => {
    const { selectedTorrentIds, lastSelectedId } = get();
    if (orderedIds.length === 0) return;

    let currentIdx: number;

    if (selectedTorrentIds.size === 0) {
      // Nothing selected, select first or last based on direction
      const newId =
        direction === "down"
          ? orderedIds[0]
          : orderedIds[orderedIds.length - 1];
      set({ selectedTorrentIds: new Set([newId]), lastSelectedId: newId });
      return;
    }

    if (selectedTorrentIds.size === 1) {
      // Single selection, move from that
      const currentId = Array.from(selectedTorrentIds)[0];
      currentIdx = orderedIds.indexOf(currentId);
    } else {
      // Multiple selected, use lastSelectedId if valid, otherwise first selected
      if (lastSelectedId !== null && orderedIds.includes(lastSelectedId)) {
        currentIdx = orderedIds.indexOf(lastSelectedId);
      } else {
        // Find first selected in order
        currentIdx = orderedIds.findIndex((id) => selectedTorrentIds.has(id));
      }
    }

    if (currentIdx === -1) {
      // Fallback: select first
      const newId = orderedIds[0];
      set({ selectedTorrentIds: new Set([newId]), lastSelectedId: newId });
      return;
    }

    const newIdx =
      direction === "down"
        ? Math.min(currentIdx + 1, orderedIds.length - 1)
        : Math.max(currentIdx - 1, 0);

    const newId = orderedIds[newIdx];
    set({ selectedTorrentIds: new Set([newId]), lastSelectedId: newId });
  },

  sidebarCollapsed: loadViewPref("sidebarCollapsed", false, isBoolean),
  sidebarOpen: false,
  trackerFilter: initialView.tracker,
  categoryFilter: initialView.category,
  tagFilter: initialView.tag,
  toggleSidebar: () =>
    set((state) => {
      const sidebarCollapsed = !state.sidebarCollapsed;
      saveViewPref("sidebarCollapsed", sidebarCollapsed);
      return { sidebarCollapsed };
    }),
  setSidebarOpen: (open) => set({ sidebarOpen: open }),
  setTrackerFilter: (tracker) => {
    saveViewPref("trackerFilter", tracker);
    publishView({ tracker });
    set({ trackerFilter: tracker });
  },
  setCategoryFilter: (category) => {
    saveViewPref("categoryFilter", category);
    publishView({ category });
    set({ categoryFilter: category });
  },
  setTagFilter: (tag) => {
    publishView({ tag });
    set({ tagFilter: tag });
  },

  sortColumn: initialView.sortColumn,
  sortDirection: initialView.sortDirection,
  setSort: (column, direction) => {
    publishView({ sortColumn: column, sortDirection: direction });
    set({ sortColumn: column, sortDirection: direction });
  },

  categories: {},
  setCategories: (categories) => set({ categories }),

  detailsModalTorrentId: null,
  openDetailsModal: (id) =>
    set({
      detailsModalTorrentId: id,
      selectedTorrentIds: new Set([id]),
      lastSelectedId: id,
    }),
  closeDetailsModal: () => set({ detailsModalTorrentId: null }),

  detailPaneRequestedTab: null,
  setDetailPaneTab: (tab) => set({ detailPaneRequestedTab: tab }),

  shortcutHelpOpen: false,
  setShortcutHelpOpen: (open) => set({ shortcutHelpOpen: open }),

  announcement: { text: "", seq: 0 },
  announce: (text) =>
    set((state) => ({
      announcement: { text, seq: state.announcement.seq + 1 },
    })),
}));

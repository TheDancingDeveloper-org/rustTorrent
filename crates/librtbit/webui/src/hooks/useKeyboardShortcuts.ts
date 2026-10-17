import { useEffect } from "react";
import { useUIStore } from "../stores/uiStore";
import { useTorrentStore } from "../stores/torrentStore";
import { isTorrentVisible } from "../helper/torrentFilters";

interface KeyboardShortcutActions {
  onDelete?: () => void;
  onToggleSidebar?: () => void;
  onOpenSettings?: () => void;
  onOpenLogs?: () => void;
  onToggleRun?: () => void;
  onOpenDetails?: () => void;
  onOpenFilePicker?: () => void;
  onFocusMagnet?: () => void;
  onToggleHelp?: () => void;
}

// Elements that already act on Enter/Space themselves; leave those keys alone.
const NATIVE_ACTIVATION_TAGS = new Set(["BUTTON", "A", "SUMMARY"]);

/**
 * Hook that sets up keyboard shortcuts for the compact view.
 * Should be called in ActionBar or CompactLayout.
 */
export function useKeyboardShortcuts(actions?: KeyboardShortcutActions) {
  const torrents = useTorrentStore((state) => state.torrents);
  const searchQuery = useUIStore((state) => state.searchQuery);
  const statusFilter = useUIStore((state) => state.statusFilter);
  const categoryFilter = useUIStore((state) => state.categoryFilter);
  const selectAll = useUIStore((state) => state.selectAll);
  const clearSelection = useUIStore((state) => state.clearSelection);
  const selectedTorrentIds = useUIStore((state) => state.selectedTorrentIds);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Ignore if user is typing in an input field
      const target = e.target as HTMLElement;
      if (
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.tagName === "SELECT" ||
        target.isContentEditable
      ) {
        return;
      }

      // Ignore if a modal is open (modals have role="dialog").
      // The shortcut overlay is itself a dialog and closes on its own keys.
      if (document.querySelector('[role="dialog"]')) {
        return;
      }

      const isMod = e.metaKey || e.ctrlKey;

      // Ctrl/Cmd+A: Select all visible torrents
      if (isMod && e.key === "a") {
        e.preventDefault();
        if (torrents) {
          const normalizedQuery = searchQuery.toLowerCase();
          const visibleIds = torrents
            .filter((t) =>
              isTorrentVisible(
                t,
                normalizedQuery,
                statusFilter,
                categoryFilter,
              ),
            )
            .map((t) => t.id);
          selectAll(visibleIds);
        }
        return;
      }

      // Ctrl/Cmd+F: Focus search input
      if (isMod && e.key === "f") {
        e.preventDefault();
        const searchInput = document.querySelector<HTMLInputElement>(
          "[data-search-input]",
        );
        searchInput?.focus();
        searchInput?.select();
        return;
      }

      // The actions below are only bound where the Toolbar supplies them. The
      // hook is mounted in several views at once, and each copy would otherwise
      // run the same action (a toggle would cancel itself out).

      // Ctrl/Cmd+O: Choose a .torrent file
      if (isMod && e.key === "o" && actions?.onOpenFilePicker) {
        e.preventDefault();
        actions.onOpenFilePicker();
        return;
      }

      // M: Focus the magnet / URL input
      if (!isMod && !e.altKey && (e.key === "m" || e.key === "M")) {
        if (actions?.onFocusMagnet) {
          e.preventDefault();
          actions.onFocusMagnet();
        }
        return;
      }

      // ?: Show or hide the shortcut reference
      if (!isMod && !e.altKey && e.key === "?" && actions?.onToggleHelp) {
        e.preventDefault();
        actions.onToggleHelp();
        return;
      }

      // Escape: Clear selection
      if (e.key === "Escape") {
        e.preventDefault();
        clearSelection();
        return;
      }

      // Delete: Open delete modal for selected torrents.
      // Backspace is deliberately not bound, so it cannot delete by accident.
      if (e.key === "Delete" && selectedTorrentIds.size > 0) {
        e.preventDefault();
        actions?.onDelete?.();
        return;
      }

      // Space / Enter act on the whole selection only when nothing focused
      // has claimed the key (a focused row already does, and prevents default).
      if (
        (e.key === " " || e.key === "Enter") &&
        !e.defaultPrevented &&
        !NATIVE_ACTIVATION_TAGS.has(target.tagName) &&
        selectedTorrentIds.size > 0
      ) {
        if (e.key === " " && actions?.onToggleRun) {
          e.preventDefault();
          actions.onToggleRun();
          return;
        }
        if (e.key === "Enter" && selectedTorrentIds.size === 1) {
          if (actions?.onOpenDetails) {
            e.preventDefault();
            actions.onOpenDetails();
          }
          return;
        }
      }

      // B: Toggle sidebar
      if (e.key === "b" || e.key === "B") {
        e.preventDefault();
        actions?.onToggleSidebar?.();
        return;
      }

      // Ctrl/Cmd+, (comma): Open settings
      if (isMod && e.key === ",") {
        e.preventDefault();
        actions?.onOpenSettings?.();
        return;
      }

      // Ctrl/Cmd+L: Open logs
      if (isMod && e.key === "l") {
        e.preventDefault();
        actions?.onOpenLogs?.();
        return;
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    torrents,
    searchQuery,
    statusFilter,
    categoryFilter,
    selectAll,
    clearSelection,
    selectedTorrentIds,
    actions,
  ]);
}

import {
  JSX,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { FaPause, FaPlay, FaTrash } from "react-icons/fa";
import { BsCollection, BsGlobe2, BsRss } from "react-icons/bs";
import { GoSearch, GoX } from "react-icons/go";
import {
  BsBodyText,
  BsBoxArrowRight,
  BsMoon,
  BsSliders2,
  BsSun,
  BsThreeDotsVertical,
} from "react-icons/bs";
import { HiOutlineMenu } from "react-icons/hi";
import debounce from "lodash.debounce";

// @ts-expect-error - SVG import handled by vite-plugin-svgr
import Logo from "../../assets/logo.svg?react";

import { APIContext } from "../context";
import { useUIStore } from "../stores/uiStore";
import { useTorrentStore } from "../stores/torrentStore";
import { useErrorStore } from "../stores/errorStore";
import { useIsLargeScreen } from "../hooks/useIsLargeScreen";
import { useKeyboardShortcuts } from "../hooks/useKeyboardShortcuts";
import { useAnnouncements } from "../hooks/useAnnouncements";
import {
  ErrorDetails,
  STATE_LIVE,
  STATE_PAUSED,
  TorrentListItem,
} from "../api-types";
import { DarkMode } from "../helper/darkMode";
import { useAuthStore } from "../stores/authStore";
import { AuthAPI, IndexarrAPI } from "../http-api";
import { useIndexarrStore } from "../stores/indexarrStore";
import { MagnetInput } from "./buttons/MagnetInput";
import { FileInput } from "./buttons/FileInput";
import { ConfigModal } from "./config/ConfigModal";
import { ShortcutHelp } from "./ShortcutHelp";
import { LiveAnnouncer } from "./LiveAnnouncer";
import { TorrentDetailsModal } from "./modal/TorrentDetailsModal";
import { DeleteTorrentModal } from "./modal/DeleteTorrentModal";
import { KEY_HINT } from "../helper/shortcutList";

interface ToolbarProps {
  title: string;
  version: string;
  onMultiFileSelect?: (files: File[]) => void;
  onLogsClick: () => void;
  menuButtons?: JSX.Element[];
}

export const Toolbar: React.FC<ToolbarProps> = ({
  title,
  version,
  onMultiFileSelect,
  onLogsClick,
  menuButtons,
}) => {
  const API = useContext(APIContext);
  const isLargeScreen = useIsLargeScreen();

  // UI store
  const selectedTorrentIds = useUIStore((state) => state.selectedTorrentIds);
  const searchQuery = useUIStore((state) => state.searchQuery);
  const setSearchQuery = useUIStore((state) => state.setSearchQuery);
  const setSidebarOpen = useUIStore((state) => state.setSidebarOpen);
  const currentPage = useUIStore((s) => s.currentPage);
  const setCurrentPage = useUIStore((s) => s.setCurrentPage);
  const detailsModalTorrentId = useUIStore((s) => s.detailsModalTorrentId);
  const openDetailsModal = useUIStore((s) => s.openDetailsModal);
  const closeDetailsModal = useUIStore((s) => s.closeDetailsModal);

  // Torrent store
  const torrents = useTorrentStore((state) => state.torrents);
  const refreshTorrents = useTorrentStore((state) => state.refreshTorrents);

  // Error store
  const setCloseableError = useErrorStore((state) => state.setCloseableError);

  // Indexarr
  const indexarrEnabled = useIndexarrStore((s) => s.status?.enabled ?? false);
  const setIndexarrStatus = useIndexarrStore((s) => s.setStatus);

  useEffect(() => {
    IndexarrAPI.getStatus()
      .then(setIndexarrStatus)
      .catch(() => setIndexarrStatus({ enabled: false }));
  }, []);

  // Local state
  const [disabled, setDisabled] = useState(false);
  const [showDeleteModal, setShowDeleteModal] = useState(false);
  const [torrentsToDelete, setTorrentsToDelete] = useState<
    Pick<TorrentListItem, "id" | "name">[]
  >([]);
  const [localSearch, setLocalSearch] = useState(searchQuery);
  const [isDark, setIsDark] = useState(DarkMode.isDark());
  const [configOpen, setConfigOpen] = useState(false);
  const authState = useAuthStore((s) => s.state);
  const refreshToken = useAuthStore((s) => s.refreshToken);
  const clearTokens = useAuthStore((s) => s.clearTokens);

  // Overflow menu for the utility icons below sm
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);
  const moreButtonRef = useRef<HTMLButtonElement>(null);
  const moreMenuRef = useRef<HTMLDivElement>(null);
  const closeMore = useCallback((returnFocus: boolean) => {
    setMoreOpen(false);
    if (returnFocus) moreButtonRef.current?.focus();
  }, []);
  useEffect(() => {
    if (!moreOpen) return;
    // Move focus into the menu when it opens
    moreMenuRef.current
      ?.querySelector<HTMLElement>('[role="menuitem"]')
      ?.focus();
    const onPointerDown = (e: PointerEvent) => {
      if (!moreRef.current?.contains(e.target as Node)) closeMore(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeMore(true);
      else if (e.key === "Tab") closeMore(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [moreOpen, closeMore]);

  // Arrow, Home and End keys move between menu items
  const onMenuKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const items = Array.from(
      e.currentTarget.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    );
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLElement);
    let next: number;
    switch (e.key) {
      case "ArrowDown":
        next = (current + 1) % items.length;
        break;
      case "ArrowUp":
        next = (current - 1 + items.length) % items.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = items.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    items[next].focus();
  };

  const handleLogout = async () => {
    if (refreshToken) {
      try {
        await AuthAPI.logout(refreshToken);
      } catch {
        // Ignore logout API errors — clear local state regardless
      }
    }
    clearTokens();
  };

  // Debounced search
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const debouncedSetSearch = useCallback(
    debounce((value: string) => setSearchQuery(value), 150),
    [setSearchQuery],
  );

  const handleSearchChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const value = e.target.value;
    setLocalSearch(value);
    debouncedSetSearch(value);
  };

  const clearSearch = () => {
    setLocalSearch("");
    setSearchQuery("");
  };

  const handleDarkModeToggle = () => {
    DarkMode.toggle();
    setIsDark(DarkMode.isDark());
  };

  const selectedCount = selectedTorrentIds.size;
  const hasSelection = selectedCount > 0;

  const getTorrentById = (id: number) => torrents?.find((t) => t.id === id);

  const openDeleteModal = useCallback(() => {
    const torrentsList = Array.from(selectedTorrentIds).map((id) => {
      const torrent = getTorrentById(id);
      return {
        id,
        name: torrent?.name ?? null,
      };
    });
    setTorrentsToDelete(torrentsList);
    setShowDeleteModal(true);
  }, [selectedTorrentIds, torrents]);

  // Space pauses the selection when every selected torrent is live, otherwise resumes it
  const toggleRunSelected = () => {
    const selected = (torrents ?? []).filter((t) =>
      selectedTorrentIds.has(t.id),
    );
    const allLive =
      selected.length > 0 &&
      selected.every((t) => t.stats?.state === STATE_LIVE);
    if (allLive) {
      pauseSelected();
    } else {
      resumeSelected();
    }
  };

  // Keyboard shortcuts (global)
  const keyboardActions = {
    onDelete: openDeleteModal,
    onToggleRun: toggleRunSelected,
    onOpenDetails: () => {
      const [id] = Array.from(selectedTorrentIds);
      if (id !== undefined) openDetailsModal(id);
    },
    onOpenFilePicker: () => {
      document.querySelector<HTMLInputElement>('input[type="file"]')?.click();
    },
    onFocusMagnet: () => {
      document
        .querySelector<HTMLInputElement>('input[placeholder^="magnet:"]')
        ?.focus();
    },
    onToggleHelp: () => {
      const { shortcutHelpOpen, setShortcutHelpOpen } = useUIStore.getState();
      setShortcutHelpOpen(!shortcutHelpOpen);
    },
  };
  useKeyboardShortcuts(keyboardActions);
  useAnnouncements();

  const runBulkAction = async (
    action: (id: number) => Promise<void>,
    skipState: string,
    errorLabel: string,
  ) => {
    setDisabled(true);
    try {
      for (const id of selectedTorrentIds) {
        const torrent = getTorrentById(id);
        if (torrent?.stats?.state === skipState) continue;
        try {
          await action(id);
          refreshTorrents();
        } catch (e) {
          setCloseableError({
            text: `Error ${errorLabel} torrent id=${id}`,
            details: e as ErrorDetails,
          });
        }
      }
    } finally {
      setDisabled(false);
    }
  };

  const pauseSelected = () =>
    runBulkAction((id) => API.pause(id), STATE_PAUSED, "pausing");
  const resumeSelected = () =>
    runBulkAction((id) => API.start(id), STATE_LIVE, "starting");

  // Hide built-in configure button when custom menuButtons are provided
  const showBuiltInConfigButton = !menuButtons || menuButtons.length === 0;

  const showTorrentActions = currentPage === "torrents";

  const navTab =
    "inline-flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-lg transition-colors cursor-pointer whitespace-nowrap";
  const navTabActive = `${navTab} bg-primary/12 text-primary`;
  const navTabInactive = `${navTab} text-secondary hover:text-text hover:bg-surface/50`;

  // touch-target widens the hit area on coarse pointers; applied to every
  // variant so the icon buttons keep the same size when disabled.
  const toolbarIcon =
    "p-1.5 touch-target text-secondary hover:text-text hover:bg-surface/50 rounded-lg cursor-pointer transition-colors inline-flex items-center justify-center";
  const toolbarIconDisabled =
    "p-1.5 touch-target text-tertiary rounded-lg inline-flex items-center justify-center opacity-40 cursor-not-allowed";
  const toolbarIconDanger =
    "p-1.5 touch-target text-error hover:text-error hover:bg-error/10 rounded-lg cursor-pointer transition-colors inline-flex items-center justify-center";
  const toolbarIconDangerDisabled =
    "p-1.5 touch-target text-error rounded-lg inline-flex items-center justify-center opacity-40 cursor-not-allowed";
  const toolbarMenuItem =
    "inline-flex items-center gap-2 px-2.5 py-1.5 text-sm text-secondary hover:text-text hover:bg-surface/50 rounded-md cursor-pointer whitespace-nowrap";

  return (
    <div className="relative z-30 bg-surface-raised drop-shadow-lg">
      {/* ── Single unified row ── */}
      <div className="flex flex-nowrap items-center gap-0.5 sm:gap-1 px-1 sm:px-2 py-1 sm:py-1.5">
        {/* Mobile hamburger */}
        {!isLargeScreen && (
          <button
            onClick={() => {
              setSidebarOpen(true);
              setCurrentPage("torrents");
            }}
            className="p-1.5 text-secondary hover:text-primary cursor-pointer rounded focus-ring touch-target"
            title="Open sidebar"
            aria-label="Open sidebar"
          >
            <HiOutlineMenu className="w-5 h-5" />
          </button>
        )}

        {/* Logo (image only — no title text) */}
        <div className="flex items-center mr-1" title={title}>
          <Logo className="w-6 h-6" alt={title} />
        </div>

        {/* Divider after logo */}
        {isLargeScreen && <div className="w-px h-5 bg-divider mx-1" />}

        {/* Page navigation tabs (pill style) */}
        {isLargeScreen && (
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => setCurrentPage("torrents")}
              className={
                currentPage === "torrents" ? navTabActive : navTabInactive
              }
            >
              <BsCollection className="w-3.5 h-3.5" />
              Torrents
            </button>
            {indexarrEnabled && (
              <button
                onClick={() => setCurrentPage("indexarr")}
                className={
                  currentPage === "indexarr" ? navTabActive : navTabInactive
                }
              >
                <BsGlobe2 className="w-3.5 h-3.5" />
                Index
              </button>
            )}
            <button
              onClick={() => setCurrentPage("rss")}
              className={currentPage === "rss" ? navTabActive : navTabInactive}
            >
              <BsRss className="w-3.5 h-3.5" />
              RSS
            </button>
          </div>
        )}

        {/* Spacer */}
        <div className="flex-1" />

        {/* Search input (torrents page only) */}
        {showTorrentActions && (
          <div className="relative hidden lg:block min-w-0 shrink">
            <GoSearch className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-tertiary" />
            <input
              type="text"
              data-search-input
              aria-label="Filter torrents"
              value={localSearch}
              onChange={handleSearchChange}
              placeholder="Filter torrents…"
              className="pl-8 pr-7 py-1.5 w-44 shrink min-w-0 text-sm bg-surface/50 border border-divider rounded-lg focus-ring focus:border-primary placeholder:text-tertiary"
            />
            {localSearch && (
              <button
                type="button"
                onClick={clearSearch}
                aria-label="Clear search"
                className="absolute right-1.5 top-1/2 -translate-y-1/2 p-1.5 text-tertiary hover:text-secondary rounded cursor-pointer focus-ring"
              >
                <GoX className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
        )}

        {/* Divider before actions */}
        {showTorrentActions && (
          <div className="hidden lg:block w-px h-5 bg-divider mx-1" />
        )}

        {/* Torrent action group */}
        {showTorrentActions && (
          <div className="flex items-center gap-0 sm:gap-0.5 bg-surface/30 rounded-lg p-0 sm:p-0.5">
            <MagnetInput className="grow-0 justify-center !px-2 sm:!px-2.5 !py-1 !text-sm !rounded-lg" />
            <FileInput
              className="grow-0 justify-center !px-2 sm:!px-2.5 !py-1 !text-sm !rounded-lg"
              onMultiFileSelect={onMultiFileSelect}
            />

            <div className="w-px h-5 bg-divider mx-0.5" />

            <button
              onClick={resumeSelected}
              disabled={disabled || !hasSelection}
              className={
                disabled || !hasSelection ? toolbarIconDisabled : toolbarIcon
              }
              title={`Resume selected (${KEY_HINT.run})`}
              aria-label="Resume selected"
            >
              <FaPlay className="w-3 h-3" />
            </button>
            <button
              onClick={pauseSelected}
              disabled={disabled || !hasSelection}
              className={
                disabled || !hasSelection ? toolbarIconDisabled : toolbarIcon
              }
              title={`Pause selected (${KEY_HINT.run})`}
              aria-label="Pause selected"
            >
              <FaPause className="w-3 h-3" />
            </button>
            <button
              onClick={openDeleteModal}
              disabled={disabled || !hasSelection}
              className={
                disabled || !hasSelection
                  ? toolbarIconDangerDisabled
                  : toolbarIconDanger
              }
              title={`Delete selected (${KEY_HINT.remove})`}
              aria-label="Delete selected"
            >
              <FaTrash className="w-3 h-3" />
            </button>

            {hasSelection && (
              <span className="hidden lg:inline text-xs text-secondary ml-0.5 mr-1">
                {selectedCount}
              </span>
            )}
          </div>
        )}

        {/* Divider before utility icons */}
        <div className="hidden lg:block w-px h-5 bg-divider mx-1" />

        {/* Utility icons: inline from sm up, folded into a menu below sm */}
        {menuButtons?.map((b, i) => (
          <span key={i}>{b}</span>
        ))}
        {showBuiltInConfigButton && (
          <ConfigModal
            isOpen={configOpen}
            onClose={() => setConfigOpen(false)}
            version={version}
          />
        )}
        <div className="hidden sm:contents">
          {showBuiltInConfigButton && (
            <button
              onClick={() => setConfigOpen(true)}
              className={toolbarIcon}
              title={`Configure (${KEY_HINT.settings})`}
              aria-label="Settings"
            >
              <BsSliders2 className="w-4 h-4" />
            </button>
          )}
          <button
            onClick={onLogsClick}
            className={toolbarIcon}
            title={`View logs (${KEY_HINT.logs})`}
            aria-label="View logs"
          >
            <BsBodyText className="w-4 h-4" />
          </button>
          <button
            onClick={handleDarkModeToggle}
            className={toolbarIcon}
            title="Toggle dark mode"
            aria-label={
              isDark ? "Switch to light theme" : "Switch to dark theme"
            }
            aria-pressed={isDark}
          >
            {isDark ? (
              <BsSun className="w-4 h-4" />
            ) : (
              <BsMoon className="w-4 h-4" />
            )}
          </button>
        </div>
        <div ref={moreRef} className="relative sm:hidden">
          <button
            ref={moreButtonRef}
            onClick={() => setMoreOpen((open) => !open)}
            className={toolbarIcon}
            title="More options"
            aria-label="More options"
            aria-haspopup="menu"
            aria-expanded={moreOpen}
          >
            <BsThreeDotsVertical className="w-4 h-4" />
          </button>
          {moreOpen && (
            <div
              ref={moreMenuRef}
              role="menu"
              aria-label="Toolbar options"
              onKeyDown={onMenuKeyDown}
              className="absolute right-0 top-full mt-1 z-20 flex flex-col min-w-40 p-1 bg-surface-raised border border-divider rounded-lg shadow-lg"
            >
              {showBuiltInConfigButton && (
                <button
                  role="menuitem"
                  onClick={() => {
                    setMoreOpen(false);
                    setConfigOpen(true);
                  }}
                  className={toolbarMenuItem}
                >
                  <BsSliders2 className="w-4 h-4" />
                  Settings
                </button>
              )}
              <button
                role="menuitem"
                onClick={() => {
                  setMoreOpen(false);
                  onLogsClick();
                }}
                className={toolbarMenuItem}
              >
                <BsBodyText className="w-4 h-4" />
                View logs
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  setMoreOpen(false);
                  handleDarkModeToggle();
                }}
                className={toolbarMenuItem}
              >
                {isDark ? (
                  <BsSun className="w-4 h-4" />
                ) : (
                  <BsMoon className="w-4 h-4" />
                )}
                {isDark ? "Light theme" : "Dark theme"}
              </button>
              {authState === "authenticated" && (
                <button
                  role="menuitem"
                  onClick={() => {
                    setMoreOpen(false);
                    handleLogout();
                  }}
                  className={toolbarMenuItem}
                >
                  <BsBoxArrowRight className="w-4 h-4" />
                  Log out
                </button>
              )}
            </div>
          )}
        </div>
        {authState === "authenticated" && (
          <div className="hidden sm:contents">
            <button
              onClick={handleLogout}
              className={toolbarIcon}
              title="Logout"
              aria-label="Log out"
            >
              <BsBoxArrowRight className="w-4 h-4" />
            </button>
          </div>
        )}
      </div>

      {/* Delete modal */}
      <DeleteTorrentModal
        show={showDeleteModal}
        onHide={() => setShowDeleteModal(false)}
        torrents={torrentsToDelete}
      />

      {detailsModalTorrentId !== null && (
        <TorrentDetailsModal
          torrentId={detailsModalTorrentId}
          isOpen={true}
          onClose={closeDetailsModal}
        />
      )}

      <ShortcutHelp />
      <LiveAnnouncer />
    </div>
  );
};

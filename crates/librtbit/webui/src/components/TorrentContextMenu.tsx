import {
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { ErrorDetails, TorrentLimits, TorrentListItem } from "../api-types";
import { formatRateLimit } from "../helper/formatRateLimit";
import { formatSecondsToTime } from "../helper/formatSecondsToTime";
import { APIContext } from "../context";
import { useErrorStore } from "../stores/errorStore";
import { useTorrentStore } from "../stores/torrentStore";
import { useUIStore } from "../stores/uiStore";
import { DeleteTorrentModal } from "./modal/DeleteTorrentModal";
import {
  FaPlay,
  FaPause,
  FaCog,
  FaTrash,
  FaCopy,
  FaTag,
  FaPlus,
  FaArrowUp,
  FaArrowDown,
  FaAngleDoubleUp,
  FaAngleDoubleDown,
  FaTachometerAlt,
  FaSeedling,
  FaSyncAlt,
} from "react-icons/fa";
import { BsCheckSquareFill, BsSquare } from "react-icons/bs";
import { KEY_HINT } from "../helper/shortcutList";

export interface ContextMenuState {
  x: number;
  y: number;
  /** Element rect to open beside (keyboard and touch). Flips above it when there is no room below. */
  anchor?: DOMRect;
  torrent: TorrentListItem;
  selectedTorrents: TorrentListItem[];
}

interface TorrentContextMenuProps {
  menu: ContextMenuState;
  onClose: () => void;
}

const MENU_WIDTH = 220;
const VIEWPORT_MARGIN = 8;
const MENU_ITEM_SELECTOR = '[role^="menuitem"]:not(:disabled)';

function clamp(min: number, value: number, max: number) {
  return Math.max(min, Math.min(value, max));
}

export const TorrentContextMenu: React.FC<TorrentContextMenuProps> = ({
  menu,
  onClose,
}) => {
  const { x, y, anchor } = menu;
  const menuRef = useRef<HTMLDivElement>(null);
  // Measured position; set in a layout effect so it is in place before the first paint
  const [position, setPosition] = useState<{
    left: number;
    top: number;
  } | null>(null);
  const [showDelete, setShowDelete] = useState(false);
  const [showCategoryMenu, setShowCategoryMenu] = useState(false);
  const [showNewCategoryInput, setShowNewCategoryInput] = useState(false);
  const [newCategoryName, setNewCategoryName] = useState("");
  const [showTagMenu, setShowTagMenu] = useState(false);
  const [tagRegistry, setTagRegistry] = useState<string[]>([]);
  const [newTagName, setNewTagName] = useState("");
  const [actionInProgress, setActionInProgress] = useState(false);
  const [showSpeedLimits, setShowSpeedLimits] = useState(false);
  const [showSeedLimits, setShowSeedLimits] = useState(false);
  const [dlRate, setDlRate] = useState("");
  const [ulRate, setUlRate] = useState("");
  const [ratioLimit, setRatioLimit] = useState("");
  const [timeLimit, setTimeLimit] = useState("");
  const [currentLimits, setCurrentLimits] = useState<TorrentLimits | null>(
    null,
  );
  const newCategoryInputRef = useRef<HTMLInputElement>(null);
  const newTagInputRef = useRef<HTMLInputElement>(null);

  const API = useContext(APIContext);
  const setCloseableError = useErrorStore((s) => s.setCloseableError);
  const refreshTorrents = useTorrentStore((s) => s.refreshTorrents);
  const openDetailsModal = useUIStore((s) => s.openDetailsModal);
  const setDetailPaneTab = useUIStore((s) => s.setDetailPaneTab);
  const categories = useUIStore((s) => s.categories);
  const setCategories = useUIStore((s) => s.setCategories);

  const targets = menu.selectedTorrents;

  useEffect(() => {
    let cancelled = false;
    API.getTags()
      .then((tags) => {
        if (!cancelled) setTagRegistry(tags);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [API]);
  const isBulk = targets.length > 1;

  const hasLive = targets.some((t) => t.stats?.state === "live");
  const hasResumable = targets.some(
    (t) => t.stats?.state === "paused" || t.stats?.state === "error",
  );
  const singleTarget = targets.length === 1 ? targets[0] : null;
  const canConfigure =
    singleTarget &&
    (singleTarget.stats?.state === "paused" ||
      singleTarget.stats?.state === "live");

  const hasFinished = targets.some((t) => t.stats?.finished);
  const hasQueueState = targets.some((t) => t.stats?.queue_state != null);
  const canRecheck = targets.some((t) => t.stats?.state !== "initializing");

  // For single target toggles
  const isSequential = singleTarget?.stats?.sequential ?? false;
  const isSuperSeeding = singleTarget?.stats?.super_seeding ?? false;

  // Fetch categories once when menu opens
  const categoriesFetched = useRef(false);
  useEffect(() => {
    if (categoriesFetched.current) return;
    categoriesFetched.current = true;
    API.getCategories()
      .then((cats) => setCategories(cats))
      .catch(() => {});
  }, [API, setCategories]);

  // Fetch current speed limits when the menu opens, to show them on the menu item
  const singleTargetId = singleTarget?.id;
  useEffect(() => {
    if (singleTargetId == null) return;
    let cancelled = false;
    API.getTorrentLimits(singleTargetId)
      .then((limits) => {
        if (!cancelled) setCurrentLimits(limits);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [singleTargetId, API]);

  // Prefill the speed limit form with the current values when it opens
  useEffect(() => {
    if (showSpeedLimits && currentLimits) {
      setDlRate(currentLimits.download_rate?.toString() ?? "");
      setUlRate(currentLimits.upload_rate?.toString() ?? "");
    }
  }, [showSpeedLimits, currentLimits]);

  useEffect(() => {
    if (showSeedLimits && singleTarget) {
      const stats = singleTarget.stats;
      setRatioLimit(
        stats?.seed_ratio_limit != null
          ? stats.seed_ratio_limit.toString()
          : "",
      );
      setTimeLimit(
        stats?.seed_time_limit_secs != null
          ? stats.seed_time_limit_secs.toString()
          : "",
      );
    }
  }, [showSeedLimits, singleTarget]);

  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      // Ignore right-clicks — those are handled by the contextmenu event
      if (e.button === 2) return;
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const handleScroll = () => onClose();

    document.addEventListener("mousedown", handleMouseDown);
    document.addEventListener("keydown", handleKeyDown);
    window.addEventListener("scroll", handleScroll, true);
    return () => {
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("scroll", handleScroll, true);
    };
  }, [onClose]);

  // Place the menu after mount, when its real height is known, so it stays on screen
  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const { height } = el.getBoundingClientRect();
    const { innerWidth, innerHeight } = window;
    let top = y;
    if (anchor && y + height > innerHeight - VIEWPORT_MARGIN) {
      const above = anchor.top - height;
      if (above >= VIEWPORT_MARGIN) top = above;
    }
    setPosition({
      left: clamp(
        VIEWPORT_MARGIN,
        x,
        innerWidth - MENU_WIDTH - VIEWPORT_MARGIN,
      ),
      top: clamp(VIEWPORT_MARGIN, top, innerHeight - height - VIEWPORT_MARGIN),
    });
  }, [x, y, anchor]);

  // Focus the first item on open; on close (any route) hand focus back to whatever opened the menu.
  // The capture must come before the focus call so the opener is recorded, not the first item.
  useEffect(() => {
    const opener = document.activeElement;
    menuRef.current?.querySelector<HTMLElement>(MENU_ITEM_SELECTOR)?.focus();
    return () => {
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
    };
  }, []);

  const handleMenuKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Tab") {
      e.preventDefault();
      onClose();
      return;
    }
    // Let text inputs (speed limits, new category) keep their arrow and Home/End keys
    if (e.target instanceof HTMLInputElement) return;
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLElement>(MENU_ITEM_SELECTOR) ?? [],
    );
    const current = items.indexOf(document.activeElement as HTMLElement);
    let next: number;
    switch (e.key) {
      case "ArrowDown":
        next = (current + 1) % items.length;
        break;
      case "ArrowUp":
        next = current <= 0 ? items.length - 1 : current - 1;
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
    items[next]?.focus();
  };

  const handleResume = async () => {
    setActionInProgress(true);
    for (const t of targets) {
      if (t.stats?.state === "paused" || t.stats?.state === "error") {
        try {
          await API.start(t.id);
        } catch (e: any) {
          setCloseableError({
            text: `Error starting torrent "${t.name ?? t.id}"`,
            details: e,
          });
        }
      }
    }
    refreshTorrents();
    onClose();
  };

  const handlePause = async () => {
    setActionInProgress(true);
    for (const t of targets) {
      if (t.stats?.state === "live") {
        try {
          await API.pause(t.id);
        } catch (e: any) {
          setCloseableError({
            text: `Error pausing torrent "${t.name ?? t.id}"`,
            details: e,
          });
        }
      }
    }
    refreshTorrents();
    onClose();
  };

  const handleRecheck = async () => {
    setActionInProgress(true);
    for (const t of targets) {
      if (t.stats?.state === "initializing") continue;
      try {
        await API.recheck(t.id);
      } catch (e: unknown) {
        setCloseableError({
          text: `Error checking torrent "${t.name ?? t.id}"`,
          details: e as ErrorDetails,
        });
      }
    }
    refreshTorrents();
    onClose();
  };

  const handleCopyName = () => {
    const text = targets
      .map((t) => t.name ?? "")
      .filter(Boolean)
      .join("\n");
    navigator.clipboard.writeText(text);
    onClose();
  };

  const handleCopyInfoHash = () => {
    const text = targets.map((t) => t.info_hash).join("\n");
    navigator.clipboard.writeText(text);
    onClose();
  };

  const handleCopyMagnetLink = () => {
    const links = targets.map((t) => {
      let magnet = `magnet:?xt=urn:btih:${t.info_hash}`;
      if (t.name) {
        magnet += `&dn=${encodeURIComponent(t.name)}`;
      }
      return magnet;
    });
    navigator.clipboard.writeText(links.join("\n"));
    onClose();
  };

  const handleConfigure = () => {
    if (singleTarget) {
      openDetailsModal(singleTarget.id);
      setDetailPaneTab("files");
    }
    onClose();
  };

  const handleSetCategory = async (category: string | null) => {
    setActionInProgress(true);
    for (const t of targets) {
      try {
        await API.setTorrentCategory(t.id, category);
      } catch (e: any) {
        setCloseableError({
          text: `Error setting category for "${t.name ?? t.id}"`,
          details: e,
        });
      }
    }
    refreshTorrents();
    onClose();
  };

  const tagNames = Array.from(
    new Set([...tagRegistry, ...targets.flatMap((t) => t.tags ?? [])]),
  ).sort((a, b) => a.localeCompare(b));

  const editTags = async (
    call: (ids: number[], tags: string[]) => Promise<void>,
    tags: string[],
    failure: string,
  ) => {
    setActionInProgress(true);
    try {
      await call(
        targets.map((t) => t.id),
        tags,
      );
    } catch (e: any) {
      setCloseableError({ text: failure, details: e });
    }
    refreshTorrents();
    onClose();
  };

  const handleAddTag = (tag: string) =>
    editTags(API.addTorrentTags, [tag], `Error adding tag "${tag}"`);

  const handleRemoveTag = (tag: string) =>
    editTags(API.removeTorrentTags, [tag], `Error removing tag "${tag}"`);

  const handleCreateAndAssignTag = async () => {
    const name = newTagName.trim();
    if (!name) return;
    setActionInProgress(true);
    try {
      await API.createTags([name]);
      await API.addTorrentTags(
        targets.map((t) => t.id),
        [name],
      );
    } catch (e: any) {
      setCloseableError({
        text: `Error adding tag "${name}"`,
        details: e,
      });
    }
    refreshTorrents();
    onClose();
  };

  const handleCreateAndAssignCategory = async () => {
    const name = newCategoryName.trim();
    if (!name) {
      setShowNewCategoryInput(false);
      return;
    }
    setActionInProgress(true);
    try {
      await API.createCategory(name);
      const cats = await API.getCategories();
      setCategories(cats);
    } catch {
      // category may already exist, still assign it
    }
    for (const t of targets) {
      try {
        await API.setTorrentCategory(t.id, name);
      } catch (e: any) {
        setCloseableError({
          text: `Error setting category for "${t.name ?? t.id}"`,
          details: e,
        });
      }
    }
    refreshTorrents();
    onClose();
  };

  const handleToggleSequential = async () => {
    setActionInProgress(true);
    for (const t of targets) {
      try {
        const current = t.stats?.sequential ?? false;
        await API.setSequential(t.id, !current);
      } catch (e: any) {
        setCloseableError({
          text: `Error toggling sequential for "${t.name ?? t.id}"`,
          details: e,
        });
      }
    }
    refreshTorrents();
    onClose();
  };

  const handleToggleSuperSeed = async () => {
    setActionInProgress(true);
    for (const t of targets) {
      try {
        const current = t.stats?.super_seeding ?? false;
        await API.setSuperSeed(t.id, !current);
      } catch (e: any) {
        setCloseableError({
          text: `Error toggling super-seed for "${t.name ?? t.id}"`,
          details: e,
        });
      }
    }
    refreshTorrents();
    onClose();
  };

  const handleQueueAction = async (
    action: (id: number) => Promise<void>,
    label: string,
  ) => {
    setActionInProgress(true);
    for (const t of targets) {
      try {
        await action(t.id);
      } catch (e: any) {
        setCloseableError({
          text: `Error ${label} for "${t.name ?? t.id}"`,
          details: e,
        });
      }
    }
    refreshTorrents();
    onClose();
  };

  const handleSetSpeedLimits = async () => {
    if (!singleTarget) return;
    setActionInProgress(true);
    try {
      await API.setTorrentLimits(singleTarget.id, {
        download_rate: dlRate ? Number(dlRate) : undefined,
        upload_rate: ulRate ? Number(ulRate) : undefined,
      });
    } catch (e: any) {
      setCloseableError({
        text: `Error setting speed limits`,
        details: e,
      });
    }
    refreshTorrents();
    onClose();
  };

  const handleSetSeedLimits = async () => {
    if (!singleTarget) return;
    setActionInProgress(true);
    try {
      await API.setTorrentSeedLimits(singleTarget.id, {
        ratio_limit: ratioLimit ? Number(ratioLimit) : null,
        time_limit_secs: timeLimit ? Number(timeLimit) : null,
      });
    } catch (e: any) {
      setCloseableError({
        text: `Error setting seed limits`,
        details: e,
      });
    }
    refreshTorrents();
    onClose();
  };

  const itemCls =
    "flex items-center gap-2 w-full px-3 py-1.5 text-sm text-left hover:bg-surface focus:bg-surface focus:outline-none cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed";
  const iconCls = "w-3.5 h-3.5 shrink-0";
  const keyHintCls = "ml-auto pl-4 text-xs text-tertiary";
  const separator = (
    <div role="separator" className="border-t border-divider my-1" />
  );
  const inputCls =
    "w-full px-2 py-1 text-sm bg-surface border border-divider rounded focus-ring placeholder:text-tertiary";

  const categoryNames = Object.keys(categories).sort((a, b) =>
    a.localeCompare(b),
  );

  const CheckIcon: React.FC<{ checked: boolean }> = ({ checked }) =>
    checked ? (
      <BsCheckSquareFill className={`${iconCls} text-primary`} />
    ) : (
      <BsSquare className={`${iconCls} text-secondary`} />
    );

  return (
    <>
      <div
        ref={menuRef}
        role="menu"
        aria-label="Torrent actions"
        onKeyDown={handleMenuKeyDown}
        className="fixed z-50 bg-surface-raised border border-divider rounded-lg shadow-lg py-1 overflow-y-auto max-h-[calc(100dvh-16px)]"
        style={{
          left: position?.left ?? x,
          top: position?.top ?? y,
          width: MENU_WIDTH,
        }}
      >
        {hasResumable && (
          <button
            role="menuitem"
            className={itemCls}
            onClick={handleResume}
            disabled={actionInProgress}
          >
            <FaPlay className={`${iconCls} text-green-600`} />
            {isBulk ? "Resume Selected" : "Resume"}
            <span className={keyHintCls}>{KEY_HINT.run}</span>
          </button>
        )}
        {hasLive && (
          <button
            role="menuitem"
            className={itemCls}
            onClick={handlePause}
            disabled={actionInProgress}
          >
            <FaPause className={`${iconCls} text-amber-500`} />
            {isBulk ? "Pause Selected" : "Pause"}
            <span className={keyHintCls}>{KEY_HINT.run}</span>
          </button>
        )}

        {(hasResumable || hasLive) && separator}

        <button
          role="menuitem"
          className={itemCls}
          onClick={handleRecheck}
          disabled={actionInProgress || !canRecheck}
        >
          <FaSyncAlt className={`${iconCls} text-secondary`} />
          {isBulk ? "Force Recheck Selected" : "Force Recheck"}
        </button>

        {separator}

        {canConfigure && (
          <>
            <button
              role="menuitem"
              className={itemCls}
              onClick={handleConfigure}
            >
              <FaCog className={`${iconCls} text-secondary`} />
              Configure Files
            </button>
            {separator}
          </>
        )}

        {/* Transfer toggles */}
        <button
          role="menuitemcheckbox"
          aria-checked={!isBulk && isSequential}
          className={itemCls}
          onClick={handleToggleSequential}
          disabled={actionInProgress}
        >
          <CheckIcon checked={isBulk ? false : isSequential} />
          Sequential Download
        </button>
        {hasFinished && (
          <button
            role="menuitemcheckbox"
            aria-checked={!isBulk && isSuperSeeding}
            className={itemCls}
            onClick={handleToggleSuperSeed}
            disabled={actionInProgress}
          >
            <CheckIcon checked={isBulk ? false : isSuperSeeding} />
            Super-Seed
          </button>
        )}

        {separator}

        {/* Queue controls */}
        {hasQueueState && (
          <>
            <button
              role="menuitem"
              className={itemCls}
              onClick={() =>
                handleQueueAction((id) => API.queueMoveTop(id), "moving to top")
              }
              disabled={actionInProgress}
            >
              <FaAngleDoubleUp className={`${iconCls} text-secondary`} />
              Move to Top
            </button>
            <button
              role="menuitem"
              className={itemCls}
              onClick={() =>
                handleQueueAction((id) => API.queueMoveUp(id), "moving up")
              }
              disabled={actionInProgress}
            >
              <FaArrowUp className={`${iconCls} text-secondary`} />
              Move Up
            </button>
            <button
              role="menuitem"
              className={itemCls}
              onClick={() =>
                handleQueueAction((id) => API.queueMoveDown(id), "moving down")
              }
              disabled={actionInProgress}
            >
              <FaArrowDown className={`${iconCls} text-secondary`} />
              Move Down
            </button>
            <button
              role="menuitem"
              className={itemCls}
              onClick={() =>
                handleQueueAction(
                  (id) => API.queueMoveBottom(id),
                  "moving to bottom",
                )
              }
              disabled={actionInProgress}
            >
              <FaAngleDoubleDown className={`${iconCls} text-secondary`} />
              Move to Bottom
            </button>
            {separator}
          </>
        )}

        {/* Per-torrent limits */}
        {singleTarget && (
          <>
            <button
              role="menuitem"
              className={itemCls}
              onClick={() => setShowSpeedLimits((v) => !v)}
              disabled={actionInProgress}
            >
              <FaTachometerAlt className={`${iconCls} text-secondary`} />
              <span className="flex-1">Set Speed Limits...</span>
              {currentLimits && (
                <span className="text-xs text-tertiary tabular-nums">
                  ↓ {formatRateLimit(currentLimits.download_rate)} · ↑{" "}
                  {formatRateLimit(currentLimits.upload_rate)}
                </span>
              )}
            </button>
            {showSpeedLimits && (
              <div className="px-3 py-1.5 flex flex-col gap-1.5">
                <input
                  type="number"
                  value={dlRate}
                  onChange={(e) => setDlRate(e.target.value)}
                  placeholder="Download (bytes/s)"
                  className={inputCls}
                  min="0"
                />
                <input
                  type="number"
                  value={ulRate}
                  onChange={(e) => setUlRate(e.target.value)}
                  placeholder="Upload (bytes/s)"
                  className={inputCls}
                  min="0"
                />
                <button
                  className="px-2 py-1 text-sm bg-primary-bg text-white rounded hover:bg-primary-bg-hover cursor-pointer"
                  onClick={handleSetSpeedLimits}
                >
                  Apply
                </button>
              </div>
            )}
            <button
              role="menuitem"
              className={itemCls}
              onClick={() => setShowSeedLimits((v) => !v)}
              disabled={actionInProgress}
            >
              <FaSeedling className={`${iconCls} text-secondary`} />
              <span className="flex-1">Set Seed Limits...</span>
              <span className="text-xs text-tertiary tabular-nums">
                {singleTarget.stats?.seed_ratio_limit != null
                  ? singleTarget.stats.seed_ratio_limit.toFixed(2)
                  : "\u221e"}{" "}
                ·{" "}
                {singleTarget.stats?.seed_time_limit_secs != null
                  ? formatSecondsToTime(singleTarget.stats.seed_time_limit_secs)
                  : "\u221e"}
              </span>
            </button>
            {showSeedLimits && (
              <div className="px-3 py-1.5 flex flex-col gap-1.5">
                <input
                  type="number"
                  value={ratioLimit}
                  onChange={(e) => setRatioLimit(e.target.value)}
                  placeholder="Ratio limit (e.g. 2.0)"
                  className={inputCls}
                  min="0"
                  step="0.1"
                />
                <input
                  type="number"
                  value={timeLimit}
                  onChange={(e) => setTimeLimit(e.target.value)}
                  placeholder="Time limit (seconds)"
                  className={inputCls}
                  min="0"
                />
                <button
                  className="px-2 py-1 text-sm bg-primary-bg text-white rounded hover:bg-primary-bg-hover cursor-pointer"
                  onClick={handleSetSeedLimits}
                >
                  Apply
                </button>
              </div>
            )}
            {separator}
          </>
        )}

        {/* Tags submenu */}
        <div className="relative">
          <button
            role="menuitem"
            className={itemCls}
            onClick={() => setShowTagMenu((v) => !v)}
            disabled={actionInProgress}
          >
            <FaTag className={`${iconCls} text-secondary`} />
            Tags...
          </button>
          {showTagMenu && (
            <div className="border-t border-divider bg-surface-raised">
              {tagNames.map((name) => {
                const onAll = targets.every((t) =>
                  (t.tags ?? []).includes(name),
                );
                return (
                  <button
                    role="menuitem"
                    key={name}
                    className={itemCls}
                    onClick={() =>
                      onAll ? handleRemoveTag(name) : handleAddTag(name)
                    }
                  >
                    <span className="flex-1 truncate">{name}</span>
                    <span className="text-xs text-tertiary">
                      {onAll ? "Remove" : "Add"}
                    </span>
                  </button>
                );
              })}
              <div className="px-3 py-1 flex gap-1">
                <input
                  ref={newTagInputRef}
                  type="text"
                  value={newTagName}
                  onChange={(e) => setNewTagName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleCreateAndAssignTag();
                  }}
                  placeholder="New tag..."
                  className={inputCls}
                />
                <button
                  type="button"
                  className="px-2 py-1 text-sm bg-primary-bg text-white rounded hover:bg-primary-bg-hover cursor-pointer"
                  onClick={handleCreateAndAssignTag}
                >
                  Add
                </button>
              </div>
            </div>
          )}
        </div>

        {/* Category submenu */}
        <div className="relative">
          <button
            role="menuitem"
            className={itemCls}
            onClick={() => setShowCategoryMenu((v) => !v)}
            disabled={actionInProgress}
          >
            <FaTag className={`${iconCls} text-secondary`} />
            Set Category...
          </button>
          {showCategoryMenu && (
            <div className="border-t border-divider bg-surface-raised">
              <button
                role="menuitem"
                className={`${itemCls} text-tertiary`}
                onClick={() => handleSetCategory(null)}
              >
                None
              </button>
              {categoryNames.map((name) => (
                <button
                  role="menuitem"
                  key={name}
                  className={itemCls}
                  onClick={() => handleSetCategory(name)}
                >
                  {name}
                </button>
              ))}
              {showNewCategoryInput ? (
                <div className="px-3 py-1">
                  <input
                    ref={newCategoryInputRef}
                    type="text"
                    value={newCategoryName}
                    onChange={(e) => setNewCategoryName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleCreateAndAssignCategory();
                      if (e.key === "Escape") {
                        setNewCategoryName("");
                        setShowNewCategoryInput(false);
                      }
                    }}
                    onBlur={handleCreateAndAssignCategory}
                    placeholder="Category name..."
                    autoFocus
                    className={inputCls}
                  />
                </div>
              ) : (
                <button
                  role="menuitem"
                  className={itemCls}
                  onClick={() => setShowNewCategoryInput(true)}
                >
                  <FaPlus className={`${iconCls} text-tertiary`} />
                  <span className="text-tertiary">New Category...</span>
                </button>
              )}
            </div>
          )}
        </div>

        {separator}

        <button role="menuitem" className={itemCls} onClick={handleCopyName}>
          <FaCopy className={`${iconCls} text-secondary`} />
          Copy Name
        </button>
        <button
          role="menuitem"
          className={itemCls}
          onClick={handleCopyInfoHash}
        >
          <FaCopy className={`${iconCls} text-secondary`} />
          Copy Info Hash
        </button>
        <button
          role="menuitem"
          className={itemCls}
          onClick={handleCopyMagnetLink}
        >
          <FaCopy className={`${iconCls} text-secondary`} />
          Copy Magnet Link
        </button>

        {separator}

        <button
          role="menuitem"
          className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-left text-error hover:bg-error/10 cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
          onClick={() => setShowDelete(true)}
          disabled={actionInProgress}
        >
          <FaTrash className={`${iconCls}`} />
          {isBulk ? `Delete ${targets.length} Torrents...` : "Delete..."}
          <span className={keyHintCls}>{KEY_HINT.remove}</span>
        </button>
      </div>

      <DeleteTorrentModal
        show={showDelete}
        onHide={() => {
          setShowDelete(false);
          onClose();
        }}
        torrents={targets.map((t) => ({ id: t.id, name: t.name }))}
      />
    </>
  );
};

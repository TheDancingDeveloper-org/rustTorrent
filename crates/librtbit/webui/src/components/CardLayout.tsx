import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { Virtuoso } from "react-virtuoso";
import { GoSearch, GoX } from "react-icons/go";
import debounce from "lodash.debounce";
import { TorrentListItem } from "../api-types";
import { Spinner } from "./Spinner";
import { TorrentCard } from "./TorrentCard";
import { useUIStore } from "../stores/uiStore";
import {
  TorrentSortColumn,
  SortDirection,
  StatusFilter,
  SORT_COLUMN_LABELS,
  STATUS_FILTER_LABELS,
  compareTorrents,
  isTorrentVisible,
} from "../helper/torrentFilters";
import { useKeyboardShortcuts } from "../hooks/useKeyboardShortcuts";
import { TorrentContextMenu, ContextMenuState } from "./TorrentContextMenu";

const LONG_PRESS_MS = 500;
const LONG_PRESS_SLOP_PX = 10;

export const CardLayout = (props: {
  torrents: Array<TorrentListItem> | null;
  loading: boolean;
}) => {
  const searchQuery = useUIStore((state) => state.searchQuery);
  const setSearchQuery = useUIStore((state) => state.setSearchQuery);
  const statusFilter = useUIStore((state) => state.statusFilter);
  const setStatusFilter = useUIStore((state) => state.setStatusFilter);
  const categoryFilter = useUIStore((state) => state.categoryFilter);
  const trackerFilter = useUIStore((state) => state.trackerFilter);
  const tagFilter = useUIStore((state) => state.tagFilter);

  // Keyboard shortcuts (Ctrl+A, Ctrl+F, Escape)
  useKeyboardShortcuts();
  const [localSearch, setLocalSearch] = useState(searchQuery);
  const storedSortColumn = useUIStore((state) => state.sortColumn);
  const sortDirection = useUIStore((state) => state.sortDirection);
  const setSort = useUIStore((state) => state.setSort);
  const sortColumn: TorrentSortColumn =
    storedSortColumn in SORT_COLUMN_LABELS
      ? (storedSortColumn as TorrentSortColumn)
      : "id";
  const [torrentContextMenu, setTorrentContextMenu] =
    useState<ContextMenuState | null>(null);

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

  const handleSortChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const [col, dir] = e.target.value.split(":") as [
      TorrentSortColumn,
      SortDirection,
    ];
    setSort(col, dir);
  };

  const normalizedQuery = searchQuery.toLowerCase().trim();

  // Sort and filter torrents for virtualization
  const filteredTorrents = useMemo(() => {
    if (!props.torrents) return null;
    return [...props.torrents]
      .filter((t) =>
        isTorrentVisible(
          t,
          normalizedQuery,
          statusFilter,
          categoryFilter,
          trackerFilter,
          undefined,
          tagFilter,
        ),
      )
      .sort((a, b) => compareTorrents(a, b, sortColumn, sortDirection));
  }, [
    props.torrents,
    normalizedQuery,
    statusFilter,
    categoryFilter,
    trackerFilter,
    tagFilter,
    sortColumn,
    sortDirection,
  ]);

  // Handlers read the card's index from the DOM (data-index) and the list from a ref,
  // so they stay stable across renders and can be shared by every card
  const filteredRef = useRef<TorrentListItem[] | null>(null);
  filteredRef.current = filteredTorrents;

  const openCardMenu = useCallback((x: number, y: number, index: number) => {
    const torrent = filteredRef.current?.[index];
    if (!torrent) return;
    setTorrentContextMenu({ x, y, torrent, selectedTorrents: [torrent] });
  }, []);

  const handleCardContextMenu = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      openCardMenu(e.clientX, e.clientY, Number(e.currentTarget.dataset.index));
    },
    [openCardMenu],
  );

  // Touch long-press (500 ms) opens the same menu as a right-click
  const longPress = useRef<{
    timer: number;
    x: number;
    y: number;
    index: number;
  } | null>(null);

  const cancelLongPress = useCallback(() => {
    if (!longPress.current) return;
    window.clearTimeout(longPress.current.timer);
    longPress.current = null;
  }, []);

  useEffect(() => cancelLongPress, [cancelLongPress]);

  const handleCardPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (e.pointerType !== "touch") return;
      cancelLongPress();
      const { clientX: x, clientY: y } = e;
      const index = Number(e.currentTarget.dataset.index);
      const timer = window.setTimeout(() => {
        longPress.current = null;
        openCardMenu(x, y, index);
      }, LONG_PRESS_MS);
      longPress.current = { timer, x, y, index };
    },
    [cancelLongPress, openCardMenu],
  );

  // Moving past a small slop means the user is scrolling, not holding still
  const handleCardPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const press = longPress.current;
      if (!press) return;
      if (
        Math.hypot(e.clientX - press.x, e.clientY - press.y) >
        LONG_PRESS_SLOP_PX
      )
        cancelLongPress();
    },
    [cancelLongPress],
  );

  // Item renderer for react-virtuoso
  const itemContent = useCallback(
    (index: number) => {
      const torrent = filteredTorrents![index];
      return (
        <div
          data-index={index}
          className="pb-1.5 sm:pb-2 px-2 sm:px-7 max-w-4xl mx-auto w-full"
          onContextMenu={handleCardContextMenu}
          onPointerDown={handleCardPointerDown}
          onPointerMove={handleCardPointerMove}
          onPointerUp={cancelLongPress}
          onPointerCancel={cancelLongPress}
        >
          <TorrentCard key={torrent.id} torrent={torrent} />
        </div>
      );
    },
    [
      filteredTorrents,
      handleCardContextMenu,
      handleCardPointerDown,
      handleCardPointerMove,
      cancelLongPress,
    ],
  );

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1.5 sm:gap-2 w-full max-w-2xl mx-auto mb-2 mt-3 px-2 sm:px-7">
        {/* Search input */}
        <div className="relative flex-1 min-w-0">
          <GoSearch className="absolute left-2.5 sm:left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-tertiary" />
          <input
            type="text"
            data-search-input
            aria-label="Filter torrents"
            value={localSearch}
            onChange={handleSearchChange}
            placeholder="Search..."
            className="w-full pl-8 sm:pl-9 pr-8 sm:pr-9 py-1.5 sm:py-2 text-sm bg-surface border border-divider rounded-lg focus-ring placeholder:text-tertiary"
          />
          {localSearch && (
            <button
              type="button"
              aria-label="Clear search"
              onClick={clearSearch}
              className="absolute right-1.5 sm:right-2 top-1/2 -translate-y-1/2 p-1 text-tertiary hover:text-secondary rounded cursor-pointer focus-ring"
            >
              <GoX className="w-4 h-4" />
            </button>
          )}
        </div>

        {/* Status filter */}
        <select
          aria-label="Status filter"
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value as StatusFilter)}
          className="py-1.5 sm:py-2 px-2 sm:px-3 text-sm bg-surface border border-divider rounded-lg focus-ring"
        >
          {(Object.keys(STATUS_FILTER_LABELS) as StatusFilter[]).map(
            (status) => (
              <option key={status} value={status}>
                {STATUS_FILTER_LABELS[status]}
              </option>
            ),
          )}
        </select>

        {/* Sort dropdown */}
        <select
          aria-label="Sort by"
          value={`${sortColumn}:${sortDirection}`}
          onChange={handleSortChange}
          className="py-1.5 sm:py-2 px-2 sm:px-3 text-sm bg-surface border border-divider rounded-lg focus-ring"
        >
          {(Object.keys(SORT_COLUMN_LABELS) as TorrentSortColumn[]).flatMap(
            (col) => [
              <option key={`${col}:desc`} value={`${col}:desc`}>
                {SORT_COLUMN_LABELS[col]} ↓
              </option>,
              <option key={`${col}:asc`} value={`${col}:asc`}>
                {SORT_COLUMN_LABELS[col]} ↑
              </option>,
            ],
          )}
        </select>
      </div>
      {filteredTorrents === null ? (
        props.loading ? (
          <Spinner
            className="justify-center m-5"
            label="Loading torrent list"
          />
        ) : null
      ) : filteredTorrents.length === 0 ? (
        <p className="text-center">No existing torrents found.</p>
      ) : (
        <div className="flex-1 min-h-0">
          <Virtuoso
            totalCount={filteredTorrents.length}
            itemContent={itemContent}
          />
        </div>
      )}
      {torrentContextMenu && (
        <TorrentContextMenu
          menu={torrentContextMenu}
          onClose={() => setTorrentContextMenu(null)}
        />
      )}
    </div>
  );
};

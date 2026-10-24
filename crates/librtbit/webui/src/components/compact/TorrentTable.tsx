import { useMemo, useCallback, useEffect, useState, useRef } from "react";
import { Virtuoso, VirtuosoHandle } from "react-virtuoso";
import { TorrentListItem } from "../../api-types";
import { TorrentTableRow } from "./TorrentTableRow";
import { useUIStore } from "../../stores/uiStore";
import { useColumnStore, ColumnDef, ColumnId } from "../../stores/columnStore";
import { Spinner } from "../Spinner";
import { SortIcon } from "../SortIcon";
import {
  isTorrentVisible,
  SortDirection,
  torrentTrackerHosts,
} from "../../helper/torrentFilters";
import { Button } from "../buttons/Button";
import { ColumnMenu } from "./ColumnMenu";
import { loadViewPref, oneOf, saveViewPref } from "../../helper/viewPrefs";
import { nextSort } from "../../helper/sortToggle";
import { TorrentContextMenu, ContextMenuState } from "../TorrentContextMenu";

// Sort columns: all sortable column IDs
const TABLE_SORT_COLUMNS = [
  "id",
  "name",
  "size",
  "progress",
  "downloadedBytes",
  "downSpeed",
  "upSpeed",
  "uploadedBytes",
  "eta",
  "peers",
  "state",
  "ratio",
  "category",
  "seeding_time",
  "queue_position",
  "availability",
  "added_on",
  "tracker",
  "amount_left",
  "save_path",
] as const;

export type TableSortColumn = (typeof TABLE_SORT_COLUMNS)[number];

const DEFAULT_SORT_COLUMN: TableSortColumn = "id";
const DEFAULT_SORT_DIRECTION: SortDirection = "desc";

interface TableSortPref {
  column: TableSortColumn;
  direction: SortDirection;
}

const isTableSortPref = (value: unknown): value is TableSortPref => {
  if (typeof value !== "object" || value === null) return false;
  const { column, direction } = value as Record<string, unknown>;
  return (
    oneOf(TABLE_SORT_COLUMNS)(column) &&
    (direction === "asc" || direction === "desc")
  );
};
const DEFAULT_SORT_PREF: TableSortPref = {
  column: DEFAULT_SORT_COLUMN,
  direction: DEFAULT_SORT_DIRECTION,
};
const RESIZE_STEP_PX = 16;

function getTableSortValue(
  t: TorrentListItem,
  column: TableSortColumn,
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
    case "downloadedBytes":
      return t.stats?.progress_bytes ?? 0;
    case "downSpeed":
      return t.stats?.live?.download_speed?.mbps ?? 0;
    case "upSpeed":
      return t.stats?.live?.upload_speed?.mbps ?? 0;
    case "uploadedBytes":
      return t.stats?.live?.snapshot.uploaded_bytes ?? 0;
    case "eta": {
      if (!t.stats?.live) return Infinity;
      const remaining =
        (t.stats.total_bytes ?? 0) - (t.stats.progress_bytes ?? 0);
      const speed = t.stats.live.download_speed?.mbps ?? 0;
      if (speed <= 0 || remaining <= 0) return remaining <= 0 ? 0 : Infinity;
      return remaining / (speed * 1024 * 1024);
    }
    case "peers":
      return t.stats?.live?.snapshot.peer_stats?.live ?? 0;
    case "state":
      return t.stats?.state ?? "";
    case "ratio": {
      if (t.stats?.ratio != null) return t.stats.ratio;
      const uploaded = t.stats?.live?.snapshot.uploaded_bytes ?? 0;
      const total = t.stats?.total_bytes ?? 1;
      return total > 0 ? uploaded / total : 0;
    }
    case "category":
      return (t.category ?? "").toLowerCase();
    case "seeding_time":
      return t.stats?.seeding_time_secs ?? 0;
    case "queue_position":
      return t.stats?.queue_position ?? Infinity;
    case "availability":
      return t.stats?.min_piece_availability ?? 0;
    case "added_on":
      return t.added_on ?? 0;
    case "tracker":
      return torrentTrackerHosts(t)[0] ?? "";
    case "amount_left":
      return Math.max(
        0,
        (t.stats?.total_bytes ?? 0) - (t.stats?.progress_bytes ?? 0),
      );
    case "save_path":
      return t.output_folder ?? "";
  }
}

/** Generate a <colgroup> from visible columns with their widths */
function TableColGroup({ columns }: { columns: ColumnDef[] }) {
  // Subscribe to columnWidths directly so we re-render when widths change
  useColumnStore((s) => s.columnWidths);
  const getWidth = useColumnStore((s) => s.getWidth);
  return (
    <colgroup>
      {columns.map((col) => {
        const w = getWidth(col.id);
        return (
          <col key={col.id} style={w > 0 ? { width: `${w}px` } : undefined} />
        );
      })}
    </colgroup>
  );
}

interface TorrentTableProps {
  torrents: TorrentListItem[] | null;
  loading: boolean;
}

export const TorrentTable: React.FC<TorrentTableProps> = ({
  torrents,
  loading,
}) => {
  const selectedTorrentIds = useUIStore((state) => state.selectedTorrentIds);
  const selectTorrent = useUIStore((state) => state.selectTorrent);
  const toggleSelection = useUIStore((state) => state.toggleSelection);
  const selectRange = useUIStore((state) => state.selectRange);
  const selectRelative = useUIStore((state) => state.selectRelative);
  const selectAll = useUIStore((state) => state.selectAll);
  const clearSelection = useUIStore((state) => state.clearSelection);
  const searchQuery = useUIStore((state) => state.searchQuery);
  const statusFilter = useUIStore((state) => state.statusFilter);
  const categoryFilter = useUIStore((state) => state.categoryFilter);
  const trackerFilter = useUIStore((state) => state.trackerFilter);
  const setSearchQuery = useUIStore((state) => state.setSearchQuery);
  const setStatusFilter = useUIStore((state) => state.setStatusFilter);
  const setCategoryFilter = useUIStore((state) => state.setCategoryFilter);
  const setTrackerFilter = useUIStore((state) => state.setTrackerFilter);
  const clearFilters = useCallback(() => {
    setSearchQuery("");
    setStatusFilter("all");
    setCategoryFilter(null);
    setTrackerFilter(null);
  }, [setSearchQuery, setStatusFilter, setCategoryFilter, setTrackerFilter]);

  // Subscribe to data directly so component re-renders on changes
  useColumnStore((s) => s.columnVisibility);
  useColumnStore((s) => s.columnWidths);
  useColumnStore((s) => s.columnOrder);
  const visibleColumns = useColumnStore((s) => s.getVisibleColumns)();
  const getWidth = useColumnStore((s) => s.getWidth);
  const setColumnWidth = useColumnStore((s) => s.setColumnWidth);

  const normalizedQuery = searchQuery.toLowerCase().trim();

  // Local sorting state
  const [sortColumn, setSortColumnState] = useState<TableSortColumn>(
    () => loadViewPref("tableSort", DEFAULT_SORT_PREF, isTableSortPref).column,
  );
  const [sortDirection, setSortDirectionState] = useState<SortDirection>(
    () =>
      loadViewPref("tableSort", DEFAULT_SORT_PREF, isTableSortPref).direction,
  );
  useEffect(() => {
    saveViewPref<TableSortPref>("tableSort", {
      column: sortColumn,
      direction: sortDirection,
    });
  }, [sortColumn, sortDirection]);

  // Resize state
  const [resizing, setResizing] = useState<{
    colId: ColumnId;
    startX: number;
    startWidth: number;
  } | null>(null);

  // Column context menu
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
  } | null>(null);

  // Torrent right-click context menu
  const [torrentContextMenu, setTorrentContextMenu] =
    useState<ContextMenuState | null>(null);

  // Keyboard grid: which row is the Tab stop and the one arrow keys act on
  const [activeIndex, setActiveIndex] = useState(0);
  const gridRef = useRef<HTMLDivElement>(null);
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const pendingRowFocus = useRef(false);

  const setSortColumn = useCallback(
    (column: TableSortColumn) => {
      const next = nextSort(
        { column: sortColumn, direction: sortDirection },
        column,
      );
      setSortColumnState(next.column);
      setSortDirectionState(next.direction);
    },
    [sortColumn, sortDirection],
  );

  // Sort and filter torrents for virtualization
  const filteredTorrents = useMemo(() => {
    if (!torrents) return null;

    return [...torrents]
      .filter((t) =>
        isTorrentVisible(
          t,
          normalizedQuery,
          statusFilter,
          categoryFilter,
          trackerFilter,
        ),
      )
      .sort((a, b) => {
        const aVal = getTableSortValue(a, sortColumn);
        const bVal = getTableSortValue(b, sortColumn);
        const cmp =
          typeof aVal === "string"
            ? aVal.localeCompare(bVal as string)
            : (aVal as number) - (bVal as number);
        return sortDirection === "asc" ? cmp : -cmp;
      });
  }, [
    torrents,
    normalizedQuery,
    statusFilter,
    categoryFilter,
    trackerFilter,
    sortColumn,
    sortDirection,
  ]);

  // Compute visible IDs for keyboard navigation
  const visibleTorrentIds = useMemo(() => {
    if (!filteredTorrents) return [];
    return filteredTorrents.map((t) => t.id);
  }, [filteredTorrents]);

  const allSelected = !!(
    visibleTorrentIds.length > 0 &&
    visibleTorrentIds.every((id) => selectedTorrentIds.has(id))
  );
  const someSelected = visibleTorrentIds.some((id) =>
    selectedTorrentIds.has(id),
  );

  const handleHeaderCheckbox = () => {
    if (allSelected) {
      clearSelection();
    } else {
      selectAll(visibleTorrentIds);
    }
  };

  const handleSort = (column: TableSortColumn) => {
    setSortColumn(column);
  };

  // Store orderedIds in a ref so handleRowClick doesn't need it as a dependency
  const orderedIdsRef = useRef<number[]>([]);
  orderedIdsRef.current = visibleTorrentIds;

  // Handle keyboard navigation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // The grid handles its own arrow keys (and calls preventDefault on them)
      if (e.defaultPrevented) return;
      const activeElement = document.activeElement;
      if (
        activeElement &&
        (activeElement.tagName === "INPUT" ||
          activeElement.tagName === "TEXTAREA" ||
          activeElement.tagName === "SELECT")
      ) {
        return;
      }

      if (e.key === "ArrowDown") {
        e.preventDefault();
        selectRelative("down", orderedIdsRef.current);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        selectRelative("up", orderedIdsRef.current);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selectRelative]);

  // Row click handler — skip right-clicks (handled by context menu)
  const handleRowClick = useCallback(
    (id: number, e: React.MouseEvent) => {
      if (e.button === 2) return;
      if (e.shiftKey) {
        e.preventDefault();
        selectRange(id, orderedIdsRef.current);
      } else {
        selectTorrent(id);
      }
    },
    [selectRange, selectTorrent],
  );

  // Opens the torrent menu for a row; shared by right-click and the keyboard (Shift+F10 / ContextMenu)
  const openTorrentMenu = useCallback(
    (id: number, pos: { x: number; y: number; anchor?: DOMRect }) => {
      const torrent = filteredTorrents?.find((t) => t.id === id);
      if (!torrent) return;

      let selected: TorrentListItem[];
      if (selectedTorrentIds.has(id)) {
        // Opened on an already-selected torrent: operate on all selected
        selected = filteredTorrents!.filter((t) =>
          selectedTorrentIds.has(t.id),
        );
      } else {
        // Opened on an unselected torrent: select only this one
        selectTorrent(id);
        selected = [torrent];
      }

      setTorrentContextMenu({
        ...pos,
        torrent,
        selectedTorrents: selected,
      });
    },
    [filteredTorrents, selectedTorrentIds, selectTorrent],
  );

  // Row right-click handler
  const handleRowContextMenu = useCallback(
    (id: number, e: React.MouseEvent) => {
      openTorrentMenu(id, { x: e.clientX, y: e.clientY });
    },
    [openTorrentMenu],
  );

  const handleRowFocus = useCallback((index: number) => {
    setActiveIndex(index);
    pendingRowFocus.current = false;
  }, []);

  const consumeRowFocus = useCallback(() => {
    const pending = pendingRowFocus.current;
    pendingRowFocus.current = false;
    return pending;
  }, []);

  // Grid keys: arrows move (and select) the row, Space toggles, Enter opens details, Shift+F10 / ContextMenu opens the menu
  const handleGridKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    if (target.getAttribute("role") !== "row") return;
    const ids = orderedIdsRef.current;
    if (ids.length === 0) return;

    const current = Number(target.dataset.rowIndex);
    const id = ids[current];
    const moveTo = (index: number) => {
      e.preventDefault();
      if (index === current) return;
      // Ask the row that becomes active to take focus once it has rendered
      pendingRowFocus.current = true;
      setActiveIndex(index);
      selectTorrent(ids[index]);
      virtuosoRef.current?.scrollIntoView({ index });
    };
    const openMenuFromKeyboard = () => {
      e.preventDefault();
      const rect = target.getBoundingClientRect();
      openTorrentMenu(id, { x: rect.left, y: rect.bottom, anchor: rect });
    };

    switch (e.key) {
      case "ArrowDown":
        moveTo(Math.min(current + 1, ids.length - 1));
        break;
      case "ArrowUp":
        moveTo(Math.max(current - 1, 0));
        break;
      case "Home":
        moveTo(0);
        break;
      case "End":
        moveTo(ids.length - 1);
        break;
      case " ":
        e.preventDefault();
        toggleSelection(id);
        break;
      case "Enter":
        e.preventDefault();
        selectTorrent(id);
        break;
      case "ContextMenu":
        openMenuFromKeyboard();
        break;
      case "F10":
        if (e.shiftKey) openMenuFromKeyboard();
        break;
    }
  };

  // Width of a column as rendered. Flex columns report 0 in the store, so fall back to the header cell
  const currentColumnWidth = useCallback(
    (colId: ColumnId, handle: HTMLElement) =>
      getWidth(colId) ||
      handle.closest("th")?.getBoundingClientRect().width ||
      0,
    [getWidth],
  );

  // Column resize: pointer capture keeps the drag going outside the handle, and
  // losing capture (or a cancelled pointer) ends it
  useEffect(() => {
    if (!resizing) return;

    const handlePointerMove = (e: PointerEvent) => {
      const delta = e.clientX - resizing.startX;
      setColumnWidth(resizing.colId, resizing.startWidth + delta);
    };

    document.addEventListener("pointermove", handlePointerMove);
    // Prevent text selection while resizing
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";

    return () => {
      document.removeEventListener("pointermove", handlePointerMove);
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
  }, [resizing, setColumnWidth]);

  const handleResizeStart = useCallback(
    (colId: ColumnId, e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      e.stopPropagation();
      e.currentTarget.setPointerCapture(e.pointerId);
      setResizing({
        colId,
        startX: e.clientX,
        startWidth: currentColumnWidth(colId, e.currentTarget),
      });
    },
    [currentColumnWidth],
  );

  const handleResizeKey = useCallback(
    (colId: ColumnId, e: React.KeyboardEvent<HTMLDivElement>) => {
      const step =
        e.key === "ArrowRight"
          ? RESIZE_STEP_PX
          : e.key === "ArrowLeft"
            ? -RESIZE_STEP_PX
            : 0;
      if (step === 0) return;
      e.preventDefault();
      setColumnWidth(colId, currentColumnWidth(colId, e.currentTarget) + step);
    },
    [currentColumnWidth, setColumnWidth],
  );

  const handleHeaderContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY });
  }, []);

  // The list can shrink under the active row (filter, search), so clamp it into range
  const focusIndex = Math.min(
    activeIndex,
    Math.max(0, (filteredTorrents?.length ?? 0) - 1),
  );

  // Item renderer for react-virtuoso
  const itemContent = useCallback(
    (index: number) => {
      const torrent = filteredTorrents![index];
      return (
        <TorrentTableRow
          key={torrent.id}
          torrent={torrent}
          isSelected={selectedTorrentIds.has(torrent.id)}
          odd={index % 2 === 1}
          rowIndex={index}
          active={index === focusIndex}
          consumeRowFocus={consumeRowFocus}
          onRowFocus={handleRowFocus}
          onRowClick={handleRowClick}
          onContextMenu={handleRowContextMenu}
          onCheckboxChange={toggleSelection}
          visibleColumns={visibleColumns}
        />
      );
    },
    [
      filteredTorrents,
      selectedTorrentIds,
      focusIndex,
      consumeRowFocus,
      handleRowFocus,
      handleRowClick,
      handleRowContextMenu,
      toggleSelection,
      visibleColumns,
    ],
  );

  if (loading) {
    return (
      <div className="flex justify-center items-center h-64">
        <Spinner />
      </div>
    );
  }

  if (!torrents || torrents.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-64 gap-1 px-4 text-center text-tertiary">
        <p className="text-lg">No torrents yet</p>
        <p>
          Paste a magnet link or drop a .torrent file anywhere on this page to
          add one.
        </p>
      </div>
    );
  }

  // Torrents exist, but the search or a filter hides all of them
  if (filteredTorrents && filteredTorrents.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-64 gap-2 px-4 text-center text-tertiary">
        <p className="text-lg">No matches</p>
        <p>No torrents match the current search or filters.</p>
        <Button variant="secondary" onClick={clearFilters}>
          Clear filters
        </Button>
      </div>
    );
  }

  return (
    <>
      <div
        ref={gridRef}
        role="grid"
        aria-label="Torrents"
        aria-rowcount={(filteredTorrents?.length ?? 0) + 1}
        aria-colcount={visibleColumns.length}
        onKeyDown={handleGridKeyDown}
        className="flex flex-col h-full"
      >
        {/* Header */}
        <table className="w-full table-fixed" role="presentation">
          <TableColGroup columns={visibleColumns} />
          <thead className="bg-surface-raised text-sm" role="presentation">
            <tr
              role="row"
              aria-rowindex={1}
              className="border-b border-divider"
              onContextMenu={handleHeaderContextMenu}
            >
              {visibleColumns.map((col) => {
                if (col.id === "checkbox") {
                  return (
                    <th
                      key="checkbox"
                      role="columnheader"
                      className="px-2 py-3 border-r border-divider/40"
                    >
                      <input
                        type="checkbox"
                        aria-label="Select all torrents"
                        checked={allSelected}
                        ref={(el) => {
                          if (el)
                            el.indeterminate = someSelected && !allSelected;
                        }}
                        onChange={handleHeaderCheckbox}
                        className="w-4 h-4 rounded border-divider-strong bg-surface text-primary focus:ring-primary"
                      />
                    </th>
                  );
                }
                if (col.id === "status_icon") {
                  return (
                    <th
                      key="status_icon"
                      role="columnheader"
                      className="px-1 py-3 border-r border-divider/40"
                    >
                      <span className="sr-only">Status</span>
                    </th>
                  );
                }

                const alignClass =
                  col.align === "center"
                    ? "text-center"
                    : col.align === "right"
                      ? "text-right"
                      : "text-left";
                const isSortable = col.sortable;
                const canResize = col.configurable;
                const ariaSort = !isSortable
                  ? undefined
                  : sortColumn !== col.id
                    ? "none"
                    : sortDirection === "asc"
                      ? "ascending"
                      : "descending";

                return (
                  <th
                    key={col.id}
                    role="columnheader"
                    aria-sort={ariaSort}
                    className={`relative px-2 py-2 text-secondary select-none whitespace-nowrap border-r border-divider/40 ${alignClass}`}
                  >
                    {isSortable ? (
                      <button
                        type="button"
                        onClick={() => handleSort(col.id as TableSortColumn)}
                        className="inline-flex items-center gap-0.5 rounded focus-ring cursor-pointer hover:text-text"
                      >
                        {col.label}
                        <SortIcon
                          column={col.id}
                          sortColumn={sortColumn}
                          sortDirection={sortDirection}
                        />
                      </button>
                    ) : (
                      col.label
                    )}
                    {canResize && (
                      <div
                        role="separator"
                        aria-orientation="vertical"
                        aria-label={`Resize ${col.label} column`}
                        tabIndex={0}
                        className="group absolute right-0 top-0 bottom-0 w-[12px] translate-x-1/2 cursor-col-resize touch-none z-10 focus-ring"
                        onPointerDown={(e) =>
                          handleResizeStart(col.id as ColumnId, e)
                        }
                        onPointerCancel={() => setResizing(null)}
                        onLostPointerCapture={() => setResizing(null)}
                        onKeyDown={(e) =>
                          handleResizeKey(col.id as ColumnId, e)
                        }
                      >
                        {/* Visible 1.5px line; the 12px box above is the hit area */}
                        <span
                          aria-hidden="true"
                          className="pointer-events-none absolute inset-y-0 left-1/2 w-[1.5px] -translate-x-1/2 bg-transparent group-hover:bg-primary/40"
                        />
                      </div>
                    )}
                  </th>
                );
              })}
            </tr>
          </thead>
        </table>
        {/* Virtualized body */}
        <div className="flex-1 min-h-0">
          <Virtuoso
            ref={virtuosoRef}
            totalCount={filteredTorrents?.length ?? 0}
            itemContent={itemContent}
          />
        </div>
      </div>

      {/* Menus sit outside the grid: they are not part of its rows */}
      {/* Column visibility context menu */}
      {contextMenu && (
        <ColumnMenu
          x={contextMenu.x}
          y={contextMenu.y}
          onClose={() => setContextMenu(null)}
        />
      )}

      {/* Torrent right-click context menu */}
      {torrentContextMenu && (
        <TorrentContextMenu
          menu={torrentContextMenu}
          onClose={() => setTorrentContextMenu(null)}
        />
      )}
    </>
  );
};

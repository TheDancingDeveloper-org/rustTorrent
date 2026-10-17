import { TorrentListItem, STATE_INITIALIZING } from "../../api-types";
import { StatusIcon } from "../StatusIcon";
import { formatBytes } from "../../helper/formatBytes";
import { formatSecondsToTime } from "../../helper/formatSecondsToTime";
import { formatUnixDate } from "../../helper/formatUnixDate";
import { getCompletionETA } from "../../helper/getCompletionETA";
import { torrentTrackerHosts } from "../../helper/torrentFilters";
import { memo, useEffect, useRef } from "react";
import { ColumnDef, ColumnId, useColumnStore } from "../../stores/columnStore";

interface TorrentTableRowProps {
  torrent: TorrentListItem;
  isSelected: boolean;
  odd?: boolean;
  /** Position in the sorted list; the header row is row 1 in ARIA terms */
  rowIndex: number;
  /** The one row in the grid that takes Tab focus (roving tabindex) */
  active: boolean;
  /** Returns true once if a keyboard move is waiting for the active row to take focus */
  consumeRowFocus: () => boolean;
  onRowFocus: (index: number) => void;
  onRowClick: (id: number, e: React.MouseEvent) => void;
  onContextMenu: (id: number, e: React.MouseEvent) => void;
  onCheckboxChange: (id: number) => void;
  visibleColumns: ColumnDef[];
}

/** Human label + badge styling for the torrent state column */
function stateBadge(
  state: string,
  finished: boolean,
  error: string | null,
  queued: boolean,
): { label: string; cls: string } {
  if (error || state === "error")
    return { label: "Error", cls: "bg-error/15 text-error" };
  if (queued)
    return { label: "Queued", cls: "bg-surface-sunken text-secondary" };
  if (state === "initializing")
    return { label: "Checking", cls: "bg-warning/15 text-warning" };
  if (state === "paused")
    return { label: "Paused", cls: "bg-surface-sunken text-tertiary" };
  if (state === "live" && finished)
    return { label: "Seeding", cls: "bg-success/15 text-success" };
  if (state === "live")
    return {
      label: "Downloading",
      cls: "bg-accent-download/15 text-accent-download",
    };
  return { label: state || "—", cls: "bg-surface-sunken text-tertiary" };
}

/** Shared colgroup matching the header */
function RowColGroup({ columns }: { columns: ColumnDef[] }) {
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

const TorrentTableRowUnmemoized: React.FC<TorrentTableRowProps> = ({
  torrent,
  isSelected,
  odd,
  rowIndex,
  active,
  consumeRowFocus,
  onRowFocus,
  onRowClick,
  onContextMenu,
  onCheckboxChange,
  visibleColumns,
}) => {
  const rowRef = useRef<HTMLTableRowElement>(null);

  // A keyboard move asks the new active row to take focus. The row may not be mounted yet
  // (Virtuoso renders it after scrolling), and the previous row may have unmounted with focus
  // on the body, so the request is consumed by whichever active row mounts or updates.
  // Rows that scroll into view without a request don't steal focus.
  useEffect(() => {
    if (!active || !rowRef.current) return;
    if (consumeRowFocus()) rowRef.current.focus({ preventScroll: true });
  }, [active, consumeRowFocus]);
  const stats = torrent.stats;
  const state = stats?.state ?? "";
  const error = stats?.error ?? null;
  const totalBytes = stats?.total_bytes ?? 1;
  const progressBytes = stats?.progress_bytes ?? 0;
  const finished = stats?.finished || false;
  const live = !!stats?.live;

  const progressPercentage = error
    ? 100
    : totalBytes === 0
      ? 100
      : Math.round((progressBytes / totalBytes) * 100);

  const downloadSpeed = stats?.live?.download_speed?.human_readable ?? "-";
  const uploadSpeed = stats?.live?.upload_speed?.human_readable ?? "-";
  const uploadedBytes = stats?.live?.snapshot.uploaded_bytes ?? 0;

  const peerStats = stats?.live?.snapshot.peer_stats;
  const peersDisplay = peerStats ? `${peerStats.live}/${peerStats.seen}` : "-";

  const eta = stats ? getCompletionETA(stats) : "-";
  const displayEta = finished ? "Done" : eta;

  const name = torrent.name ?? "";

  const handleRowClick = (e: React.MouseEvent) => {
    onRowClick(torrent.id, e);
  };

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    onContextMenu(torrent.id, e);
  };

  const handleCheckboxClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    onCheckboxChange(torrent.id);
  };

  const cellBorder = "border-r border-divider/40";

  function renderCell(col: ColumnDef): React.ReactNode {
    const alignClass =
      col.align === "center"
        ? "text-center"
        : col.align === "right"
          ? "text-right"
          : "text-left";
    const baseCls = `px-2 align-middle whitespace-nowrap ${cellBorder}`;

    switch (col.id as ColumnId) {
      case "checkbox":
        return (
          <td
            role="gridcell"
            key="checkbox"
            className={`px-2 align-middle text-center ${cellBorder}`}
            onMouseDown={handleCheckboxClick}
          >
            <input
              type="checkbox"
              tabIndex={-1}
              aria-label="Select torrent"
              checked={isSelected}
              onChange={() => {}}
              className="w-4 h-4 rounded border-divider-strong bg-surface text-primary focus:ring-primary"
            />
          </td>
        );
      case "status_icon":
        return (
          <td
            role="gridcell"
            key="status_icon"
            className={`px-1 align-middle ${cellBorder}`}
          >
            <StatusIcon
              className="w-5 h-5"
              error={!!error}
              live={live}
              finished={finished}
              queued={stats?.queue_state === "Queued"}
            />
          </td>
        );
      case "id":
        return (
          <td
            role="gridcell"
            key="id"
            className={`${baseCls} text-center text-tertiary font-mono`}
          >
            {torrent.id}
          </td>
        );
      case "name":
        return (
          <td
            role="gridcell"
            key="name"
            className={`px-2 align-middle ${alignClass} ${cellBorder}`}
          >
            <div className="truncate" title={name}>
              {name || "Loading..."}
            </div>
            {error && (
              <div className="truncate text-sm text-error" title={error}>
                {error}
              </div>
            )}
          </td>
        );
      case "size":
        return (
          <td
            role="gridcell"
            key="size"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {formatBytes(totalBytes)}
          </td>
        );
      case "progress":
        return (
          <td
            role="gridcell"
            key="progress"
            className={`px-2 align-middle text-center ${cellBorder}`}
          >
            <div className="flex items-center gap-2">
              <div className="flex-1 h-2 bg-divider rounded-full overflow-hidden">
                <div
                  className={`h-full rounded-full transition-[width] duration-500 ${
                    error
                      ? "bg-error-bg"
                      : finished
                        ? "bg-success-bg"
                        : state === STATE_INITIALIZING
                          ? "bg-warning-bg"
                          : "bg-accent-download"
                  }`}
                  style={{ width: `${progressPercentage}%` }}
                />
              </div>
              <span className="text-sm text-secondary w-8 text-right tabular-nums">
                {progressPercentage}%
              </span>
            </div>
          </td>
        );
      case "downloadedBytes":
        return (
          <td
            role="gridcell"
            key="downloadedBytes"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {formatBytes(progressBytes)}
          </td>
        );
      case "downSpeed": {
        const active = (stats?.live?.download_speed?.mbps ?? 0) > 0.01;
        return (
          <td
            role="gridcell"
            key="downSpeed"
            className={`${baseCls} ${alignClass} tabular-nums ${
              active ? "text-accent-download font-medium" : "text-secondary"
            }`}
          >
            {downloadSpeed}
          </td>
        );
      }
      case "upSpeed": {
        const active = (stats?.live?.upload_speed?.mbps ?? 0) > 0.01;
        return (
          <td
            role="gridcell"
            key="upSpeed"
            className={`${baseCls} ${alignClass} tabular-nums ${
              active ? "text-accent-upload font-medium" : "text-secondary"
            }`}
          >
            {uploadSpeed}
          </td>
        );
      }
      case "uploadedBytes":
        return (
          <td
            role="gridcell"
            key="uploadedBytes"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {uploadedBytes > 0 ? formatBytes(uploadedBytes) : ""}
          </td>
        );
      case "eta":
        return (
          <td
            role="gridcell"
            key="eta"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {displayEta}
          </td>
        );
      case "peers":
        return (
          <td
            role="gridcell"
            key="peers"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {peersDisplay}
          </td>
        );
      case "state": {
        const badge = stateBadge(
          state,
          finished,
          error,
          stats?.queue_state === "Queued",
        );
        return (
          <td
            role="gridcell"
            key="state"
            className={`${baseCls} ${alignClass}`}
          >
            <span
              className={`inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium ${badge.cls}`}
            >
              {badge.label}
            </span>
          </td>
        );
      }
      case "info_hash":
        return (
          <td
            role="gridcell"
            key="info_hash"
            className={`px-2 align-middle ${alignClass} font-mono text-xs text-tertiary ${cellBorder}`}
          >
            <div className="truncate" title={torrent.info_hash}>
              {torrent.info_hash}
            </div>
          </td>
        );
      case "ratio": {
        const ratio = stats?.ratio;
        const ratioDisplay =
          ratio != null
            ? ratio.toFixed(2)
            : totalBytes > 0
              ? (uploadedBytes / totalBytes).toFixed(2)
              : "0.00";
        return (
          <td
            role="gridcell"
            key="ratio"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {ratioDisplay}
          </td>
        );
      }
      case "category":
        return (
          <td
            role="gridcell"
            key="category"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            <span className="truncate">{torrent.category || "\u2014"}</span>
          </td>
        );
      case "seeding_time": {
        const seedTime = stats?.seeding_time_secs;
        return (
          <td
            role="gridcell"
            key="seeding_time"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {seedTime != null ? formatSecondsToTime(seedTime) : "\u2014"}
          </td>
        );
      }
      case "queue_position": {
        const queueState = stats?.queue_state;
        const queuePos = stats?.queue_position;
        let queueDisplay: string;
        if (queueState === "Queued" && queuePos != null) {
          queueDisplay = `#${queuePos}`;
        } else if (queueState === "Active") {
          queueDisplay = "Active";
        } else {
          queueDisplay = "\u2014";
        }
        return (
          <td
            role="gridcell"
            key="queue_position"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {queueDisplay}
          </td>
        );
      }
      case "sequential":
        return (
          <td
            role="gridcell"
            key="sequential"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {stats?.sequential ? "\u2713" : "\u2014"}
          </td>
        );
      case "availability": {
        const avail = stats?.min_piece_availability;
        return (
          <td
            role="gridcell"
            key="availability"
            className={`${baseCls} ${alignClass} text-secondary`}
          >
            {avail != null ? avail.toFixed(1) : "\u2014"}
          </td>
        );
      }
      case "added_on":
        return (
          <td
            role="gridcell"
            key="added_on"
            className={`${baseCls} ${alignClass} text-secondary tabular-nums`}
          >
            {formatUnixDate(torrent.added_on)}
          </td>
        );
      case "tracker": {
        const hosts = torrentTrackerHosts(torrent);
        return (
          <td
            role="gridcell"
            key="tracker"
            className={`${baseCls} ${alignClass} text-secondary`}
            title={hosts.join(", ")}
          >
            <span className="truncate">
              {hosts.length === 0
                ? "\u2014"
                : hosts.length === 1
                  ? hosts[0]
                  : `${hosts[0]} +${hosts.length - 1}`}
            </span>
          </td>
        );
      }
      default:
        return <td role="gridcell" key={col.id} className={baseCls} />;
    }
  }

  return (
    // Presentation wrappers: the grid in TorrentTable supplies the table semantics
    <table className="w-full table-fixed" role="presentation">
      <RowColGroup columns={visibleColumns} />
      <tbody role="presentation">
        <tr
          ref={rowRef}
          role="row"
          aria-selected={isSelected}
          aria-rowindex={rowIndex + 2}
          data-row-index={rowIndex}
          tabIndex={active ? 0 : -1}
          onFocus={(e) => {
            if (e.target === e.currentTarget) onRowFocus(rowIndex);
          }}
          onMouseDown={handleRowClick}
          onContextMenu={handleContextMenu}
          className={`focus-ring cursor-pointer border-b border-divider/60 text-sm h-8 pointer-coarse:h-11 transition-colors ${
            isSelected
              ? "bg-primary/15"
              : `${odd ? "bg-surface-sunken/40" : ""} hover:bg-primary/5`
          }`}
        >
          {visibleColumns.map((col) => renderCell(col))}
        </tr>
      </tbody>
    </table>
  );
};

export const TorrentTableRow = memo(TorrentTableRowUnmemoized);

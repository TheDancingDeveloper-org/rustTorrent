import {
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { PeerStats, PeerStatsSnapshot, TorrentListItem } from "../../api-types";
import { APIContext } from "../../context";
import { formatBytes } from "../../helper/formatBytes";
import { SortIcon } from "../SortIcon";
import { customSetInterval } from "../../helper/customSetInterval";
import { nextSort } from "../../helper/sortToggle";

interface PeersTabProps {
  torrent: TorrentListItem | null;
}

interface PeerWithSpeed {
  addr: string;
  stats: PeerStats;
  downloadSpeed: number;
  uploadSpeed: number;
}

interface SpeedSample {
  downloadSpeed: number;
  uploadSpeed: number;
  timestamp: number;
}

const SPEED_AVERAGE_WINDOW_MS = 5000;

type PeerSortColumn =
  | "addr"
  | "connKind"
  | "downloadSpeed"
  | "uploadSpeed"
  | "downloaded"
  | "uploaded";
type PeerSortDirection = "asc" | "desc";

const formatSpeed = (bytesPerSecond: number): string => {
  if (bytesPerSecond === 0) return "-";
  return formatBytes(bytesPerSecond) + "/s";
};

export const PeersTab: React.FC<PeersTabProps> = ({ torrent }) => {
  const torrentId = torrent?.id;
  const statsResponse = torrent?.stats ?? null;
  const API = useContext(APIContext);
  const [peerSnapshot, setPeerSnapshot] = useState<PeerStatsSnapshot | null>(
    null,
  );
  const [peersWithSpeed, setPeersWithSpeed] = useState<PeerWithSpeed[]>([]);
  const [sortColumn, setSortColumn] = useState<PeerSortColumn>("downloadSpeed");
  const [sortDirection, setSortDirection] = useState<PeerSortDirection>("desc");

  const prevSnapshotRef = useRef<{
    snapshot: PeerStatsSnapshot;
    timestamp: number;
  } | null>(null);
  // Track speed samples per peer for averaging
  const speedHistoryRef = useRef<Map<string, SpeedSample[]>>(new Map());

  // Fetch peer stats periodically
  useEffect(() => {
    if (torrentId == null || !statsResponse?.live) return;

    return customSetInterval(() => {
      return API.getPeerStats(torrentId).then(
        (stats) => {
          setPeerSnapshot(stats);
          return 1000;
        },
        (err) => {
          console.error(err);
          return 5000;
        },
      );
    }, 0);
  }, [torrentId, !!statsResponse?.live]);

  // Compute speeds when snapshot changes
  useEffect(() => {
    if (!peerSnapshot) {
      setPeersWithSpeed([]);
      return;
    }

    const now = Date.now();
    const prev = prevSnapshotRef.current;
    const speedHistory = speedHistoryRef.current;

    const newPeers: PeerWithSpeed[] = Object.entries(peerSnapshot.peers).map(
      ([addr, stats]) => {
        let instantDownloadSpeed = 0;
        let instantUploadSpeed = 0;

        // Compute instantaneous speed from prev snapshot
        if (prev && prev.snapshot.peers[addr]) {
          const timeDiffSec = (now - prev.timestamp) / 1000;
          if (timeDiffSec > 0) {
            const prevStats = prev.snapshot.peers[addr];
            const downloadedDiff =
              stats.counters.fetched_bytes - prevStats.counters.fetched_bytes;
            const uploadedDiff =
              stats.counters.uploaded_bytes - prevStats.counters.uploaded_bytes;
            instantDownloadSpeed = Math.max(0, downloadedDiff / timeDiffSec);
            instantUploadSpeed = Math.max(0, uploadedDiff / timeDiffSec);
          }
        }

        // Add sample to history
        let history = speedHistory.get(addr) ?? [];
        history.push({
          downloadSpeed: instantDownloadSpeed,
          uploadSpeed: instantUploadSpeed,
          timestamp: now,
        });
        // Remove old samples outside the averaging window
        history = history.filter(
          (s) => now - s.timestamp <= SPEED_AVERAGE_WINDOW_MS,
        );
        speedHistory.set(addr, history);

        // Compute average speed over the window
        let avgDownloadSpeed = 0;
        let avgUploadSpeed = 0;
        if (history.length > 0) {
          avgDownloadSpeed =
            history.reduce((sum, s) => sum + s.downloadSpeed, 0) /
            history.length;
          avgUploadSpeed =
            history.reduce((sum, s) => sum + s.uploadSpeed, 0) / history.length;
        }

        return {
          addr,
          stats,
          downloadSpeed: avgDownloadSpeed,
          uploadSpeed: avgUploadSpeed,
        };
      },
    );

    // Clean up history for peers that are no longer present
    for (const addr of speedHistory.keys()) {
      if (!peerSnapshot.peers[addr]) {
        speedHistory.delete(addr);
      }
    }

    setPeersWithSpeed(newPeers);
    prevSnapshotRef.current = { snapshot: peerSnapshot, timestamp: now };
  }, [peerSnapshot]);

  const handleSort = useCallback(
    (column: PeerSortColumn) => {
      const next = nextSort(
        { column: sortColumn, direction: sortDirection },
        column,
      );
      setSortColumn(next.column);
      setSortDirection(next.direction);
    },
    [sortColumn, sortDirection],
  );

  const sortedPeers = useMemo(() => {
    return [...peersWithSpeed].sort((a, b) => {
      let cmp = 0;
      switch (sortColumn) {
        case "addr":
          cmp = a.addr.localeCompare(b.addr);
          break;
        case "connKind":
          cmp = (a.stats.conn_kind ?? "").localeCompare(
            b.stats.conn_kind ?? "",
          );
          break;
        case "downloadSpeed":
          cmp = a.downloadSpeed - b.downloadSpeed;
          break;
        case "uploadSpeed":
          cmp = a.uploadSpeed - b.uploadSpeed;
          break;
        case "downloaded":
          cmp = a.stats.counters.fetched_bytes - b.stats.counters.fetched_bytes;
          break;
        case "uploaded":
          cmp =
            a.stats.counters.uploaded_bytes - b.stats.counters.uploaded_bytes;
          break;
      }
      return sortDirection === "asc" ? cmp : -cmp;
    });
  }, [peersWithSpeed, sortColumn, sortDirection]);

  if (!statsResponse) {
    return <div className="p-4 text-tertiary">Loading...</div>;
  }

  if (!statsResponse.live) {
    return (
      <div className="p-4 text-tertiary">
        No live peer information available (torrent is not live)
      </div>
    );
  }

  const headerClass =
    "px-2 py-1.5 text-left text-sm text-secondary select-none";

  return (
    <div className="p-3">
      <div className="overflow-x-auto">
        <table className="min-w-full">
          <thead>
            <tr className="border-b border-divider">
              <th
                className={headerClass}
                aria-sort={
                  sortColumn !== "addr"
                    ? "none"
                    : sortDirection === "asc"
                      ? "ascending"
                      : "descending"
                }
              >
                <button
                  type="button"
                  onClick={() => handleSort("addr")}
                  className="inline-flex items-center gap-0.5 rounded focus-ring hover:text-text"
                >
                  IP Address
                  <SortIcon
                    column="addr"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                  />
                </button>
              </th>
              <th
                className={headerClass}
                aria-sort={
                  sortColumn !== "connKind"
                    ? "none"
                    : sortDirection === "asc"
                      ? "ascending"
                      : "descending"
                }
              >
                <button
                  type="button"
                  onClick={() => handleSort("connKind")}
                  className="inline-flex items-center gap-0.5 rounded focus-ring hover:text-text"
                >
                  Type
                  <SortIcon
                    column="connKind"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                  />
                </button>
              </th>
              <th
                className={headerClass}
                aria-sort={
                  sortColumn !== "downloadSpeed"
                    ? "none"
                    : sortDirection === "asc"
                      ? "ascending"
                      : "descending"
                }
              >
                <button
                  type="button"
                  onClick={() => handleSort("downloadSpeed")}
                  className="inline-flex items-center gap-0.5 rounded focus-ring hover:text-text"
                >
                  Down Speed
                  <SortIcon
                    column="downloadSpeed"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                  />
                </button>
              </th>
              <th
                className={headerClass}
                aria-sort={
                  sortColumn !== "uploadSpeed"
                    ? "none"
                    : sortDirection === "asc"
                      ? "ascending"
                      : "descending"
                }
              >
                <button
                  type="button"
                  onClick={() => handleSort("uploadSpeed")}
                  className="inline-flex items-center gap-0.5 rounded focus-ring hover:text-text"
                >
                  Up Speed
                  <SortIcon
                    column="uploadSpeed"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                  />
                </button>
              </th>
              <th
                className={headerClass}
                aria-sort={
                  sortColumn !== "downloaded"
                    ? "none"
                    : sortDirection === "asc"
                      ? "ascending"
                      : "descending"
                }
              >
                <button
                  type="button"
                  onClick={() => handleSort("downloaded")}
                  className="inline-flex items-center gap-0.5 rounded focus-ring hover:text-text"
                >
                  Downloaded
                  <SortIcon
                    column="downloaded"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                  />
                </button>
              </th>
              <th
                className={headerClass}
                aria-sort={
                  sortColumn !== "uploaded"
                    ? "none"
                    : sortDirection === "asc"
                      ? "ascending"
                      : "descending"
                }
              >
                <button
                  type="button"
                  onClick={() => handleSort("uploaded")}
                  className="inline-flex items-center gap-0.5 rounded focus-ring hover:text-text"
                >
                  Uploaded
                  <SortIcon
                    column="uploaded"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                  />
                </button>
              </th>
            </tr>
          </thead>
          <tbody>
            {sortedPeers.length === 0 ? (
              <tr>
                <td
                  colSpan={6}
                  className="px-2 py-3 text-center text-sm text-tertiary"
                >
                  {peerSnapshot === null
                    ? "Loading peer list..."
                    : "No connected peers"}
                </td>
              </tr>
            ) : (
              sortedPeers.map((peer) => (
                <tr key={peer.addr} className="hover:bg-surface-raised">
                  <td className="px-2 py-1 text-sm font-mono">{peer.addr}</td>
                  <td className="px-2 py-1 text-sm text-secondary">
                    {peer.stats.conn_kind ?? "-"}
                  </td>
                  <td className="px-2 py-1 text-sm text-success">
                    {formatSpeed(peer.downloadSpeed)}
                  </td>
                  <td className="px-2 py-1 text-sm text-primary">
                    {formatSpeed(peer.uploadSpeed)}
                  </td>
                  <td className="px-2 py-1 text-sm text-secondary">
                    {formatBytes(peer.stats.counters.fetched_bytes)}
                  </td>
                  <td className="px-2 py-1 text-sm text-secondary">
                    {formatBytes(peer.stats.counters.uploaded_bytes)}
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
};

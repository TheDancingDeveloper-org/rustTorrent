import { useContext, useEffect, useState } from "react";
import { APIContext } from "../context";
import { DhtStats, SessionStats } from "../api-types";
import { useStatsStore } from "../stores/statsStore";
import { formatBytes } from "../helper/formatBytes";
import { formatSecondsToTime } from "../helper/formatSecondsToTime";

function dhtNodeCount(stats: DhtStats | null): number | null {
  if (!stats) return null;
  const v4 = Number(stats.routing_table_size ?? 0);
  const v6 = Number(stats.routing_table_size_v6 ?? 0);
  if (!Number.isFinite(v4) || !Number.isFinite(v6)) return null;
  return v4 + v6;
}

function ratio(uploaded: number, downloaded: number): string | null {
  if (downloaded <= 0) return null;
  return (uploaded / downloaded).toFixed(2);
}

const Stat: React.FC<{
  label: string;
  value: string;
  hint?: string;
}> = ({ label, value, hint }) => (
  <div className="rounded-lg border border-divider bg-surface-raised px-4 py-3">
    <div className="text-xs text-tertiary">{label}</div>
    <div className="mt-1 text-lg font-medium tabular-nums text-text">{value}</div>
    {hint && <div className="mt-1 text-xs text-tertiary">{hint}</div>}
  </div>
);

const Unavailable: React.FC<{ label: string; reason: string }> = ({
  label,
  reason,
}) => (
  <div className="rounded-lg border border-dashed border-divider px-4 py-3">
    <div className="text-xs text-tertiary">{label}</div>
    <div className="mt-1 text-sm text-secondary">Not available</div>
    <div className="mt-1 text-xs text-tertiary">{reason}</div>
  </div>
);

export const StatisticsPage: React.FC = () => {
  const API = useContext(APIContext);
  const stats = useStatsStore((state) => state.stats);
  const [dht, setDht] = useState<DhtStats | null>(null);
  const [dhtError, setDhtError] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      API.stats()
        .then((next: SessionStats) => {
          if (!cancelled) useStatsStore.getState().setStats(next);
        })
        .catch(() => {});
      API.getDhtStats()
        .then((next) => {
          if (cancelled) return;
          setDht(next);
          setDhtError(false);
        })
        .catch(() => {
          if (!cancelled) setDhtError(true);
        });
    };
    poll();
    const interval = setInterval(poll, 5000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [API]);

  const sessionRatio = ratio(
    stats.counters.uploaded_bytes,
    stats.counters.fetched_bytes,
  );
  const nodes = dhtNodeCount(dht);

  return (
    <div className="h-full overflow-auto p-4 sm:p-6">
      <h2 className="text-lg font-semibold text-text">Statistics</h2>
      <p className="mt-1 text-sm text-secondary">
        Session totals from GET /stats. All-time counters, a share ratio that
        survives restarts, and the piece cache are not exposed by the API.
      </p>

      <h3 className="mt-6 mb-2 text-sm font-medium text-secondary">
        Transfer (this session)
      </h3>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <Stat
          label="Downloaded"
          value={formatBytes(stats.counters.fetched_bytes)}
          hint={`${stats.download_speed.human_readable} now`}
        />
        <Stat
          label="Uploaded"
          value={formatBytes(stats.counters.uploaded_bytes)}
          hint={`${stats.upload_speed.human_readable} now`}
        />
        <Stat
          label="Session ratio"
          value={sessionRatio ?? "—"}
          hint={
            sessionRatio
              ? "Uploaded ÷ downloaded since this process started"
              : "No downloaded bytes this session"
          }
        />
        <Stat
          label="Uptime"
          value={formatSecondsToTime(stats.uptime_seconds)}
        />
      </div>

      <h3 className="mt-6 mb-2 text-sm font-medium text-secondary">Peers</h3>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <Stat label="Live" value={String(stats.peers.live)} />
        <Stat label="Connecting" value={String(stats.peers.connecting)} />
        <Stat label="Queued" value={String(stats.peers.queued)} />
        <Stat label="Seen" value={String(stats.peers.seen)} />
        <Stat label="Dead" value={String(stats.peers.dead)} />
        <Stat label="Not needed" value={String(stats.peers.not_needed)} />
        <Stat
          label="DHT nodes"
          value={dhtError ? "n/a" : nodes == null ? "—" : String(nodes)}
          hint="IPv4 + IPv6 routing table"
        />
      </div>

      <h3 className="mt-6 mb-2 text-sm font-medium text-secondary">
        Not exposed by the API
      </h3>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <Unavailable
          label="All-time downloaded"
          reason="GET /stats counters reset when the session starts."
        />
        <Unavailable
          label="All-time uploaded"
          reason="There is no persisted transfer total."
        />
        <Unavailable
          label="All-time ratio"
          reason="Needs persisted all-time up and down totals."
        />
        <Unavailable
          label="Read cache"
          reason="The piece cache does not publish hits, misses or size."
        />
        <Unavailable
          label="Write cache"
          reason="No write-cache occupancy or overload counter is exposed."
        />
      </div>
    </div>
  );
};

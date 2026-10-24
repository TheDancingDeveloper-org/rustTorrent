import { useContext, useEffect, useState } from "react";
import { TorrentLimits } from "../api-types";
import { APIContext } from "../context";

const REFRESH_MS = 10_000;

/**
 * Per-torrent speed limits, keyed by torrent id. The torrent list does not carry
 * them, so they are fetched per torrent from the limits endpoint, only while
 * `enabled` (a limit column is shown) and refreshed every few seconds.
 */
export function useTorrentLimits(
  ids: number[],
  enabled: boolean,
): Map<number, TorrentLimits> {
  const API = useContext(APIContext);
  const [limits, setLimits] = useState<Map<number, TorrentLimits>>(
    () => new Map(),
  );
  const key = enabled ? ids.join(",") : "";

  useEffect(() => {
    if (!key) return;
    const torrentIds = key.split(",").map(Number);
    let cancelled = false;
    const load = async () => {
      const results = await Promise.all(
        torrentIds.map((id) =>
          API.getTorrentLimits(id)
            .then((l): [number, TorrentLimits] => [id, l])
            .catch(() => null),
        ),
      );
      if (cancelled) return;
      setLimits(
        new Map(results.filter((r): r is [number, TorrentLimits] => !!r)),
      );
    };
    load();
    const timer = setInterval(load, REFRESH_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [API, key]);

  return limits;
}

import { formatBytes } from "./formatBytes";

/** A per-torrent rate limit in bytes/s; unset or 0 means unlimited. */
export function formatRateLimit(rate: number | null | undefined): string {
  return rate ? `${formatBytes(rate)}/s` : "∞";
}

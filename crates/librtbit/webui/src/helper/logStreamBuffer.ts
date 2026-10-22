// Buffer and pause rules for the server log view. Kept free of React so the
// behaviour can be unit-tested without a DOM.

export const LOG_LEVELS = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"] as const;

/** Levels shown when the view opens. DEBUG and TRACE stay one click away. */
export const DEFAULT_ENABLED_LEVELS: readonly string[] = [
  "INFO",
  "WARN",
  "ERROR",
];

/** Hard cap on lines held in memory; the oldest are dropped first. */
export const LOG_BUFFER_CAP = 3000;

/**
 * Prepend `incoming` (newest first) to `current` (newest first), keeping at
 * most `cap` lines. Returns `current` itself when there is nothing to add.
 */
export function prependBounded<T>(
  current: readonly T[],
  incoming: readonly T[],
  cap: number,
): T[] {
  if (incoming.length === 0) {
    return current as T[];
  }
  const merged = incoming.concat(current);
  return merged.length > cap ? merged.slice(0, cap) : merged;
}

/**
 * Lines to render. While paused (`pausedAt` is the newest id at the moment of
 * pausing) only lines at or before that id are shown, so the view holds still.
 */
export function visibleLines<T extends { id: number }>(
  lines: readonly T[],
  pausedAt: number | null,
): readonly T[] {
  if (pausedAt === null) {
    return lines;
  }
  return lines.filter((line) => line.id <= pausedAt);
}

/** How many buffered lines arrived after the view was paused. */
export function linesSincePause<T extends { id: number }>(
  lines: readonly T[],
  pausedAt: number | null,
): number {
  if (pausedAt === null) {
    return 0;
  }
  let count = 0;
  for (const line of lines) {
    if (line.id > pausedAt) {
      count++;
    }
  }
  return count;
}

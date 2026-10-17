export const RETRY_MIN_MS = 1000;
export const RETRY_MAX_MS = 30000;

/**
 * Delay before the next refresh after `failures` consecutive failures: 1s, 2s,
 * 4s, ... capped at 30s. Zero or fewer failures give the minimum delay.
 */
export function retryDelayMs(failures: number): number {
  if (failures < 1) return RETRY_MIN_MS;
  return Math.min(RETRY_MIN_MS * 2 ** (failures - 1), RETRY_MAX_MS);
}

// Per-browser view preferences (sort, filters, sidebar) kept in localStorage.
// Storage can be missing or throw (private windows, blocked site data), and a
// stored value can be stale or hand-edited, so reads fall back to the default
// and writes are best-effort.

const PREFIX = "rtbit.view.";

export function loadViewPref<T>(
  key: string,
  fallback: T,
  isValid: (value: unknown) => value is T,
): T {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (raw === null) return fallback;
    const parsed: unknown = JSON.parse(raw);
    return isValid(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
}

export function saveViewPref<T>(key: string, value: T): void {
  try {
    localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    // Storage unavailable: the preference simply does not survive a reload.
  }
}

export function oneOf<T extends string>(
  allowed: readonly T[],
): (value: unknown) => value is T {
  return (value: unknown): value is T =>
    typeof value === "string" && (allowed as readonly string[]).includes(value);
}

export function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

export function isStringOrNull(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}

export function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

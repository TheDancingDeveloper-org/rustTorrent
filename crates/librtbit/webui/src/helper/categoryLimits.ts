export interface CategoryLimitFields {
  ratio: string;
  seedingMinutes: string;
  inactiveMinutes: string;
}

export interface CategoryLimitValues {
  ratio_limit?: number;
  seeding_time_limit?: number;
  inactive_seeding_time_limit?: number;
}

// The daemon serialises a Duration as {secs, nanos}; accept plain seconds too.
type WireSecs = number | { secs: number } | null | undefined;

const toSecs = (v: WireSecs): number | undefined =>
  v == null ? undefined : typeof v === "number" ? v : v.secs;

const minutesText = (v: WireSecs): string => {
  const secs = toSecs(v);
  return secs === undefined ? "" : String(Math.floor(secs / 60));
};

export function limitFieldsFromCategory(cat?: {
  ratio_limit?: number | null;
  seeding_time_limit?: WireSecs;
  inactive_seeding_time_limit?: WireSecs;
}): CategoryLimitFields {
  return {
    ratio: cat?.ratio_limit == null ? "" : String(cat.ratio_limit),
    seedingMinutes: minutesText(cat?.seeding_time_limit),
    inactiveMinutes: minutesText(cat?.inactive_seeding_time_limit),
  };
}

function wholeMinutes(text: string, what: string): number | string | undefined {
  const t = text.trim();
  if (t === "") return undefined;
  if (!/^\d+$/.test(t) || !Number.isSafeInteger(Number(t) * 60)) {
    return `${what} must be a whole number of minutes.`;
  }
  return Number(t) * 60;
}

/** Blank = use global. Returns either the request values or an error message. */
export function parseCategoryLimits(
  f: CategoryLimitFields,
): { values: CategoryLimitValues } | { error: string } {
  const values: CategoryLimitValues = {};
  const r = f.ratio.trim();
  if (r !== "") {
    const n = Number(r);
    if (!Number.isFinite(n)) return { error: "Ratio limit must be a number." };
    if (n < 0) return { error: "Ratio limit must be 0 or more." };
    values.ratio_limit = n;
  }
  const s = wholeMinutes(f.seedingMinutes, "Seeding time limit");
  if (typeof s === "string") return { error: s };
  if (s !== undefined) values.seeding_time_limit = s;
  const i = wholeMinutes(f.inactiveMinutes, "Inactive seeding time limit");
  if (typeof i === "string") return { error: i };
  if (i !== undefined) values.inactive_seeding_time_limit = i;
  return { values };
}

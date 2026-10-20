export type SortToggleDirection = "asc" | "desc";

export interface SortToggleState<C extends string> {
  column: C;
  direction: SortToggleDirection;
}

// Clicking the sorted column flips its direction; clicking another column
// sorts it descending. Pure, so callers can set column and direction from one
// computed value instead of nesting state updaters (StrictMode runs updaters
// twice, which made a nested toggle cancel itself out).
export function nextSort<C extends string>(
  prev: SortToggleState<C>,
  column: C,
): SortToggleState<C> {
  if (prev.column === column) {
    return {
      column,
      direction: prev.direction === "asc" ? "desc" : "asc",
    };
  }
  return { column, direction: "desc" };
}

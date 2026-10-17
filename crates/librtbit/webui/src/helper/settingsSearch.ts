// Pure matching for the settings search, kept free of React so it can be tested.

export interface SearchableSetting {
  label: string;
  tab: string;
}

export interface SearchableTab {
  id: string;
  label: string;
}

/** Settings whose label contains the query, or whose tab's label does. */
export function matchSettings<S extends SearchableSetting>(
  settings: readonly S[],
  tabs: readonly SearchableTab[],
  query: string,
): S[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  const tabLabel = new Map(tabs.map((t) => [t.id, t.label.toLowerCase()]));
  return settings.filter(
    (s) =>
      s.label.toLowerCase().includes(needle) ||
      (tabLabel.get(s.tab) ?? "").includes(needle),
  );
}

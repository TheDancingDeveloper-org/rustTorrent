import { create } from "zustand";

export type ColumnId =
  | "checkbox"
  | "status_icon"
  | "id"
  | "name"
  | "size"
  | "progress"
  | "downloadedBytes"
  | "downSpeed"
  | "upSpeed"
  | "downLimit"
  | "upLimit"
  | "uploadedBytes"
  | "eta"
  | "peers"
  | "state"
  | "info_hash"
  | "ratio"
  | "category"
  | "seeding_time"
  | "queue_position"
  | "sequential"
  | "availability"
  | "added_on"
  | "tracker"
  | "amount_left"
  | "save_path";

// IDs of configurable columns in their default order
const DEFAULT_CONFIGURABLE_ORDER: ColumnId[] = [
  "id",
  "name",
  "size",
  "progress",
  "downloadedBytes",
  "downSpeed",
  "upSpeed",
  "downLimit",
  "upLimit",
  "uploadedBytes",
  "eta",
  "peers",
  "state",
  "info_hash",
  "ratio",
  "category",
  "seeding_time",
  "queue_position",
  "sequential",
  "availability",
  "added_on",
  "tracker",
  "amount_left",
  "save_path",
];

export interface ColumnDef {
  id: ColumnId;
  label: string;
  defaultWidth: number; // 0 means flex (takes remaining space)
  minWidth: number;
  align: "left" | "center" | "right";
  defaultVisible: boolean;
  configurable: boolean; // false for checkbox, status_icon
  sortable: boolean;
}

export const COLUMN_DEFS: ColumnDef[] = [
  {
    id: "checkbox",
    label: "",
    defaultWidth: 32,
    minWidth: 32,
    align: "center",
    defaultVisible: true,
    configurable: false,
    sortable: false,
  },
  {
    id: "status_icon",
    label: "",
    defaultWidth: 32,
    minWidth: 32,
    align: "center",
    defaultVisible: true,
    configurable: false,
    sortable: false,
  },
  {
    id: "id",
    label: "ID",
    defaultWidth: 48,
    minWidth: 36,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "name",
    label: "Name",
    defaultWidth: 0,
    minWidth: 100,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "size",
    label: "Size",
    defaultWidth: 80,
    minWidth: 60,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "progress",
    label: "Progress",
    defaultWidth: 120,
    minWidth: 80,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "downloadedBytes",
    label: "Recv",
    defaultWidth: 80,
    minWidth: 60,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "downSpeed",
    label: "↓ Speed",
    defaultWidth: 80,
    minWidth: 60,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "upSpeed",
    label: "↑ Speed",
    defaultWidth: 80,
    minWidth: 60,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "downLimit",
    label: "Down Limit",
    defaultWidth: 90,
    minWidth: 60,
    align: "center",
    defaultVisible: false,
    configurable: true,
    sortable: true,
  },
  {
    id: "upLimit",
    label: "Up Limit",
    defaultWidth: 90,
    minWidth: 60,
    align: "center",
    defaultVisible: false,
    configurable: true,
    sortable: true,
  },
  {
    id: "uploadedBytes",
    label: "Sent",
    defaultWidth: 80,
    minWidth: 60,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "eta",
    label: "ETA",
    defaultWidth: 80,
    minWidth: 60,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "peers",
    label: "Peers",
    defaultWidth: 64,
    minWidth: 50,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "state",
    label: "State",
    defaultWidth: 100,
    minWidth: 60,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "info_hash",
    label: "Info Hash",
    defaultWidth: 150,
    minWidth: 80,
    align: "center",
    defaultVisible: false,
    configurable: true,
    sortable: false,
  },
  {
    id: "ratio",
    label: "Ratio",
    defaultWidth: 80,
    minWidth: 50,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "category",
    label: "Category",
    defaultWidth: 120,
    minWidth: 60,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "seeding_time",
    label: "Seed Time",
    defaultWidth: 100,
    minWidth: 60,
    align: "center",
    defaultVisible: false,
    configurable: true,
    sortable: true,
  },
  {
    id: "queue_position",
    label: "Queue",
    defaultWidth: 60,
    minWidth: 40,
    align: "center",
    defaultVisible: false,
    configurable: true,
    sortable: true,
  },
  {
    id: "sequential",
    label: "Seq",
    defaultWidth: 50,
    minWidth: 36,
    align: "center",
    defaultVisible: false,
    configurable: true,
    sortable: false,
  },
  {
    id: "availability",
    label: "Avail",
    defaultWidth: 70,
    minWidth: 50,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "added_on",
    label: "Added On",
    defaultWidth: 140,
    minWidth: 90,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "tracker",
    label: "Tracker",
    defaultWidth: 150,
    minWidth: 80,
    align: "center",
    defaultVisible: true,
    configurable: true,
    sortable: true,
  },
  {
    id: "amount_left",
    label: "Remaining",
    defaultWidth: 90,
    minWidth: 60,
    align: "center",
    defaultVisible: false,
    configurable: true,
    sortable: true,
  },
  {
    id: "save_path",
    label: "Save Path",
    defaultWidth: 200,
    minWidth: 80,
    align: "center",
    defaultVisible: false,
    configurable: true,
    sortable: true,
  },
];

const STORAGE_KEY_WIDTHS = "rtbit-column-widths";
const STORAGE_KEY_VISIBLE = "rtbit-column-visible";
const STORAGE_KEY_ORDER = "rtbit-column-order";

function loadJson(key: string): Record<string, any> {
  try {
    const stored = localStorage.getItem(key);
    return stored ? JSON.parse(stored) : {};
  } catch {
    return {};
  }
}

function loadColumnOrder(): ColumnId[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY_ORDER);
    if (stored) {
      const parsed = JSON.parse(stored) as ColumnId[];
      // Ensure all configurable columns are present (in case new columns were added)
      const missing = DEFAULT_CONFIGURABLE_ORDER.filter(
        (id) => !parsed.includes(id),
      );
      return [...parsed, ...missing];
    }
  } catch {
    // ignore
  }
  return [...DEFAULT_CONFIGURABLE_ORDER];
}

export interface ColumnStore {
  columnWidths: Record<string, number>;
  columnVisibility: Record<string, boolean>;
  columnOrder: ColumnId[];

  getWidth: (id: ColumnId) => number;
  isVisible: (id: ColumnId) => boolean;
  getVisibleColumns: () => ColumnDef[];

  setColumnWidth: (id: ColumnId, width: number) => void;
  toggleColumnVisibility: (id: ColumnId) => void;
  moveColumn: (id: ColumnId, direction: "up" | "down") => void;
  resetColumns: () => void;
}

export const useColumnStore = create<ColumnStore>((set, get) => ({
  columnWidths: loadJson(STORAGE_KEY_WIDTHS),
  columnVisibility: loadJson(STORAGE_KEY_VISIBLE),
  columnOrder: loadColumnOrder(),

  getWidth: (id) => {
    const w = get().columnWidths[id];
    if (w !== undefined) return w;
    return COLUMN_DEFS.find((c) => c.id === id)?.defaultWidth ?? 80;
  },

  isVisible: (id) => {
    const v = get().columnVisibility[id];
    if (v !== undefined) return v;
    return COLUMN_DEFS.find((c) => c.id === id)?.defaultVisible ?? true;
  },

  getVisibleColumns: () => {
    const { isVisible, columnOrder } = get();
    // Fixed columns first, then configurable columns in user's order
    const fixed = COLUMN_DEFS.filter((c) => !c.configurable && isVisible(c.id));
    const ordered = columnOrder
      .map((id) => COLUMN_DEFS.find((c) => c.id === id)!)
      .filter((c) => c && isVisible(c.id));
    return [...fixed, ...ordered];
  },

  setColumnWidth: (id, width) => {
    const def = COLUMN_DEFS.find((c) => c.id === id);
    const minWidth = def?.minWidth ?? 30;
    const clampedWidth = Math.max(minWidth, width);

    set((state) => {
      const newWidths = { ...state.columnWidths, [id]: clampedWidth };
      localStorage.setItem(STORAGE_KEY_WIDTHS, JSON.stringify(newWidths));
      return { columnWidths: newWidths };
    });
  },

  toggleColumnVisibility: (id) => {
    const def = COLUMN_DEFS.find((c) => c.id === id);
    if (!def?.configurable) return;

    set((state) => {
      const current = state.columnVisibility[id] ?? def.defaultVisible;
      const newVisible = { ...state.columnVisibility, [id]: !current };
      localStorage.setItem(STORAGE_KEY_VISIBLE, JSON.stringify(newVisible));
      return { columnVisibility: newVisible };
    });
  },

  moveColumn: (id, direction) => {
    set((state) => {
      const order = [...state.columnOrder];
      const idx = order.indexOf(id);
      if (idx === -1) return state;
      const targetIdx = direction === "up" ? idx - 1 : idx + 1;
      if (targetIdx < 0 || targetIdx >= order.length) return state;
      [order[idx], order[targetIdx]] = [order[targetIdx], order[idx]];
      localStorage.setItem(STORAGE_KEY_ORDER, JSON.stringify(order));
      return { columnOrder: order };
    });
  },

  resetColumns: () => {
    localStorage.removeItem(STORAGE_KEY_WIDTHS);
    localStorage.removeItem(STORAGE_KEY_VISIBLE);
    localStorage.removeItem(STORAGE_KEY_ORDER);
    set({
      columnWidths: {},
      columnVisibility: {},
      columnOrder: [...DEFAULT_CONFIGURABLE_ORDER],
    });
  },
}));

// Keyboard shortcuts shown in the "?" overlay. Keep in step with the handlers
// in hooks/useKeyboardShortcuts.ts and the Toolbar's keyboard actions.

export interface ShortcutEntry {
  keys: string;
  action: string;
}

// Key labels quoted in tooltips and menus, so they read the same as the overlay.
export const KEY_HINT = {
  run: "Space",
  remove: "Delete",
  settings: "Ctrl/⌘+,",
  logs: "Ctrl/⌘+L",
} as const;

export const SHORTCUTS: readonly ShortcutEntry[] = [
  { keys: "Ctrl/⌘ + A", action: "Select all visible torrents" },
  { keys: "Ctrl/⌘ + F", action: "Focus the search box" },
  { keys: "Ctrl/⌘ + O", action: "Choose a .torrent file to add" },
  { keys: "M", action: "Focus the magnet / URL input" },
  { keys: "Space", action: "Pause or resume the selected torrents" },
  { keys: "Enter", action: "Open details for the selected torrent" },
  { keys: "Delete", action: "Delete the selected torrents (asks first)" },
  { keys: "Shift + F10 or Menu", action: "Open the menu for the focused row" },
  { keys: "Esc", action: "Clear the selection" },
  { keys: "B", action: "Show or hide the sidebar" },
  { keys: "Ctrl/⌘ + ,", action: "Open settings" },
  { keys: "Ctrl/⌘ + L", action: "Open logs" },
  { keys: "?", action: "Show or hide this list" },
];

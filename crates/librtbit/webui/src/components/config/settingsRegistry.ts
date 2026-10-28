// Single list of individual settings. The settings search, the restart
// markers and the field labels in each tab are all read from it, so a
// setting's label is written once.

export type SettingsTabId =
  | "general"
  | "folders"
  | "downloads"
  | "speed"
  | "connections"
  | "behaviour"
  | "dht"
  | "security"
  | "http-api"
  | "rss";

export interface SettingDefinition {
  id: string;
  tab: SettingsTabId;
  label: string;
  // Read-only here: the engine reads it once at startup, so a change needs a restart.
  restartRequired?: boolean;
  // Shown beside the restart marker, saying which CLI flag sets it.
  description?: string;
}

// restartRequired entries were checked against the engine: the listen port and
// listen IP are bound when the session is created, DHT and LSD are created with
// the session (--disable-dht, --disable-lsd), UPnP port forwarding is started
// once (--disable-upnp-port-forward), peer exchange has no switch, encryption
// mode is not implemented, the HTTP API listener is bound at startup, and the
// persistence location is read only in main() at startup. There is no runtime
// setter for any of them. The HTTP bind address is display-only: changing it
// here is refused so the web UI cannot be exposed without authentication. The download folder is different: api_set_folders
// applies it live, so it is not marked.
export const SETTINGS: readonly SettingDefinition[] = [
  { id: "download-folder", tab: "folders", label: "Default download folder" },
  { id: "completed-folder", tab: "folders", label: "Completed folder" },
  {
    id: "data-directory",
    tab: "folders",
    label: "Data directory",
    restartRequired: true,
    description:
      "Where session state is persisted. Set with --persistence-location.",
  },

  {
    id: "incomplete-file-suffix",
    tab: "downloads",
    label: "Append .part to incomplete files",
  },

  { id: "download-rate-limit", tab: "speed", label: "Download rate limit" },
  { id: "upload-rate-limit", tab: "speed", label: "Upload rate limit" },
  { id: "peer-limit", tab: "speed", label: "Peer limit" },
  {
    id: "concurrent-init-limit",
    tab: "speed",
    label: "Concurrent init limit",
  },
  {
    id: "alt-speed-enabled",
    tab: "speed",
    label: "Enable alternative speed limits",
  },
  {
    id: "alt-download-limit",
    tab: "speed",
    label: "Alt download limit (KB/s)",
  },
  { id: "alt-upload-limit", tab: "speed", label: "Alt upload limit (KB/s)" },
  { id: "speed-schedule", tab: "speed", label: "Enable schedule" },

  {
    id: "listen-port",
    tab: "connections",
    label: "Listen port",
    restartRequired: true,
    description: "Set with --listen-port.",
  },
  {
    id: "listen-ip",
    tab: "connections",
    label: "Bind address (peer listener)",
    restartRequired: true,
    description: "Set with --listen-ip.",
  },
  {
    id: "upnp-port-forward",
    tab: "connections",
    label: "UPnP port forward",
    restartRequired: true,
    description: "On unless --disable-upnp-port-forward is set.",
  },
  {
    id: "pex-enabled",
    tab: "connections",
    label: "Peer exchange (PEX)",
    restartRequired: true,
    description: "Always on. There is no flag to turn it off.",
  },
  {
    id: "encryption-mode",
    tab: "connections",
    label: "Encryption mode",
    restartRequired: true,
    description: "Informational. This engine has no encryption-mode switch.",
  },

  { id: "max-active-downloads", tab: "behaviour", label: "Max active downloads" },
  { id: "max-active-uploads", tab: "behaviour", label: "Max active uploads" },
  { id: "max-active-total", tab: "behaviour", label: "Max active total" },

  {
    id: "seed-ratio-enabled",
    tab: "behaviour",
    label: "Limit seed ratio globally",
  },
  { id: "seed-ratio-limit", tab: "behaviour", label: "Ratio limit" },
  {
    id: "seed-time-enabled",
    tab: "behaviour",
    label: "Limit seeding time globally",
  },
  { id: "seed-time-limit", tab: "behaviour", label: "Time limit (hours)" },
  {
    id: "add-start-paused",
    tab: "behaviour",
    label: "Start torrents paused",
  },
  {
    id: "double-click-action",
    tab: "behaviour",
    label: "Double-click action",
  },

  {
    id: "dht-enabled",
    tab: "dht",
    label: "DHT enabled",
    restartRequired: true,
    description: "Turned off with --disable-dht.",
  },
  {
    id: "lsd-enabled",
    tab: "dht",
    label: "Local service discovery (LSD)",
    restartRequired: true,
    description: "Turned off with --disable-lsd.",
  },

  { id: "current-password", tab: "security", label: "Current Password" },
  { id: "new-username", tab: "security", label: "New Username" },
  { id: "new-password", tab: "security", label: "New Password" },
  { id: "confirm-password", tab: "security", label: "Confirm New Password" },

  {
    id: "http-api-bind-address",
    tab: "http-api",
    label: "HTTP API bind address",
    restartRequired: true,
    description: "Set with --http-api-listen-addr.",
  },

  { id: "feed-history-limit", tab: "rss", label: "Feed History Limit" },
];

/** The registry label for a setting. Throws on an unknown id, so a typo fails loudly. */
export const settingLabel = (id: string): string => {
  const setting = SETTINGS.find((s) => s.id === id);
  if (!setting) throw new Error(`Unknown setting id: ${id}`);
  return setting.label;
};

export const restartRequiredSettings = (
  tab?: SettingsTabId,
): SettingDefinition[] =>
  SETTINGS.filter((s) => s.restartRequired && (!tab || s.tab === tab));

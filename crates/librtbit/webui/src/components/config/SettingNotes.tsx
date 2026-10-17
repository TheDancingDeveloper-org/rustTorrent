import React from "react";
import {
  restartRequiredSettings,
  type SettingsTabId,
} from "./settingsRegistry";

export const RestartRequiredBadge: React.FC = () => (
  <span className="text-xs px-1.5 py-0.5 rounded bg-warning-bg text-warning whitespace-nowrap">
    Restart required
  </span>
);

// Read-only notes for the settings on a tab that only take effect at startup.
// Rendered from the registry, so the marker and the tab cannot drift apart.
export const RestartNotes: React.FC<{ tab: SettingsTabId }> = ({ tab }) => {
  const settings = restartRequiredSettings(tab);
  if (settings.length === 0) return null;
  return (
    <ul className="space-y-2 text-sm">
      {settings.map((setting) => (
        <li key={setting.id} className="flex flex-wrap items-center gap-2">
          <span className="font-medium text-primary">{setting.label}</span>
          <RestartRequiredBadge />
          {setting.description && (
            <span className="text-secondary text-xs">
              {setting.description}
            </span>
          )}
        </li>
      ))}
    </ul>
  );
};

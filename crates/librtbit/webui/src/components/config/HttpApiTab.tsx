import React from "react";
import { RestartNotes } from "./SettingNotes";

const InfoRow = ({ label, value }: { label: string; value: string }) => (
  <div className="flex items-center">
    <span className="w-40 text-secondary text-sm">{label}</span>
    <span className="text-primary text-sm font-medium">{value}</span>
  </div>
);

export const HttpApiTab: React.FC = () => {
  return (
    <div className="space-y-3 py-2">
      <InfoRow label="API URL" value={window.location.origin} />
      <InfoRow label="Swagger UI" value="/swagger/" />
      <RestartNotes tab="http-api" />
      <div className="mt-4 text-secondary text-xs">
        HTTP API settings (listen address, read-only mode, basic auth) are
        configured via CLI arguments at startup.
      </div>
    </div>
  );
};

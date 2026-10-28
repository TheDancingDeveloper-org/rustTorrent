import React, { useContext, useEffect, useState } from "react";
import { APIContext } from "../../context";
import { NetworkConfig } from "../../api-types";
import { RestartNotes } from "./SettingNotes";

const InfoRow = ({ label, value }: { label: string; value: string }) => (
  <div className="flex items-center">
    <span className="w-40 text-secondary text-sm">{label}</span>
    <span className="text-primary text-sm font-medium">{value}</span>
  </div>
);

export const HttpApiTab: React.FC = () => {
  const API = useContext(APIContext);
  const [network, setNetwork] = useState<NetworkConfig | null>(null);

  useEffect(() => {
    API.getNetworkConfig()
      .then(setNetwork)
      .catch(() => setNetwork(null));
  }, [API]);

  return (
    <div className="space-y-3 py-2">
      <InfoRow label="API URL" value={window.location.origin} />
      <InfoRow label="Swagger UI" value="/swagger/" />
      <InfoRow
        label="Reported bind"
        value={network?.http_api_listen || "unavailable"}
      />
      <RestartNotes tab="http-api" />
      <div className="mt-4 text-secondary text-xs">
        The HTTP API bind address is fixed at startup
        (--http-api-listen-addr) and is shown here only. It cannot be changed
        from the web UI, so a bind-address change cannot turn authentication
        off or expose this page.
      </div>
    </div>
  );
};

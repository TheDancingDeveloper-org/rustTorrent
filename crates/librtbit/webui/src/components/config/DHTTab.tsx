import React, { useCallback, useContext, useEffect, useState } from "react";
import { APIContext } from "../../context";
import { DhtStats, NetworkConfig } from "../../api-types";
import { Spinner } from "../Spinner";
import { RestartNotes } from "./SettingNotes";
import { NetworkSettingsForm } from "./NetworkSettingsForm";

const InfoRow = ({ label, value }: { label: string; value: string }) => (
  <div className="flex items-center">
    <span className="w-40 text-secondary text-sm">{label}</span>
    <span className="text-text text-sm font-medium tabular-nums">{value}</span>
  </div>
);

const formatLabel = (key: string): string => {
  return key.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
};

export const DHTTab: React.FC = () => {
  const API = useContext(APIContext);
  const [dhtStats, setDhtStats] = useState<DhtStats | null>(null);
  const [network, setNetwork] = useState<NetworkConfig | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadNetwork = useCallback(() => {
    API.getNetworkConfig()
      .then(setNetwork)
      .catch(() => setNetwork(null));
  }, [API]);

  useEffect(() => {
    loadNetwork();
    API.getDhtStats()
      .then(setDhtStats)
      .catch(() => setError("DHT is disabled or unavailable"));
  }, [API, loadNetwork]);

  const form = network && (
    <NetworkSettingsForm
      network={network}
      fields={["dht", "lsd"]}
      onSaved={loadNetwork}
    />
  );

  if (error && !network) {
    return <div className="text-secondary py-2">{error}</div>;
  }
  if (error) {
    return (
      <div className="space-y-3 py-2">
        <div className="text-secondary">{error}</div>
        <InfoRow
          label="DHT"
          value={network?.dht_enabled ? "On" : "Off"}
        />
        <InfoRow
          label="Local discovery"
          value={network?.lsd_enabled ? "On" : "Off"}
        />
        {form}
        <RestartNotes tab="dht" />
      </div>
    );
  }

  if (!dhtStats) {
    return (
      <div className="flex justify-center py-4">
        <Spinner />
      </div>
    );
  }

  return (
    <div className="space-y-3 py-2">
      {Object.entries(dhtStats).map(([key, value]) => {
        if (value === null || value === undefined) return null;
        // Skip complex nested objects, only show simple values
        if (typeof value === "object") return null;
        return (
          <InfoRow key={key} label={formatLabel(key)} value={String(value)} />
        );
      })}
      <InfoRow
        label="DHT"
        value={network ? (network.dht_enabled ? "On" : "Off") : "unavailable"}
      />
      <InfoRow
        label="Local discovery"
        value={network ? (network.lsd_enabled ? "On" : "Off") : "unavailable"}
      />
      {form}
      <RestartNotes tab="dht" />
      <div className="mt-4 text-secondary text-xs">
        DHT and local service discovery start with the session. A saved value
        applies at the next start and replaces --disable-dht and --disable-lsd.
      </div>
    </div>
  );
};

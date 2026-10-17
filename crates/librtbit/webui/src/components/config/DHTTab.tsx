import React, { useContext, useEffect, useState } from "react";
import { APIContext } from "../../context";
import { DhtStats } from "../../api-types";
import { Spinner } from "../Spinner";
import { RestartNotes } from "./SettingNotes";

const InfoRow = ({ label, value }: { label: string; value: string }) => (
  <div className="flex items-center">
    <span className="w-40 text-secondary text-sm">{label}</span>
    <span className="text-primary text-sm font-medium">{value}</span>
  </div>
);

const formatLabel = (key: string): string => {
  return key.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
};

export const DHTTab: React.FC = () => {
  const API = useContext(APIContext);
  const [dhtStats, setDhtStats] = useState<DhtStats | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    API.getDhtStats()
      .then(setDhtStats)
      .catch(() => setError("DHT is disabled or unavailable"));
  }, [API]);

  if (error) {
    return <div className="text-secondary py-2">{error}</div>;
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
      <RestartNotes tab="dht" />
      <div className="mt-4 text-secondary text-xs">
        DHT settings are configured via CLI arguments at startup.
      </div>
    </div>
  );
};

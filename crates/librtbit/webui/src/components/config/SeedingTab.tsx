import React, { useContext, useEffect, useState } from "react";
import { Fieldset } from "../forms/Fieldset";
import { FormInput } from "../forms/FormInput";
import { FormCheckbox } from "../forms/FormCheckbox";
import { APIContext } from "../../context";
import { SeedLimitsConfig } from "../../api-types";
import { Spinner } from "../Spinner";

export const SeedingTab: React.FC = () => {
  const API = useContext(APIContext);
  const [limits, setLimits] = useState<SeedLimitsConfig>({
    ratio_limit: null,
    time_limit_secs: null,
  });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    API.getSeedLimits()
      .then((config) => {
        setLimits(config);
      })
      .catch(() => {
        setError("Could not load seed limits");
      })
      .finally(() => setLoading(false));
  }, [API]);

  const saveLimits = async (newLimits: SeedLimitsConfig) => {
    setLimits(newLimits);
    try {
      await API.setSeedLimits(newLimits);
    } catch {
      // silent
    }
  };

  if (loading) {
    return (
      <div className="flex justify-center py-4">
        <Spinner />
      </div>
    );
  }

  if (error) {
    return <div className="text-secondary py-2">{error}</div>;
  }

  const ratioEnabled = limits.ratio_limit != null;
  const timeEnabled = limits.time_limit_secs != null;
  const timeLimitHours =
    limits.time_limit_secs != null
      ? Math.round((limits.time_limit_secs / 3600) * 100) / 100
      : "";

  return (
    <div className="py-2">
      <Fieldset label="Seed Ratio Limit">
        <FormCheckbox
          checked={ratioEnabled}
          setting="seed-ratio-enabled"
          name="ratio_limit_enabled"
          help="Stop seeding when the upload/download ratio reaches this value."
          onChange={() => {
            saveLimits({
              ...limits,
              ratio_limit: ratioEnabled ? null : 2.0,
            });
          }}
        />
        {ratioEnabled && (
          <div className="mt-2 ml-6">
            <FormInput
              setting="seed-ratio-limit"
              name="ratio_limit"
              inputType="number"
              value={
                limits.ratio_limit != null ? String(limits.ratio_limit) : ""
              }
              onChange={(e) => {
                const val = e.target.valueAsNumber;
                saveLimits({
                  ...limits,
                  ratio_limit: isNaN(val) || val <= 0 ? null : val,
                });
              }}
              help="e.g. 2.0 means upload 2x the downloaded amount"
            />
          </div>
        )}
      </Fieldset>

      <Fieldset label="Seed Time Limit">
        <FormCheckbox
          checked={timeEnabled}
          setting="seed-time-enabled"
          name="time_limit_enabled"
          help="Stop seeding after this amount of time."
          onChange={() => {
            saveLimits({
              ...limits,
              time_limit_secs: timeEnabled ? null : 24 * 3600,
            });
          }}
        />
        {timeEnabled && (
          <div className="mt-2 ml-6">
            <FormInput
              setting="seed-time-limit"
              name="time_limit_hours"
              inputType="number"
              value={String(timeLimitHours)}
              onChange={(e) => {
                const val = e.target.valueAsNumber;
                saveLimits({
                  ...limits,
                  time_limit_secs:
                    isNaN(val) || val <= 0 ? null : Math.round(val * 3600),
                });
              }}
              help="Seeding time limit in hours"
            />
          </div>
        )}
      </Fieldset>

      <div className="text-secondary text-xs mt-4">
        Super-seeding can be enabled per-torrent from the torrent context menu.
      </div>
    </div>
  );
};

import React, { useContext, useEffect, useState } from "react";
import { Fieldset } from "../forms/Fieldset";
import { FormCheckbox } from "../forms/FormCheckbox";
import { APIContext } from "../../context";
import { DownloadsConfig } from "../../api-types";
import { Spinner } from "../Spinner";

// Saved as soon as a box is toggled, like the Seeding tab.
export const DownloadsTab: React.FC = () => {
  const API = useContext(APIContext);
  const [config, setConfig] = useState<DownloadsConfig | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    API.getDownloadsConfig()
      .then(setConfig)
      .catch(() => setError("Could not load download settings"));
  }, [API]);

  const save = async (next: DownloadsConfig) => {
    const previous = config;
    setConfig(next);
    setError(null);
    try {
      await API.setDownloadsConfig(next);
    } catch {
      setConfig(previous);
      setError("Could not save download settings");
    }
  };

  if (!config) {
    return error ? (
      <div className="text-secondary py-2">{error}</div>
    ) : (
      <div className="flex justify-center py-4">
        <Spinner />
      </div>
    );
  }

  return (
    <div className="py-2">
      <Fieldset label="Incomplete files">
        <FormCheckbox
          checked={config.incomplete_file_suffix}
          setting="incomplete-file-suffix"
          name="incomplete_file_suffix"
          help="Files are written as <name>.part until they finish. Applies to torrents added after the change."
          onChange={() =>
            save({
              ...config,
              incomplete_file_suffix: !config.incomplete_file_suffix,
            })
          }
        />
      </Fieldset>
      {error && (
        <div className="text-sm text-error" role="alert">
          {error}
        </div>
      )}
      <div className="text-secondary text-xs mt-4">
        Download and move-on-completion folders are on the Folders tab; the
        add-torrent "Start torrents paused" default is on the Behaviour tab.
        Files are created sparse; full pre-allocation is not supported.
      </div>
    </div>
  );
};

import React, { useContext, useEffect, useState } from "react";
import { APIContext } from "../../context";
import { Fieldset } from "../forms/Fieldset";
import { FormInput } from "../forms/FormInput";

// Same caps the server enforces.
const CAPS = {
  connect: 300,
  readWrite: 600,
  keepAlive: 600,
};

const parseSecs = (raw: string, cap: number): number | null => {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  if (!Number.isInteger(value) || value < 1 || value > cap) return null;
  return value;
};

const SecsField: React.FC<{
  setting: string;
  name: string;
  value: string;
  cap: number;
  help: string;
  onChange: (value: string) => void;
}> = ({ setting, name, value, cap, help, onChange }) => {
  const invalid = value.trim() !== "" && parseSecs(value, cap) === null;
  return (
    <div>
      <FormInput
        setting={setting}
        name={name}
        inputType="number"
        value={value}
        placeholder="engine default"
        help={help}
        onChange={(e) => onChange(e.target.value)}
      />
      {invalid && (
        <div className="text-sm text-red-500 -mt-1 mb-2">
          Enter a whole number from 1 to {cap}, or leave empty for the engine
          default.
        </div>
      )}
    </div>
  );
};

export const AdvancedTab: React.FC = () => {
  const API = useContext(APIContext);
  const [connect, setConnect] = useState("");
  const [readWrite, setReadWrite] = useState("");
  const [keepAlive, setKeepAlive] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    API.getAdvanced()
      .then((config) => {
        setConnect(config.connect_timeout_secs?.toString() ?? "");
        setReadWrite(config.read_write_timeout_secs?.toString() ?? "");
        setKeepAlive(config.keep_alive_secs?.toString() ?? "");
      })
      .catch(() => setError("Could not load the advanced settings."))
      .finally(() => setLoaded(true));
  }, [API]);

  const values = {
    connect_timeout_secs: parseSecs(connect, CAPS.connect),
    read_write_timeout_secs: parseSecs(readWrite, CAPS.readWrite),
    keep_alive_secs: parseSecs(keepAlive, CAPS.keepAlive),
  };
  const valid =
    (connect.trim() === "" || values.connect_timeout_secs !== null) &&
    (readWrite.trim() === "" || values.read_write_timeout_secs !== null) &&
    (keepAlive.trim() === "" || values.keep_alive_secs !== null);

  const save = () => {
    if (!valid) return;
    setSaving(true);
    setSaved(false);
    setError(null);
    API.setAdvanced(values)
      .then(() => setSaved(true))
      .catch(() => setError("The server refused these values."))
      .finally(() => setSaving(false));
  };

  if (!loaded) return <div className="py-2 text-secondary text-sm">Loading…</div>;

  return (
    <div className="py-2">
      <Fieldset label="Peer connection timeouts">
        <p className="text-sm text-secondary mb-2">
          Applied to torrents added after saving. Empty uses the engine
          default. The server rejects anything outside the caps.
        </p>
        <SecsField
          setting="connect-timeout"
          name="connect_timeout_secs"
          value={connect}
          cap={CAPS.connect}
          help={`Seconds to wait while connecting to a peer. Cap ${CAPS.connect}.`}
          onChange={setConnect}
        />
        <SecsField
          setting="read-write-timeout"
          name="read_write_timeout_secs"
          value={readWrite}
          cap={CAPS.readWrite}
          help={`Seconds a peer may go quiet before the connection is dropped. Cap ${CAPS.readWrite}.`}
          onChange={setReadWrite}
        />
        <SecsField
          setting="keep-alive-interval"
          name="keep_alive_secs"
          value={keepAlive}
          cap={CAPS.keepAlive}
          help={`Seconds between keep-alive messages. Cap ${CAPS.keepAlive}.`}
          onChange={setKeepAlive}
        />
        <button
          type="button"
          onClick={save}
          disabled={!valid || saving}
          className="px-3 py-1.5 text-sm rounded bg-primary text-white disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save advanced settings"}
        </button>
        {saved && (
          <span className="ml-2 text-sm text-secondary">Saved.</span>
        )}
        {error && <div className="text-sm text-red-500 mt-2">{error}</div>}
      </Fieldset>
    </div>
  );
};

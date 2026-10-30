import React, { useContext, useEffect, useState } from "react";
import { APIContext } from "../../context";
import { NetworkConfig, NetworkSettings } from "../../api-types";
import { FormInput } from "../forms/FormInput";
import { FormCheckbox } from "../forms/FormCheckbox";
import { RestartRequiredBadge } from "./SettingNotes";

type Field = "listen" | "upnp" | "dht" | "lsd";

// Editable startup-only network settings. The server saves them and applies
// them at the next start, so every save says a restart is required. The form
// starts from the saved values, or from what the process is running with.
export const NetworkSettingsForm: React.FC<{
  network: NetworkConfig;
  fields: readonly Field[];
  onSaved: () => void;
}> = ({ network, fields, onSaved }) => {
  const API = useContext(APIContext);
  const pending = network.pending ?? {};
  const [ip, setIp] = useState("");
  const [port, setPort] = useState("");
  const [upnp, setUpnp] = useState(false);
  const [dht, setDht] = useState(false);
  const [lsd, setLsd] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setIp(pending.listen_ip ?? network.listen_ip ?? "");
    setPort(String(pending.listen_port ?? network.listen_port ?? ""));
    setUpnp(pending.upnp_port_forward ?? network.upnp_port_forward);
    setDht(pending.dht_enabled ?? network.dht_enabled);
    setLsd(pending.lsd_enabled ?? network.lsd_enabled);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [network]);

  const has = (field: Field) => fields.includes(field);
  const portNumber = Number(port);
  const portValid =
    !has("listen") ||
    (Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535);

  const save = () => {
    const settings: NetworkSettings = {};
    if (has("listen")) {
      if (ip.trim() !== "") settings.listen_ip = ip.trim();
      settings.listen_port = portNumber;
    }
    if (has("upnp")) settings.upnp_port_forward = upnp;
    if (has("dht")) settings.dht_enabled = dht;
    if (has("lsd")) settings.lsd_enabled = lsd;
    setSaving(true);
    setSaved(false);
    setError(null);
    API.setNetworkConfig(settings)
      .then(() => {
        setSaved(true);
        onSaved();
      })
      .catch((err: unknown) =>
        setError(
          err instanceof Error ? err.message : "The server refused these values.",
        ),
      )
      .finally(() => setSaving(false));
  };

  return (
    <div className="space-y-2">
      {has("listen") && (
        <>
          <FormInput
            setting="listen-ip"
            name="listen_ip"
            inputType="text"
            value={ip}
            placeholder="::"
            help="IPv4 or IPv6 address for incoming peers. :: listens on every interface."
            onChange={(e) => setIp(e.target.value)}
          />
          <FormInput
            setting="listen-port"
            name="listen_port"
            inputType="number"
            value={port}
            help="1 to 65535."
            onChange={(e) => setPort(e.target.value)}
          />
        </>
      )}
      {has("upnp") && (
        <FormCheckbox
          setting="upnp-port-forward"
          name="upnp_port_forward"
          checked={upnp}
          onChange={(e) => setUpnp(e.target.checked)}
        />
      )}
      {has("dht") && (
        <FormCheckbox
          setting="dht-enabled"
          name="dht_enabled"
          checked={dht}
          onChange={(e) => setDht(e.target.checked)}
        />
      )}
      {has("lsd") && (
        <FormCheckbox
          setting="lsd-enabled"
          name="lsd_enabled"
          checked={lsd}
          onChange={(e) => setLsd(e.target.checked)}
        />
      )}
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={save}
          disabled={saving || !portValid}
          className="px-3 py-1.5 text-sm rounded bg-primary text-white disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save for next start"}
        </button>
        <RestartRequiredBadge />
        {saved && (
          <span className="text-sm text-secondary">
            Saved. Restart to apply.
          </span>
        )}
      </div>
      {!portValid && (
        <div className="text-sm text-error">Enter a port from 1 to 65535.</div>
      )}
      {error && <div className="text-sm text-error">{error}</div>}
    </div>
  );
};

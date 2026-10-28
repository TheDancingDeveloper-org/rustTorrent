import React, { useContext, useEffect, useState } from "react";
import { APIContext } from "../../context";
import { Fieldset } from "../forms/Fieldset";
import { FormInput } from "../forms/FormInput";

export const ProxyFilterTab: React.FC = () => {
  const API = useContext(APIContext);
  const [proxy, setProxy] = useState("");
  const [filter, setFilter] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    Promise.all([API.getProxy(), API.getIpFilter()])
      .then(([proxyConfig, filterConfig]) => {
        setProxy(proxyConfig.proxy_url ?? "");
        setFilter(filterConfig.filter);
      })
      .catch(() => setError("Could not load connection settings."))
      .finally(() => setLoaded(true));
  }, [API]);

  const save = () => {
    setSaving(true);
    setSaved(false);
    setError(null);
    const trimmed = proxy.trim();
    // A masked URL came back from the server. Sending it would store the mask
    // as a password, so leave the proxy unchanged unless the user typed a new one.
    const proxyWrite = trimmed.includes("***")
      ? Promise.resolve()
      : API.setProxy({ proxy_url: trimmed === "" ? null : trimmed });
    Promise.all([proxyWrite, API.setIpFilter({ filter })])
      .then(() => API.getProxy())
      .then((next) => {
        setProxy(next.proxy_url ?? "");
        setSaved(true);
      })
      .catch(async (err: unknown) => {
        const message =
          err instanceof Error ? err.message : "The server refused these values.";
        setError(message);
      })
      .finally(() => setSaving(false));
  };

  if (!loaded) return <div className="py-2 text-secondary text-sm">Loading…</div>;

  return (
    <div className="py-2">
      <Fieldset label="Outgoing proxy">
        <p className="text-sm text-secondary mb-2">
          SOCKS5 or HTTP proxy. A password is stored with the session and
          shown here masked. Leave empty to connect directly. Peer connections
          use a new proxy immediately. Trackers, DHT and local peer discovery
          keep the previous address until you restart, so a proxy change is
          marked restart-required.
        </p>
        <FormInput
          setting="proxy-url"
          name="proxy_url"
          inputType="text"
          value={proxy}
          placeholder="socks5://host:1080"
          help="socks5:// or http:// URL. Do not reuse a masked value; type the URL again to change it."
          onChange={(e) => setProxy(e.target.value)}
        />
      </Fieldset>
      <Fieldset label="IP filter">
        <p className="text-sm text-secondary mb-2">
          One range per line, as name:start-end. Blank lines and lines starting
          with # are ignored. The server refuses a list over 1 MiB and names
          the first invalid line.
        </p>
        <textarea
          aria-label="IP filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          rows={8}
          className="w-full font-mono text-sm p-2 rounded border border-secondary bg-transparent"
          placeholder={"lan:10.0.0.0-10.0.0.255"}
        />
        <button
          type="button"
          onClick={save}
          disabled={saving}
          className="mt-2 px-3 py-1.5 text-sm rounded bg-primary text-white disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save connection settings"}
        </button>
        {saved && <span className="ml-2 text-sm text-secondary">Saved.</span>}
        {error && <div className="text-sm text-error mt-2">{error}</div>}
      </Fieldset>
    </div>
  );
};

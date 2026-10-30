import React, { useCallback, useContext, useEffect, useState } from "react";
import { useStatsStore } from "../../stores/statsStore";
import { APIContext } from "../../context";
import { RestartNotes } from "./SettingNotes";
import { NetworkSettingsForm } from "./NetworkSettingsForm";
import { Fieldset } from "../forms/Fieldset";
import {
  ConnectionStatSingle,
  ConnectionStatsPerFamily,
  NetworkConfig,
} from "../../api-types";

const headerCell = "pb-1 px-2";
const numericCell = "px-2 text-right text-text tabular-nums";

const ConnectionRow = ({
  protocol,
  family,
  stat,
}: {
  protocol: string;
  family: string;
  stat: ConnectionStatSingle;
}) => {
  // Skip rows where there's no activity at all
  if (stat.attempts === 0 && stat.successes === 0 && stat.errors === 0) {
    return null;
  }

  return (
    <tr className="border-t border-divider">
      <td className="px-2 py-1 text-text font-medium">{protocol}</td>
      <td className="px-2 py-1 text-text">{family}</td>
      <td className={numericCell}>{stat.attempts.toLocaleString()}</td>
      <td className={numericCell}>{stat.successes.toLocaleString()}</td>
      <td className={numericCell}>{stat.errors.toLocaleString()}</td>
    </tr>
  );
};

const ProtocolRows = ({
  protocol,
  stats,
}: {
  protocol: string;
  stats: ConnectionStatsPerFamily;
}) => (
  <>
    <ConnectionRow protocol={protocol} family="IPv4" stat={stats.v4} />
    <ConnectionRow protocol={protocol} family="IPv6" stat={stats.v6} />
  </>
);

const yesNo = (value: boolean) => (value ? "On" : "Off");

export const ConnectionTab: React.FC = () => {
  const stats = useStatsStore((state) => state.stats);
  const conns = stats.connections;
  const API = useContext(APIContext);
  const [network, setNetwork] = useState<NetworkConfig | null>(null);

  const load = useCallback(() => {
    API.getNetworkConfig()
      .then(setNetwork)
      .catch(() => setNetwork(null));
  }, [API]);
  useEffect(load, [load]);

  return (
    <div className="py-2">
      <p className="text-secondary text-sm mb-3">
        Connection statistics from the current session.
      </p>
      <table className="w-full text-sm">
        <thead>
          <tr className="text-tertiary text-left">
            <th className={headerCell}>Protocol</th>
            <th className={headerCell}>Family</th>
            <th className={`${headerCell} text-right`}>Attempts</th>
            <th className={`${headerCell} text-right`}>Successes</th>
            <th className={`${headerCell} text-right`}>Errors</th>
          </tr>
        </thead>
        <tbody>
          <ProtocolRows protocol="TCP" stats={conns.tcp} />
          <ProtocolRows protocol="uTP" stats={conns.utp} />
          <ProtocolRows protocol="SOCKS" stats={conns.socks} />
        </tbody>
      </table>
      <div className="mt-6 space-y-2 text-sm">
        <div className="flex items-center gap-2">
          <span className="w-52 text-secondary">Listen address</span>
          <span className="text-text font-medium">
            {network
              ? `${network.listen_ip ?? "not listening"}:${network.listen_port ?? ""}`
              : "unavailable"}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-52 text-secondary">UPnP port forward</span>
          <span className="text-text font-medium">
            {network ? yesNo(network.upnp_port_forward) : "unavailable"}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-52 text-secondary">Peer exchange</span>
          <span className="text-text font-medium">
            {network ? (network.pex_enabled ? "On (not configurable)" : "Off") : "unavailable"}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-52 text-secondary">Encryption</span>
          <span className="text-text font-medium">
            {network?.encryption ?? "Not supported by this engine"}
          </span>
        </div>
      </div>

      {network && (
        <div className="mt-6">
          <Fieldset label="Peer listener">
            <p className="text-sm text-secondary mb-2">
              The values above are what is running now. Saved values apply at
              the next start and replace the --listen-ip, --listen-port and
              --disable-upnp-port-forward flags.
            </p>
            <NetworkSettingsForm
              network={network}
              fields={["listen", "upnp"]}
              onSaved={load}
            />
          </Fieldset>
        </div>
      )}
      <div className="mt-6">
        <RestartNotes tab="connections" />
      </div>
      <div className="mt-4 text-secondary text-xs">
        Peer exchange is always on and encryption mode is not supported, so
        those two are informational. The web UI bind address is not editable
        here.
      </div>
    </div>
  );
};

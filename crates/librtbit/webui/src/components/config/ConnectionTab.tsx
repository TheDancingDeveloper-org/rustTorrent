import React from "react";
import { useStatsStore } from "../../stores/statsStore";
import { RestartNotes } from "./SettingNotes";
import {
  ConnectionStatSingle,
  ConnectionStatsPerFamily,
} from "../../api-types";

const headerCell = "pb-1 px-2";
const numericCell = "px-2 text-right text-primary tabular-nums";

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
      <td className="px-2 py-1 text-primary">{protocol}</td>
      <td className="px-2 py-1 text-primary">{family}</td>
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

export const ConnectionTab: React.FC = () => {
  const stats = useStatsStore((state) => state.stats);
  const conns = stats.connections;

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
      <div className="mt-6 mb-3">
        <label className="font-medium mb-3 block pb-1 border-b border-divider">
          Encryption
        </label>
        <div className="mt-2">
          <select
            className="block border border-divider rounded bg-transparent py-1.5 px-2 focus:ring-0 focus:border-primary sm:leading-6 text-primary"
            disabled
            value="enabled"
          >
            <option value="disabled">Disabled</option>
            <option value="enabled">Enabled (prefer encrypted)</option>
            <option value="forced">Forced (require encrypted)</option>
          </select>
          <div className="text-sm text-secondary mt-2 space-y-1">
            <p>
              <strong>Disabled:</strong> No encryption, all connections are
              plaintext.
            </p>
            <p>
              <strong>Enabled:</strong> Prefer encrypted connections but allow
              plaintext fallback.
            </p>
            <p>
              <strong>Forced:</strong> Only allow encrypted connections; reject
              unencrypted peers.
            </p>
          </div>
        </div>
      </div>

      <div className="mt-6">
        <RestartNotes tab="connections" />
      </div>
      <div className="mt-4 text-secondary text-xs">
        Connection settings (listen port, TCP/uTP, UPnP, SOCKS proxy, encryption
        mode) are configured via CLI arguments at startup.
      </div>
    </div>
  );
};

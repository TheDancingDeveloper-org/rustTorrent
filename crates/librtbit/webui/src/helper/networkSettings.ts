// Pure helpers for the startup-only network settings form, kept here so the
// node tests can reach them without React.

// Shape of the parts of GET /session/network these helpers read.
export interface NetworkView {
  listen_ip: string | null;
  listen_port: number | null;
  dht_enabled: boolean;
  lsd_enabled: boolean;
  upnp_port_forward: boolean;
  pending?: {
    listen_ip?: string | null;
    listen_port?: number | null;
    dht_enabled?: boolean | null;
    lsd_enabled?: boolean | null;
    upnp_port_forward?: boolean | null;
  } | null;
}

// A blocking problem with the typed listen IP, or null. An empty value means
// "keep the startup flag".
export function listenIpError(ip: string): string | null {
  const value = ip.trim();
  if (value === "") return null;
  if (value.includes("%")) {
    return "IPv6 zones (%interface) are not supported. Leave the zone off, or pick the interface with --bind-device.";
  }
  const bare = value.replace(/^\[/, "").replace(/\]$/, "");
  const ipv4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(bare);
  if (ipv4) {
    return ipv4.slice(1).every((part) => Number(part) <= 255)
      ? null
      : "Enter a valid IPv4 or IPv6 address.";
  }
  if (bare.includes(":") && /^[0-9a-fA-F:.]+$/.test(bare)) return null;
  return "Enter a valid IPv4 or IPv6 address.";
}

// Non-blocking warnings for values that are legal but often a mistake.
export function networkWarnings(ip: string, port: number): string[] {
  const out: string[] = [];
  if (Number.isInteger(port) && port >= 1 && port < 1024) {
    out.push(
      "Ports below 1024 need root or CAP_NET_BIND_SERVICE. If the daemon cannot bind it, it falls back to the startup flags.",
    );
  }
  const bare = ip.trim().replace(/^\[/, "").replace(/\]$/, "");
  if (bare === "0.0.0.0" || bare === "::") {
    out.push(
      bare === "::"
        ? ":: listens for peers on every interface, IPv4 and IPv6."
        : "0.0.0.0 listens for peers on every IPv4 interface and none on IPv6.",
    );
  }
  return out;
}

export interface SavedRow {
  label: string;
  running: string;
  saved: string;
  differs: boolean;
}

const onOff = (v: boolean) => (v ? "On" : "Off");

// One row per saved value, next to what the process runs with now.
export function savedVsRunning(network: NetworkView): SavedRow[] {
  const p = network.pending;
  if (!p) return [];
  const rows: SavedRow[] = [];
  const add = (label: string, running: string, saved: string) =>
    rows.push({ label, running, saved, differs: running !== saved });
  if (p.listen_ip != null) add("Listen IP", network.listen_ip ?? "off", p.listen_ip);
  if (p.listen_port != null)
    add("Listen port", String(network.listen_port ?? "off"), String(p.listen_port));
  if (p.upnp_port_forward != null)
    add("UPnP port forward", onOff(network.upnp_port_forward), onOff(p.upnp_port_forward));
  if (p.dht_enabled != null)
    add("DHT", onOff(network.dht_enabled), onOff(p.dht_enabled));
  if (p.lsd_enabled != null)
    add("Local discovery", onOff(network.lsd_enabled), onOff(p.lsd_enabled));
  return rows;
}

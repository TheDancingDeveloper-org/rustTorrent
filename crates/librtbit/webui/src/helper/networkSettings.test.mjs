// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import {
  listenIpError,
  networkWarnings,
  savedVsRunning,
} from "./networkSettings.ts";

test("listen IP accepts v4, v6, brackets and empty", () => {
  for (const ip of ["", "127.0.0.1", "::", "::1", "[::1]", "fe80::1", "::ffff:10.0.0.1"]) {
    assert.equal(listenIpError(ip), null, ip);
  }
});

test("listen IP rejects zones and junk", () => {
  assert.match(listenIpError("fe80::1%eth0"), /zone/);
  assert.match(listenIpError("300.1.1.1"), /valid/);
  assert.match(listenIpError("example.com"), /valid/);
});

test("warns on privileged ports and wildcard binds", () => {
  assert.equal(networkWarnings("127.0.0.1", 6881).length, 0);
  assert.match(networkWarnings("", 80)[0], /below 1024/);
  assert.match(networkWarnings("0.0.0.0", 6881)[0], /IPv4/);
  assert.match(networkWarnings("::", 6881)[0], /every interface/);
  assert.equal(networkWarnings("::", 443).length, 2);
});

test("saved values are listed next to running values", () => {
  const base = {
    listen_ip: "::",
    listen_port: 6881,
    dht_enabled: true,
    lsd_enabled: true,
    upnp_port_forward: true,
  };
  assert.deepEqual(savedVsRunning({ ...base, pending: null }), []);
  const rows = savedVsRunning({
    ...base,
    pending: { listen_port: 7000, dht_enabled: true },
  });
  assert.deepEqual(rows, [
    { label: "Listen port", running: "6881", saved: "7000", differs: true },
    { label: "DHT", running: "On", saved: "On", differs: false },
  ]);
});

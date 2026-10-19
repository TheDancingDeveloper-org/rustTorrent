#!/usr/bin/env python3
"""Sonarr -> rustTorrent end-to-end smoke check.

Configures rustTorrent as a qBittorrent download client in a running Sonarr
(through Sonarr's own API, from its download-client schema), runs Sonarr's
"Test" action (login, version probe, preferences and categories, as Sonarr's
proxy issues them), saves the client, and checks Sonarr's health list carries
no download-client problem. Standard library only.

    SONARR_API_KEY=... tests/sonarr-e2e/smoke.py \\
        --sonarr http://sonarr:8989 --rtbit-host rtbit --rtbit-port 3030
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request


def call(base: str, key: str, method: str, path: str, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"{base}/api/v3{path}",
        data=data,
        method=method,
        headers={"X-Api-Key": key, "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            raw = r.read()
            return r.status, json.loads(raw) if raw else None
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw)
        except ValueError:
            return e.code, raw.decode(errors="replace")


def wait_ping(base: str, timeout: float) -> None:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            with urllib.request.urlopen(f"{base}/ping", timeout=5) as r:
                if r.status == 200:
                    return
        except OSError:
            pass
        time.sleep(2)
    sys.exit(f"Sonarr at {base} did not answer /ping within {timeout:.0f}s")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sonarr", default="http://sonarr:8989")
    ap.add_argument("--rtbit-host", default="rtbit")
    ap.add_argument("--rtbit-port", type=int, default=3030)
    ap.add_argument("--username", default="admin")
    ap.add_argument("--password", default="adminadmin")
    ap.add_argument("--category", default="tv-sonarr")
    ap.add_argument("--wait", type=float, default=180)
    args = ap.parse_args()
    key = os.environ.get("SONARR_API_KEY") or sys.exit("SONARR_API_KEY is not set")

    wait_ping(args.sonarr, args.wait)
    status, schemas = call(args.sonarr, key, "GET", "/downloadclient/schema")
    assert status == 200, f"downloadclient/schema: {status} {schemas}"
    schema = next(s for s in schemas if s["implementation"] == "QBittorrent")
    values = {
        "host": args.rtbit_host,
        "port": args.rtbit_port,
        "useSsl": False,
        "username": args.username,
        "password": args.password,
        "tvCategory": args.category,
    }
    for f in schema["fields"]:
        if f["name"] in values:
            f["value"] = values[f["name"]]
    client = {**schema, "name": "rustTorrent", "enable": True, "priority": 1}
    client.pop("id", None)

    status, body = call(args.sonarr, key, "POST", "/downloadclient/test", client)
    print(f"downloadclient/test -> {status} {body if body else ''}")
    if status != 200:
        return 1

    status, saved = call(args.sonarr, key, "POST", "/downloadclient", client)
    print(f"downloadclient create -> {status}")
    if status not in (200, 201):
        print(saved)
        return 1

    # Health is cached; ask for a fresh check, then require it to come back clean.
    call(args.sonarr, key, "POST", "/command", {"name": "CheckHealth"})
    problems = []
    for _ in range(20):
        time.sleep(3)
        status, health = call(args.sonarr, key, "GET", "/health")
        problems = [
            h for h in (health or [])
            if "DownloadClient" in h.get("source", "") and h.get("type") in ("error", "warning")
        ]
        if not problems:
            break
    if problems:
        print(f"health reports download-client problems: {problems}")
        return 1
    print("Sonarr accepted rustTorrent as a qBittorrent download client.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

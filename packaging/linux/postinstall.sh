#!/bin/sh
# Runs after deb/rpm install. Creates the rtbit system user from the
# sysusers.d entry, then picks up the new unit. Does not start the service.
set -e

if command -v systemd-sysusers >/dev/null 2>&1; then
    systemd-sysusers /usr/lib/sysusers.d/rtbit.conf
elif ! getent passwd rtbit >/dev/null; then
    useradd --system --home-dir /var/lib/rtbit --shell /usr/sbin/nologin rtbit
fi

mkdir -p /var/lib/rtbit
chown rtbit:rtbit /var/lib/rtbit

if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
fi

#!/usr/bin/env bash
# Build, lint and test the Tauri desktop app (desktop/src-tauri) and its menu
# description (desktop/menu).
#
# Used by the desktop job in ci.yml. The shared rust-heavy runners have no
# webkit or Node, and nothing here changes their dpkg or apt state: the webkit, GTK
# and appindicator development packages and a checksummed Node are installed inside the same
# digest-pinned container that build-desktop.sh uses.
set -euo pipefail

# rust:1.95-bookworm, pinned by index digest. Keep in step with build-desktop.sh.
image="docker.io/library/rust:1.95-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1"

command -v docker >/dev/null 2>&1 || { echo "::error::docker is required for the desktop build"; exit 1; }
# The runner may itself run in a container that talks to the host's docker
# daemon, so a bind mount of $PWD would name a path the daemon cannot see.
# Copy the tree in with docker cp instead.
cid="$(docker create -w /src \
  -e CARGO_TARGET_DIR=/src/target/desktop \
  -e CARGO_INCREMENTAL=0 \
  -e CARGO_BUILD_JOBS \
  "$image" bash -euo pipefail -c '
    apt-get update -qq
    apt-get install -y --no-install-recommends pkg-config libssl-dev \
      libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
      librsvg2-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev
    # swarmforge builds the web UI it embeds with npm (CI uses Node 24).
    node=node-v24.11.1-linux-x64
    curl -fsSL -o /tmp/$node.tar.xz https://nodejs.org/dist/v24.11.1/$node.tar.xz
    echo "60e3b0a8500819514aca603487c254298cd776de0698d3cd08f11dba5b8289a8  /tmp/$node.tar.xz" | sha256sum -c -
    tar -xJf /tmp/$node.tar.xz -C /opt
    export PATH="/opt/$node/bin:$PATH"
    rustup component add clippy
    pkgs=(-p rtbit-desktop -p rtbit-desktop-menu)
    cargo clippy --locked "${pkgs[@]}" --all-targets -- -D warnings
    cargo test --locked "${pkgs[@]}" --all-targets
  ')"
trap 'docker rm -f "$cid" >/dev/null 2>&1 || true' EXIT
tar -c --exclude=./target --exclude=./.git --exclude='*/node_modules' \
  --transform 's,^\.,src,' . | docker cp - "$cid:/"
docker start -a "$cid"

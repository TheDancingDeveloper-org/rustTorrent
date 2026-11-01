#!/usr/bin/env bash
# Build and stage the desktop client (crates/rtbit-fluent, gpui) release assets.
#
#   build-desktop.sh linux   <tag> <dir>   binary + .deb, built in a container
#   build-desktop.sh windows <tag> <dir>   stages the native Windows .exe; run on the self-hosted Windows runner
#   build-desktop.sh sbom    <tag> <dir>   CycloneDX SBOM of the client graph
#
# Artifacts are named rtbit-fluent-<tag>-*.
#
# Used by the build job in release.yml and by release-dry-run.yml. The
# rust-heavy runners are shared and not ephemeral, so nothing here changes
# their dpkg or apt state: the Linux build installs the wayland, X11 and GL
# development packages inside a digest-pinned container.
#
# Expects the CARGO_PROFILE_RELEASE_GITHUB_* environment from the caller. The
# Linux build also needs nfpm on PATH.
set -euo pipefail

mode="${1:?usage: build-desktop.sh <linux|windows|sbom> <tag> <dir>}"
tag="${2:?usage: build-desktop.sh <linux|windows|sbom> <tag> <dir>}"
dir="${3:?usage: build-desktop.sh <linux|windows|sbom> <tag> <dir>}"
tmp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
mkdir -p "$dir"

crate=rtbit-fluent
client=fluent

# rust:1.95-bookworm, pinned by index digest.
image="docker.io/library/rust:1.95-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1"

case "$mode" in
  linux)
    command -v docker >/dev/null 2>&1 || { echo "::error::docker is required for the desktop client Linux build"; exit 1; }
    # The runner may itself run in a container that talks to the host's
    # docker daemon, so a bind mount of $PWD would name a path the daemon
    # cannot see. Copy the tree in and the binary out with docker cp instead.
    cid="$(docker create -w /src \
      -e CARGO_TARGET_DIR=/src/target/desktop-linux \
      -e CARGO_PROFILE_RELEASE_GITHUB_LTO \
      -e CARGO_PROFILE_RELEASE_GITHUB_CODEGEN_UNITS \
      -e CARGO_PROFILE_RELEASE_GITHUB_STRIP \
      -e CLIENT_CRATE="$crate" \
      "$image" bash -euo pipefail -c '
        apt-get update -qq
        apt-get install -y --no-install-recommends pkg-config \
          libwayland-dev libxkbcommon-dev libx11-dev libxcursor-dev \
          libxrandr-dev libxi-dev libgl1-mesa-dev libegl1-mesa-dev \
          libxcb1-dev libxkbcommon-x11-dev libfontconfig1-dev libvulkan-dev
        cargo build --locked --profile release-github -p "$CLIENT_CRATE"
      ')"
    trap 'docker rm -f "$cid" >/dev/null 2>&1 || true' EXIT
    tar -c --exclude=./target --exclude=./.git --exclude='*/node_modules' \
      --transform 's,^\.,src,' . | docker cp - "$cid:/"
    docker start -a "$cid"
    mkdir -p target/desktop-linux/release-github
    docker cp "$cid:/src/target/desktop-linux/release-github/$crate" "target/desktop-linux/release-github/$crate"
    bin=target/desktop-linux/release-github/$crate
    test -s "$bin"
    VERSION="${tag#v}" ARCH=amd64 BINARY="$bin" \
      nfpm package --packager deb --config "packaging/linux/nfpm-$client.yaml" \
      --target "$dir/rtbit-$client-$tag-amd64.deb"
    cp "$bin" "$dir/rtbit-$client-$tag-linux-x86_64"
    ;;
  windows)
    # Native build only: gpui_windows compiles DirectX shaders with the
    # Windows SDK, so this cannot cross-compile from Linux. The caller has
    # already run `cargo build -p rtbit-fluent --release --target
    # x86_64-pc-windows-msvc`.
    bin="target/x86_64-pc-windows-msvc/release/$crate.exe"
    test -s "$bin"
    cp "$bin" "$dir/rtbit-$client-$tag-windows-x86_64.exe"
    ;;
  sbom)
    if ! command -v cargo-cyclonedx >/dev/null 2>&1; then
      cargo install --locked cargo-cyclonedx --version 0.5.7 --root "$tmp/cyclonedx"
      export PATH="$tmp/cyclonedx/bin:$PATH"
    fi
    cargo-cyclonedx cyclonedx \
      --manifest-path "crates/$crate/Cargo.toml" \
      --format json --spec-version 1.5 \
      --override-filename "$crate-sbom"
    mv "crates/$crate/$crate-sbom.json" "$dir/rtbit-$client-$tag-sbom.cdx.json"
    ;;
  *)
    echo "unknown mode: $mode" >&2
    exit 2
    ;;
esac

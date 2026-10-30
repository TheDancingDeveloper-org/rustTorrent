#!/usr/bin/env bash
# Build and stage the native client (crates/rtbit-egui) release assets.
#
#   build-egui.sh linux   <tag> <dir>   binary + .deb, built in a container
#   build-egui.sh windows <tag> <dir>   .exe, cross-built with cargo-zigbuild
#   build-egui.sh sbom    <tag> <dir>   CycloneDX SBOM of the rtbit-egui graph
#
# Used by the build job in release.yml and by release-dry-run.yml. The
# rust-heavy runners are shared and not ephemeral, so nothing here changes
# their dpkg or apt state: the Linux build installs the wayland, X11 and GL
# development packages inside a digest-pinned container, and the Windows
# build fetches a checksummed zig into RUNNER_TEMP.
#
# Expects the CARGO_PROFILE_RELEASE_GITHUB_* environment from the caller. The
# Linux build also needs nfpm on PATH.
set -euo pipefail

mode="${1:?usage: build-egui.sh <linux|windows|sbom> <tag> <dir>}"
tag="${2:?usage: build-egui.sh <linux|windows|sbom> <tag> <dir>}"
dir="${3:?usage: build-egui.sh <linux|windows|sbom> <tag> <dir>}"
tmp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
mkdir -p "$dir"

# rust:1.95-bookworm, pinned by index digest.
image="docker.io/library/rust:1.95-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1"

case "$mode" in
  linux)
    command -v docker >/dev/null 2>&1 || { echo "::error::docker is required for the native client Linux build"; exit 1; }
    # Its own target dir, so the container's build never mixes with the
    # daemon's host build. The container runs as root; the tree is handed
    # back to the runner user before it exits.
    docker run --rm \
      -v "$PWD:/src" -w /src \
      -e CARGO_TARGET_DIR=/src/target/egui-linux \
      -e CARGO_PROFILE_RELEASE_GITHUB_LTO \
      -e CARGO_PROFILE_RELEASE_GITHUB_CODEGEN_UNITS \
      -e CARGO_PROFILE_RELEASE_GITHUB_STRIP \
      -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
      "$image" bash -euo pipefail -c '
        trap "chown -R \"\$HOST_UID:\$HOST_GID\" /src/target/egui-linux" EXIT
        apt-get update -qq
        apt-get install -y --no-install-recommends pkg-config \
          libwayland-dev libxkbcommon-dev libx11-dev libxcursor-dev \
          libxrandr-dev libxi-dev libgl1-mesa-dev libegl1-mesa-dev
        cargo build --locked --profile release-github -p rtbit-egui
      '
    bin=target/egui-linux/release-github/rtbit-egui
    test -s "$bin"
    VERSION="${tag#v}" ARCH=amd64 BINARY="$bin" \
      nfpm package --packager deb --config packaging/linux/nfpm-egui.yaml \
      --target "$dir/rtbit-egui-$tag-amd64.deb"
    cp "$bin" "$dir/rtbit-egui-$tag-linux-x86_64"
    ;;
  windows)
    zig="zig-x86_64-linux-0.15.2"
    if ! test -x "$tmp/$zig/zig"; then
      curl -fsSL -o "$tmp/$zig.tar.xz" "https://ziglang.org/download/0.15.2/$zig.tar.xz"
      echo "02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239  $tmp/$zig.tar.xz" | sha256sum -c -
      tar -xJf "$tmp/$zig.tar.xz" -C "$tmp"
    fi
    # windows-gnu raw-dylib imports need a mingw dlltool; zig ships one.
    mkdir -p "$tmp/zig-wrappers"
    printf '#!/bin/sh\nexec zig dlltool "$@"\n' > "$tmp/zig-wrappers/x86_64-w64-mingw32-dlltool"
    chmod +x "$tmp/zig-wrappers/x86_64-w64-mingw32-dlltool"
    if ! test -x "$tmp/zigbuild/bin/cargo-zigbuild"; then
      cargo install --locked cargo-zigbuild --version 0.23.4 --root "$tmp/zigbuild"
    fi
    export PATH="$tmp/$zig:$tmp/zig-wrappers:$tmp/zigbuild/bin:$PATH"
    rustup target add x86_64-pc-windows-gnu
    CARGO_TARGET_DIR=target/egui-windows \
      cargo zigbuild --locked --profile release-github -p rtbit-egui --target x86_64-pc-windows-gnu
    cp target/egui-windows/x86_64-pc-windows-gnu/release-github/rtbit-egui.exe \
      "$dir/rtbit-egui-$tag-windows-x86_64.exe"
    ;;
  sbom)
    if ! command -v cargo-cyclonedx >/dev/null 2>&1; then
      cargo install --locked cargo-cyclonedx --version 0.5.7 --root "$tmp/cyclonedx"
      export PATH="$tmp/cyclonedx/bin:$PATH"
    fi
    cargo-cyclonedx cyclonedx \
      --manifest-path crates/rtbit-egui/Cargo.toml \
      --format json --spec-version 1.5 \
      --override-filename rtbit-egui-sbom
    mv crates/rtbit-egui/rtbit-egui-sbom.json "$dir/rtbit-egui-$tag-sbom.cdx.json"
    ;;
  *)
    echo "unknown mode: $mode" >&2
    exit 2
    ;;
esac

#!/usr/bin/env bash
# Build and stage the native client (crates/rtbit-egui) release assets.
#
#   [CLIENT=egui|fluent] build-egui.sh linux   <tag> <dir>   binary + .deb, built in a container
#   [CLIENT=egui|fluent] build-egui.sh windows <tag> <dir>   .exe, cross-built with cargo-zigbuild
#   [CLIENT=egui|fluent] build-egui.sh sbom    <tag> <dir>   CycloneDX SBOM of the client graph
#
# CLIENT selects the native client: egui (crates/rtbit-egui, the default) or
# fluent (crates/rtbit-fluent). Artifacts are named rtbit-$CLIENT-<tag>-*.
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

client="${CLIENT:-egui}"
case "$client" in
  egui) crate=rtbit-egui ;;
  fluent) crate=rtbit-fluent ;;
  *) echo "unknown CLIENT: $client (want egui or fluent)" >&2; exit 2 ;;
esac

# rust:1.95-bookworm, pinned by index digest.
image="docker.io/library/rust:1.95-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1"

case "$mode" in
  linux)
    command -v docker >/dev/null 2>&1 || { echo "::error::docker is required for the native client Linux build"; exit 1; }
    # The runner may itself run in a container that talks to the host's
    # docker daemon, so a bind mount of $PWD would name a path the daemon
    # cannot see. Copy the tree in and the binary out with docker cp instead.
    cid="$(docker create -w /src \
      -e CARGO_TARGET_DIR=/src/target/egui-linux \
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
    mkdir -p target/egui-linux/release-github
    docker cp "$cid:/src/target/egui-linux/release-github/$crate" "target/egui-linux/release-github/$crate"
    bin=target/egui-linux/release-github/$crate
    test -s "$bin"
    VERSION="${tag#v}" ARCH=amd64 BINARY="$bin" \
      nfpm package --packager deb --config "packaging/linux/nfpm-$client.yaml" \
      --target "$dir/rtbit-$client-$tag-amd64.deb"
    cp "$bin" "$dir/rtbit-$client-$tag-linux-x86_64"
    ;;
  windows)
    if [ "$client" = fluent ]; then
      # TODO(WI-2780): rtbit-fluent Windows is not built until a native Windows job exists
      echo "::error::CLIENT=fluent has no Windows build until a native Windows job exists (WI-2780)" >&2
      exit 2
    fi
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
      cargo zigbuild --locked --profile release-github -p "$crate" --target x86_64-pc-windows-gnu
    cp "target/egui-windows/x86_64-pc-windows-gnu/release-github/$crate.exe" \
      "$dir/rtbit-$client-$tag-windows-x86_64.exe"
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

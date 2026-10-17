#!/usr/bin/env bash
# Checks repository hygiene: licence and notice files, version alignment,
# documented paths and container image tags. Usage: check-repo-hygiene.sh [root]
set -euo pipefail

root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$root"

failures=0
fail() {
  printf 'repo hygiene: %s\n' "$1" >&2
  failures=$((failures + 1))
}

# Root documents that the release and contributor docs link to.
for file in LICENSE NOTICE CHANGELOG.md CONTRIBUTING.md SECURITY.md README.md; do
  [[ -s "$file" ]] || fail "missing or empty $file"
done

# LICENSE must carry the full Apache-2.0 terms, not only the header.
if ! grep -q 'TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION' LICENSE 2>/dev/null; then
  fail 'LICENSE does not contain the full Apache-2.0 terms'
fi

# The desktop package version must match the rtbit release version.
rtbit_version="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/rtbit/Cargo.toml | head -1)"
desktop_version="$(sed -n 's/^  "version": "\(.*\)",/\1/p' desktop/package.json | head -1)"
if [[ -z "$rtbit_version" ]]; then
  fail 'could not read the version from crates/rtbit/Cargo.toml'
elif [[ "$desktop_version" != "$rtbit_version" ]]; then
  fail "desktop/package.json version '$desktop_version' does not match rtbit '$rtbit_version'"
fi

# Every documented reference to the published image must use one tag.
image='ghcr.io/thedancingdeveloper-org/rusttorrent:'
shopt -s nullglob
compose_files=(docker-compose*.yml)
shopt -u nullglob
tags="$(grep -ohi "${image}[A-Za-z0-9._-]*" README.md "${compose_files[@]}" 2>/dev/null | sed "s#^${image}##I" | sort -u || true)"
tag_count="$(printf '%s\n' "$tags" | grep -c . || true)"
if (( tag_count > 1 )); then
  fail "image references use several tags: $(printf '%s ' $tags)"
fi

# Root docs must not point at the retired library layout.
for file in README.md runme.md Makefile; do
  if grep -q 'libs/librtbit' "$file" 2>/dev/null; then
    fail "$file refers to the retired ../libs/librtbit layout"
  fi
done

# The Makefile's web UI directory must exist.
webui_dir="$(sed -n 's/^WEBUI_DIR ?= //p' Makefile | head -1)"
if [[ -z "$webui_dir" || ! -d "$webui_dir" ]]; then
  fail "Makefile WEBUI_DIR '$webui_dir' does not exist"
fi

# The changelog must keep an Unreleased section for pending changes.
if ! grep -q '^## \[Unreleased\]' CHANGELOG.md 2>/dev/null; then
  fail 'CHANGELOG.md has no [Unreleased] section'
fi

if (( failures > 0 )); then
  printf 'repo hygiene: %d check(s) failed\n' "$failures" >&2
  exit 1
fi
echo 'repo hygiene: all checks passed'

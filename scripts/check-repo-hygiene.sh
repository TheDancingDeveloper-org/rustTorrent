#!/usr/bin/env bash
# Checks repository hygiene: licence and notice files, one version across all
# packages, documented paths, release-notes location and container image tags.
# Usage: check-repo-hygiene.sh [root]
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

# NOTICE names the fork's copyright holder.
if ! grep -q 'Copyright rustTorrent contributors' NOTICE 2>/dev/null; then
  fail 'NOTICE does not name the rustTorrent contributors as copyright holder'
fi

# The provenance record states the licence the crates actually carry.
if grep -q 'Apache-2.0/MIT' docs/FORK_PROVENANCE.md 2>/dev/null; then
  fail 'docs/FORK_PROVENANCE.md claims MIT, but the crates are Apache-2.0 only'
fi

# One version for every package: the rtbit release version is the reference.
rtbit_version="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/rtbit/Cargo.toml | head -1)"
if [[ -z "$rtbit_version" ]]; then
  fail 'could not read the version from crates/rtbit/Cargo.toml'
else
  while IFS= read -r manifest; do
    # Virtual workspace manifests have no [package] version to compare.
    grep -q '^\[package\]' "$manifest" || continue
    v="$(sed -n 's/^version = "\(.*\)"/\1/p' "$manifest" | head -1)"
    # Members that inherit take the root [workspace.package] version.
    if [[ -z "$v" ]] && grep -q '^version\.workspace = true' "$manifest"; then
      v="$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
    fi
    [[ "$v" == "$rtbit_version" ]] || fail "$manifest version '$v' does not match rtbit '$rtbit_version'"
  done < <(find . -name Cargo.toml -not -path './target/*' -not -path '*/node_modules/*' -not -path './.git/*')
  desktop_version="$(sed -n 's/^  "version": "\(.*\)",/\1/p' desktop/package.json | head -1)"
  [[ "$desktop_version" == "$rtbit_version" ]] || fail "desktop/package.json version '$desktop_version' does not match rtbit '$rtbit_version'"
  lock_version="$(grep -A1 '"name": "rtbit-desktop"' package-lock.json | sed -n 's/.*"version": "\(.*\)".*/\1/p' | head -1)"
  [[ "$lock_version" == "$rtbit_version" ]] || fail "package-lock.json desktop version '$lock_version' does not match rtbit '$rtbit_version'"
fi

# No upstream author or personal email in the manifests.
if grep -rn 'igor.katson@' --include=Cargo.toml . >/dev/null 2>&1; then
  fail 'a Cargo.toml still names the upstream author with an email'
fi

# Release notes live under docs/releases, one file per tag, and the release
# workflow reads that same path.
if compgen -G 'RELEASE_NOTES_*.md' >/dev/null; then
  fail 'release notes found at the repository root; move them to docs/releases/'
fi
[[ -d docs/releases ]] || fail 'docs/releases/ is missing'
if ! grep -q 'notes="docs/releases/${tag}.md"' .github/workflows/release.yml 2>/dev/null; then
  fail '.github/workflows/release.yml does not read notes from docs/releases/'
fi

# The documentation directory was merged into docs/.
[[ -e documentation ]] && fail 'documentation/ still exists; its contents belong in docs/'

# Documented paths must point at directories that exist, not the retired
# ../libs layout. The historical review record is exempt.
for file in README.md runme.md Makefile desktop/README.md docs/ReleaseProcess.md docs/issues/README.md; do
  if grep -q 'libs/librtbit' "$file" 2>/dev/null; then
    fail "$file refers to the retired ../libs/librtbit layout"
  fi
done

# The Makefile's web UI directory must exist.
webui_dir="$(sed -n 's/^WEBUI_DIR ?= //p' Makefile | head -1)"
if [[ -z "$webui_dir" || ! -d "$webui_dir" ]]; then
  fail "Makefile WEBUI_DIR '$webui_dir' does not exist"
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

# The changelog must keep an Unreleased section for pending changes.
if ! grep -q '^## \[Unreleased\]' CHANGELOG.md 2>/dev/null; then
  fail 'CHANGELOG.md has no [Unreleased] section'
fi

if (( failures > 0 )); then
  printf 'repo hygiene: %d check(s) failed\n' "$failures" >&2
  exit 1
fi
echo 'repo hygiene: all checks passed'

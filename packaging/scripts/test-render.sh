#!/usr/bin/env bash
# Checks the per-release packaging renderers against a fake SHA256SUMS file:
# every placeholder is filled with the matching checksum, and a renderer fails
# closed when an artifact it needs is missing. No network, no real release.
# Run from anywhere; the renderers expect the repository root as cwd.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

tag=v9.9.9-rc.1
fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# One distinct fake checksum per artifact, so a swapped value is caught.
artifacts=(
  "rtbit-$tag-linux-x86_64"
  "rtbit-$tag-service-files.tar.gz"
  "rtbit-$tag-vendor.tar.xz"
  "rtbit-$tag-windows-x86_64.exe"
)
mkdir "$work/full"
for i in "${!artifacts[@]}"; do
  printf '%064d  %s\n' "$((i + 1))" "${artifacts[$i]}"
done >"$work/full/SHA256SUMS-$tag.txt"
sha() { printf '%064d' "$1"; }

for channel in aur homebrew flatpak winget; do
  out="$work/out-$channel"
  bash "packaging/scripts/render-$channel.sh" "$tag" "$work/full" "$out" >/dev/null ||
    fail "render-$channel.sh failed on a complete checksum file"
  [[ -n "$(ls -A "$out")" ]] || fail "render-$channel.sh wrote nothing"
  if grep -rq 'REPLACE_WITH_' "$out"; then
    fail "render-$channel.sh left a placeholder: $(grep -rho 'REPLACE_WITH_[A-Z_]*' "$out" | sort -u | tr '\n' ' ')"
  fi
  grep -rqF "$tag" "$out" || grep -rqF "${tag#v}" "$out" ||
    fail "render-$channel.sh output does not mention the release"
done

grep -qF "$(sha 1)" "$work/out-homebrew/rtbit.rb" || fail "homebrew: wrong linux checksum"
grep -qF "$(sha 1)" "$work/out-aur/PKGBUILD" || fail "aur: wrong binary checksum"
grep -qF "$(sha 2)" "$work/out-aur/PKGBUILD" || fail "aur: wrong service-files checksum"
grep -q 'pkgver=9.9.9rc.1' "$work/out-aur/PKGBUILD" || fail "aur: pkgver keeps the hyphen"
grep -rqF "$(sha 3)" "$work/out-flatpak" || fail "flatpak: wrong vendor checksum"
grep -rqiF "$(sha 4)" "$work/out-winget" || fail "winget: wrong exe checksum"

# Each renderer refuses a checksum file that lacks its artifact.
check_missing() {
  local channel="$1" artifact="$2"
  local dir="$work/missing-$channel"
  mkdir -p "$dir"
  grep -vF "  $artifact" "$work/full/SHA256SUMS-$tag.txt" >"$dir/SHA256SUMS-$tag.txt"
  if bash "packaging/scripts/render-$channel.sh" "$tag" "$dir" "$dir/out" >/dev/null 2>&1; then
    fail "render-$channel.sh succeeded without $artifact"
  fi
}
check_missing homebrew "rtbit-$tag-linux-x86_64"
check_missing aur "rtbit-$tag-service-files.tar.gz"
check_missing flatpak "rtbit-$tag-vendor.tar.xz"
check_missing winget "rtbit-$tag-windows-x86_64.exe"

echo "packaging renderers: all checks passed"

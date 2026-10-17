#!/usr/bin/env bash
# Render packaging/aur/PKGBUILD for one release from its verified SHA256SUMS.
#
#   render-aur.sh <tag> <dir> <out-dir>
#
# Run only after verify-release.sh has passed. Fails closed if an expected
# artefact is missing from the checksum file, and if any placeholder is left.
set -euo pipefail

tag="${1:?usage: render-aur.sh <tag> <dir> <out-dir>}"
dir="${2:?usage: render-aur.sh <tag> <dir> <out-dir>}"
out="${3:?usage: render-aur.sh <tag> <dir> <out-dir>}"

sums="$dir/SHA256SUMS-$tag.txt"
sha_of() {
  awk -v name="$1" '$2 == name { print $1 }' "$sums"
}

bin_sha=$(sha_of "rtbit-$tag-linux-x86_64")
svc_sha=$(sha_of "rtbit-$tag-service-files.tar.gz")
test -n "$bin_sha"
test -n "$svc_sha"

# pkgver may not contain hyphens, so 0.1.0-rc.1 becomes 0.1.0rc.1.
pkgver=$(printf '%s' "${tag#v}" | tr -d '-')

mkdir -p "$out"
sed \
  -e "s/REPLACE_WITH_TAG/$tag/g" \
  -e "s/REPLACE_WITH_AUR_PKGVER/$pkgver/g" \
  -e "s/REPLACE_WITH_SHA256_BINARY/$bin_sha/" \
  -e "s/REPLACE_WITH_SHA256_SERVICE_FILES/$svc_sha/" \
  packaging/aur/PKGBUILD > "$out/PKGBUILD"

if grep -q 'REPLACE_WITH_' "$out/PKGBUILD"; then
  echo "unfilled placeholder left in $out/PKGBUILD" >&2
  exit 1
fi

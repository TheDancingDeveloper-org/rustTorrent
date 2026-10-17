#!/usr/bin/env bash
# Render the Flatpak manifest and metainfo for one release.
#
#   render-flatpak.sh <tag> <dir> <out-dir>
#
# Run from the repository checkout at the tag, after verify-release.sh has
# passed. The manifest's vendored-crates archive digest comes from the verified
# SHA256SUMS; the source commit comes from the checkout. Fails closed if the
# vendor archive is missing from the checksum file, and if any placeholder is
# left in the rendered output.
set -euo pipefail

tag="${1:?usage: render-flatpak.sh <tag> <dir> <out-dir>}"
dir="${2:?usage: render-flatpak.sh <tag> <dir> <out-dir>}"
out="${3:?usage: render-flatpak.sh <tag> <dir> <out-dir>}"

sums="$dir/SHA256SUMS-$tag.txt"
vendor_sha=$(awk -v name="rtbit-$tag-vendor.tar.xz" '$2 == name { print $1 }' "$sums")
test -n "$vendor_sha"

commit=$(git rev-parse HEAD)
release_date=$(git log -1 --format=%cs HEAD)
version=${tag#v}

mkdir -p "$out"
sed \
  -e "s/REPLACE_WITH_TAG/$tag/g" \
  -e "s/REPLACE_WITH_COMMIT/$commit/" \
  -e "s/REPLACE_WITH_SHA256_VENDOR/$vendor_sha/" \
  packaging/flatpak/io.github.TheDancingDeveloper.rustTorrent.yml \
  > "$out/io.github.TheDancingDeveloper.rustTorrent.yml"

sed \
  -e "s/REPLACE_WITH_VERSION/$version/" \
  -e "s/REPLACE_WITH_RELEASE_DATE/$release_date/" \
  packaging/flatpak/io.github.TheDancingDeveloper.rustTorrent.metainfo.xml \
  > "$out/io.github.TheDancingDeveloper.rustTorrent.metainfo.xml"

for f in "$out"/*; do
  if grep -q 'REPLACE_WITH_[A-Z]' "$f"; then
    echo "unfilled placeholder left in $f" >&2
    exit 1
  fi
done

#!/usr/bin/env bash
# Render packaging/homebrew/rtbit.rb for one release from its verified
# SHA256SUMS.
#
#   render-homebrew.sh <tag> <dir> <out-dir>
#
# Run only after verify-release.sh has passed. Fails closed if the Linux
# binary is missing from the checksum file, and if any placeholder is left.
set -euo pipefail

tag="${1:?usage: render-homebrew.sh <tag> <dir> <out-dir>}"
dir="${2:?usage: render-homebrew.sh <tag> <dir> <out-dir>}"
out="${3:?usage: render-homebrew.sh <tag> <dir> <out-dir>}"

sums="$dir/SHA256SUMS-$tag.txt"
bin_sha=$(awk -v name="rtbit-$tag-linux-x86_64" '$2 == name { print $1 }' "$sums")
test -n "$bin_sha"

mkdir -p "$out"
sed \
  -e "s/REPLACE_WITH_TAG/$tag/g" \
  -e "s/REPLACE_WITH_VERSION/${tag#v}/" \
  -e "s/REPLACE_WITH_SHA256_LINUX_BINARY/$bin_sha/" \
  packaging/homebrew/rtbit.rb > "$out/rtbit.rb"

if grep -q 'REPLACE_WITH_[A-Z]' "$out/rtbit.rb"; then
  echo "unfilled placeholder left in $out/rtbit.rb" >&2
  exit 1
fi

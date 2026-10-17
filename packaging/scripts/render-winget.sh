#!/usr/bin/env bash
# Render the winget manifest set for one release from its verified SHA256SUMS.
#
#   render-winget.sh <tag> <dir> <out-dir>
#
# Run only after verify-release.sh has passed. Fails closed if the Windows
# executable is missing from the checksum file, and if any placeholder is left.
# The installer is not Authenticode-signed yet; that is tracked separately.
set -euo pipefail

tag="${1:?usage: render-winget.sh <tag> <dir> <out-dir>}"
dir="${2:?usage: render-winget.sh <tag> <dir> <out-dir>}"
out="${3:?usage: render-winget.sh <tag> <dir> <out-dir>}"

sums="$dir/SHA256SUMS-$tag.txt"
exe_sha=$(awk -v name="rtbit-$tag-windows-x86_64.exe" '$2 == name { print $1 }' "$sums")
test -n "$exe_sha"
# winget expects the digest as 64 upper-case hex characters.
exe_sha=$(printf '%s' "$exe_sha" | tr 'a-f' 'A-F')

version=${tag#v}
src=packaging/winget/TheDancingDeveloper.rustTorrent
mkdir -p "$out"
for f in "$src"/*.yaml; do
  sed \
    -e "s/REPLACE_WITH_TAG/$tag/g" \
    -e "s/REPLACE_WITH_VERSION/$version/g" \
    -e "s/REPLACE_WITH_SHA256_WINDOWS_EXE/$exe_sha/" \
    "$f" > "$out/$(basename "$f")"
done

if grep -q 'REPLACE_WITH_[A-Z]' "$out"/*.yaml; then
  echo "unfilled placeholder left in $out" >&2
  exit 1
fi

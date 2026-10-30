#!/usr/bin/env bash
# Authenticode-sign the Windows executable in a release directory and
# regenerate SHA256SUMS-<tag>.txt so it lists the signed bytes.
#
#   sign-windows.sh <tag> <dir>
#
# Reads the signing key from the environment, never from argv:
#   RUSTTORRENT_AUTHENTICODE_PFX_B64       base64 PKCS#12 (key and certificate)
#   RUSTTORRENT_AUTHENTICODE_PFX_PASSWORD  its password
#   AUTHENTICODE_TSA_URL                   RFC 3161 timestamp server (optional)
# The decoded PKCS#12 and the password file live in a private temp directory
# that is removed on exit.
set -euo pipefail

tag="${1:?usage: sign-windows.sh <tag> <dir>}"
dir="${2:?usage: sign-windows.sh <tag> <dir>}"
: "${RUSTTORRENT_AUTHENTICODE_PFX_B64:?RUSTTORRENT_AUTHENTICODE_PFX_B64 is not set}"
: "${RUSTTORRENT_AUTHENTICODE_PFX_PASSWORD:?RUSTTORRENT_AUTHENTICODE_PFX_PASSWORD is not set}"
tsa="${AUTHENTICODE_TSA_URL:-http://timestamp.digicert.com}"

# The daemon and the native client executables.
exes=("$dir/rtbit-$tag-windows-x86_64.exe")
if test -f "$dir/rtbit-egui-$tag-windows-x86_64.exe"; then
  exes+=("$dir/rtbit-egui-$tag-windows-x86_64.exe")
fi
test -f "${exes[0]}"

umask 077
work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/authenticode.XXXXXX")
trap 'rm -rf "$work"' EXIT
printf '%s' "$RUSTTORRENT_AUTHENTICODE_PFX_B64" | base64 -d > "$work/key.pfx"
printf '%s' "$RUSTTORRENT_AUTHENTICODE_PFX_PASSWORD" > "$work/pass"

for exe in "${exes[@]}"; do
  osslsigncode sign \
    -pkcs12 "$work/key.pfx" \
    -readpass "$work/pass" \
    -h sha256 \
    -n "rustTorrent" \
    -i "https://github.com/TheDancingDeveloper-org/rustTorrent" \
    -ts "$tsa" \
    -in "$exe" -out "$work/signed.exe"
  install -m 644 "$work/signed.exe" "$exe"
done

(cd "$dir" && sha256sum rtbit-*) > "$dir/SHA256SUMS-$tag.txt"

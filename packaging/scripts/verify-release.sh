#!/usr/bin/env bash
# Verify a signed release directory before anything is published or rendered
# into a channel manifest.
#
#   verify-release.sh <tag> <dir>
#
# Checks the keyless cosign signature on SHA256SUMS-<tag>.txt against the
# release.yml workflow identity, then checks every artefact the file lists,
# then checks the Authenticode signature and RFC 3161 timestamp on each Windows
# executable against the public signing certificate, which is read base64 PEM
# from RUSTTORRENT_AUTHENTICODE_CERT_PEM_B64.
# Fails closed on any missing file, bad signature or digest mismatch.
set -euo pipefail

tag="${1:?usage: verify-release.sh <tag> <dir>}"
dir="${2:?usage: verify-release.sh <tag> <dir>}"
: "${RUSTTORRENT_AUTHENTICODE_CERT_PEM_B64:?RUSTTORRENT_AUTHENTICODE_CERT_PEM_B64 is not set}"

sums="$dir/SHA256SUMS-$tag.txt"
test -f "$sums.sig"
test -f "$sums.pem"

cosign verify-blob \
  --certificate "$sums.pem" \
  --signature "$sums.sig" \
  --certificate-identity-regexp '^https://github.com/TheDancingDeveloper-org/rustTorrent/\.github/workflows/release\.yml@refs/(tags|heads)/.+$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "$sums"

(cd "$dir" && sha256sum -c "SHA256SUMS-$tag.txt")

work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/authenticode-verify.XXXXXX")
trap 'rm -rf "$work"' EXIT
printf '%s' "$RUSTTORRENT_AUTHENTICODE_CERT_PEM_B64" | base64 -d > "$work/cert.pem"
tsa_ca="${AUTHENTICODE_TSA_CAFILE:-/etc/ssl/certs/ca-certificates.crt}"
test -f "$dir/rtbit-$tag-windows-x86_64.exe"
# Every Windows executable: the daemon and, when shipped, the native client.
for exe in "$dir"/rtbit*-"$tag"-windows-x86_64.exe; do
  osslsigncode verify -CAfile "$work/cert.pem" -TSA-CAfile "$tsa_ca" -in "$exe" | tee "$work/verify.log"
  # osslsigncode passes an untimestamped file; the release must carry one.
  grep -q '^Timestamp Server Signature verification: ok' "$work/verify.log"
done

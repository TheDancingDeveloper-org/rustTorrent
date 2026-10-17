#!/usr/bin/env bash
# Verify a signed release directory before anything is published or rendered
# into a channel manifest.
#
#   verify-release.sh <tag> <dir>
#
# Checks the keyless cosign signature on SHA256SUMS-<tag>.txt against the
# release.yml workflow identity, then checks every artefact the file lists.
# Fails closed on any missing file, bad signature or digest mismatch.
set -euo pipefail

tag="${1:?usage: verify-release.sh <tag> <dir>}"
dir="${2:?usage: verify-release.sh <tag> <dir>}"

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

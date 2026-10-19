#!/usr/bin/env bash
# Brings up tests/sonarr-e2e (rustTorrent + Sonarr), runs smoke.py from a
# container on the compose network, and tears the stack down.
#
#   RTBIT_IMAGE=ghcr.io/thedancingdeveloper-org/rusttorrent:staging tests/sonarr-e2e/run.sh
set -euo pipefail

dir=$(cd "$(dirname "$0")" && pwd)
project=${SONARR_E2E_PROJECT:-sonarr-e2e}
compose=(docker compose -p "$project" -f "$dir/docker-compose.yml")

cleanup() { "${compose[@]}" down -v -t 5 >/dev/null 2>&1 || true; }
trap cleanup EXIT

"${compose[@]}" up -d --quiet-pull --wait --wait-timeout 300

# Sonarr writes its API key into config.xml on first start.
key=
for _ in $(seq 60); do
  key=$("${compose[@]}" exec -T sonarr sh -c \
    "sed -n 's:.*<ApiKey>\(.*\)</ApiKey>.*:\1:p' /config/config.xml" 2>/dev/null || true)
  [ -n "$key" ] && break
  sleep 2
done
[ -n "$key" ] || { echo "Sonarr API key not found" >&2; exit 1; }

status=0
docker run --rm -i --network "${project}_default" -e SONARR_API_KEY="$key" \
  python:3.12-alpine python3 -I - "$@" <"$dir/smoke.py" || status=$?

if [ "$status" -ne 0 ]; then
  "${compose[@]}" logs --tail 60 >&2 || true
fi
exit "$status"

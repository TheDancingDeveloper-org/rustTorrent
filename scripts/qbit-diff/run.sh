#!/usr/bin/env bash
# Brings up the differential stack, replays the client fixtures against real
# qBittorrent and rustTorrent, writes the report, and tears the stack down.
#
#   RTBIT_IMAGE=ghcr.io/thedancingdeveloper-org/rusttorrent:staging \
#     scripts/qbit-diff/run.sh [report.md] [extra qbit_diff.py args...]
#
# The harness runs in a container on the compose network, so it works even when
# the docker daemon does not share the caller's network namespace.
set -euo pipefail

repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
report=${1:-qbit-diff-report.md}
shift || true
project=${QBIT_DIFF_PROJECT:-qbitdiff}
compose=(docker compose -p "$project" -f "$repo/scripts/qbit-diff/docker-compose.yml")

cleanup() { "${compose[@]}" down -v -t 5 >/dev/null 2>&1 || true; }
trap cleanup EXIT

# Host ports are not needed by the in-network harness; keep them off defaults.
export QBIT_PORT=${QBIT_PORT:-28781} RTBIT_PORT=${RTBIT_PORT:-28782}
"${compose[@]}" up -d --quiet-pull

status=0
tar -C "$repo" -c \
    scripts/qbit-diff/qbit_diff.py \
    crates/librtbit/src/http_api/handlers/qbit_replay \
    crates/librtbit/src/http_api/handlers/qbit_parity_spec.json |
  docker run --rm -i --network "${project}_default" python:3.12-alpine sh -c '
    mkdir /w && cd /w && tar x >&2 &&
    python3 -I scripts/qbit-diff/qbit_diff.py \
      --reference http://qbittorrent:8080 --candidate http://rtbit:3030 \
      --report /w/report.md "$@" >&2; rc=$?; cat /w/report.md; exit $rc
  ' sh "$@" >"$report" || status=$?

if [ "$status" -ne 0 ]; then
  echo "--- qbittorrent logs ---" >&2; "${compose[@]}" logs --tail 40 qbittorrent >&2 || true
  echo "--- rtbit logs ---" >&2; "${compose[@]}" logs --tail 40 rtbit >&2 || true
fi
exit "$status"

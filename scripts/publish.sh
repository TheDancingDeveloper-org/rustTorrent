#!/usr/bin/env bash
set -euo pipefail

publish_flag="--dry-run"
resume=false
if [[ "${1:-}" == "--execute" ]]; then
  publish_flag=""
  shift
fi
if [[ "${1:-}" == "--resume" ]]; then
  resume=true
  shift
fi
[[ $# -eq 0 ]] || {
  echo "usage: $0 [--execute [--resume]]" >&2
  exit 2
}
if $resume && [[ -n "$publish_flag" ]]; then
  echo '--resume is valid only with --execute' >&2
  exit 2
fi

version="0.1.0-rc.1"

# Pre-release versions (a '-' suffix such as 0.1.0-rc.1) are never published to
# crates.io from this script. Exit 0 so a release pipeline that calls it for every
# tag does not fail on a pre-release tag.
if [[ "$version" == *-* ]]; then
  echo "skipping pre-release $version: crates.io publication is for final versions only"
  exit 0
fi

# Keep this list explicit: Cargo package publication must follow dependency
# order, and directory discovery can publish the binary or a leaf before the
# family it depends on. The old librtbit-* repositories and packages remain
# available as rollback assets; this script publishes only the coordinated
# swarmforge family.
packages=(
  swarmforge-clone-to-owned
  swarmforge-buffers
  swarmforge-sha1-wrapper
  swarmforge-bencode
  swarmforge-core
  swarmforge-peer-protocol
  swarmforge-dht
  swarmforge-lsd
  swarmforge-tracker-comms
  swarmforge-upnp
  swarmforge-upnp-serve
  swarmforge
)

sparse_path() {
  local package="$1"
  case "${#package}" in
    1) printf '1/%s' "$package" ;;
    2) printf '2/%s' "$package" ;;
    3) printf '3/%s/%s' "${package:0:1}" "$package" ;;
    *) printf '%s/%s/%s' "${package:0:2}" "${package:2:2}" "$package" ;;
  esac
}

package_is_visible() {
  local package="$1" body
  body="$(curl --fail --silent --show-error \
    --user-agent 'cargo swarmforge-release' \
    "https://index.crates.io/$(sparse_path "$package")")" || return 1
  jq --exit-status --arg version "$version" \
    'select(.vers == $version and (.yanked | not))' \
    >/dev/null <<<"$body"
}

package_status() {
  curl --silent --output /dev/null --write-out '%{http_code}' \
    --user-agent 'cargo swarmforge-release' \
    "https://index.crates.io/$(sparse_path "$1")"
}

wait_until_epoch() {
  local epoch="$1" now
  while true; do
    now="$(date -u +%s)"
    ((now >= epoch)) && return
    sleep 10
  done
}

publish_with_rate_limit() {
  local package="$1" output retry_text retry_epoch
  while true; do
    if output="$(cargo publish --locked -p "$package" 2>&1)"; then
      printf '%s\n' "$output"
      return
    fi
    printf '%s\n' "$output" >&2
    retry_text="$(sed -n 's/.*Please try again after \(.* GMT\) and see.*/\1/p' \
      <<<"$output" | tail -n 1)"
    [[ -n "$retry_text" ]] || return 1
    retry_epoch="$(( $(date -u -d "$retry_text" +%s) + 5 ))"
    echo "rate limit for $package; retrying after $(date -u -d "@$retry_epoch" +%Y-%m-%dT%H:%M:%SZ)" >&2
    wait_until_epoch "$retry_epoch"
    package_is_visible "$package" && {
      echo "$package became visible after a rejected upload; refusing a duplicate" >&2
      return 1
    }
  done
}

if [[ -z "$publish_flag" ]]; then
  command -v curl >/dev/null
  command -v jq >/dev/null
  [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]] || {
    echo 'CARGO_REGISTRY_TOKEN is required for --execute' >&2
    exit 1
  }

  # Names are checked as one family immediately before the first upload. A
  # normal release requires every name to be vacant. Explicit resume mode is
  # only for recovery after a partially accepted family publication: it skips
  # an existing, non-yanked 0.1.0 and still rejects every other registry state.
  for package in "${packages[@]}"; do
    status="$(package_status "$package")"
    if $resume && [[ "$status" == "200" ]] && package_is_visible "$package"; then
      continue
    fi
    [[ "$status" == "404" ]] || {
      echo "refusing publication: $package sparse-index status is $status, expected 404" >&2
      exit 1
    }
  done
fi

for package in "${packages[@]}"; do
  if [[ -z "$publish_flag" ]] && $resume && package_is_visible "$package"; then
    echo "verified existing $package $version; skipping"
    continue
  fi
  cargo publish --locked --dry-run -p "$package" "$@"
  [[ -z "$publish_flag" ]] || continue

  publish_with_rate_limit "$package"
  for _ in {1..60}; do
    package_is_visible "$package" && break
    sleep 2
  done
  package_is_visible "$package" || {
    echo "$package $version was uploaded but did not appear in the sparse index" >&2
    exit 1
  }
done

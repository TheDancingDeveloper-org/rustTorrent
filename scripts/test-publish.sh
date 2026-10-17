#!/usr/bin/env bash
# Checks the pre-release guard in scripts/publish.sh. Each case runs a copy of the
# script with its version swapped in, against a stub cargo that only logs its arguments.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
publish="$here/publish.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir "$work/bin"
cat >"$work/bin/cargo" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$CARGO_LOG"
EOF
chmod +x "$work/bin/cargo"
export PATH="$work/bin:$PATH" CARGO_LOG="$work/cargo.log"
unset CARGO_REGISTRY_TOKEN

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# run_publish <version> [publish.sh args...] runs a copy of publish.sh with the given version.
run_publish() {
  local version="$1"
  shift
  sed "s/^version=\"0.1.0\"\$/version=\"$version\"/" "$publish" >"$work/publish.sh"
  grep -q "^version=\"$version\"\$" "$work/publish.sh" || fail "could not set version to $version"
  : >"$CARGO_LOG"
  bash "$work/publish.sh" "$@"
}

# A pre-release exits 0 with the skip message and never reaches cargo.
out="$(run_publish 0.1.0-rc.1)" || fail "dry-run of a pre-release exited non-zero"
grep -q "skipping pre-release 0.1.0-rc.1" <<<"$out" || fail "no skip message for a pre-release"
[[ ! -s "$CARGO_LOG" ]] || fail "cargo ran for a pre-release (dry-run)"

# The skip also applies to --execute, before the CARGO_REGISTRY_TOKEN check.
out="$(run_publish 0.1.0-rc.1 --execute)" || fail "--execute of a pre-release exited non-zero"
grep -q "skipping pre-release 0.1.0-rc.1" <<<"$out" || fail "no skip message for --execute"
[[ ! -s "$CARGO_LOG" ]] || fail "cargo ran for a pre-release (--execute)"

# A final version is not skipped and still runs the dry-run path.
out="$(run_publish 0.1.0)" || fail "dry-run of a final version exited non-zero"
! grep -q "skipping pre-release" <<<"$out" || fail "final version was skipped"
[[ -s "$CARGO_LOG" ]] || fail "cargo was not invoked for a final version"
! grep -qv -- "--dry-run" "$CARGO_LOG" || fail "a final-version run invoked cargo without --dry-run"

echo "publish pre-release guard: ok"

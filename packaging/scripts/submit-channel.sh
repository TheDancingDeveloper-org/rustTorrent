#!/usr/bin/env bash
# Submit one rendered distribution channel for a published release.
#
#   submit-channel.sh <channel> <tag> <rendered-dir>
#
# <channel> is aur, homebrew, winget or flatpak; <rendered-dir> holds the
# output of the matching render-*.sh, which ran against the verified,
# cosign-signed SHA256SUMS. Each channel needs its own credential (below). A
# channel whose credential is unset is skipped with a notice, so a release is
# never failed by a channel that has not been set up yet.
#
#   aur       AUR_SSH_KEY, AUR_KNOWN_HOSTS (pinned host key, fails closed)
#   homebrew  HOMEBREW_TAP_TOKEN; tap repo HOMEBREW_TAP (default below)
#   winget    WINGET_TOKEN: a PAT whose owner has a fork of winget-pkgs
#   flatpak   FLATHUB_TOKEN: a PAT with push access to a fork of the Flathub
#             app repo (the first Flathub submission is manual)
set -euo pipefail

channel="${1:?usage: submit-channel.sh <channel> <tag> <rendered-dir>}"
tag="${2:?usage: submit-channel.sh <channel> <tag> <rendered-dir>}"
dir="${3:?usage: submit-channel.sh <channel> <tag> <rendered-dir>}"
version=${tag#v}
app_id=io.github.TheDancingDeveloper.rustTorrent
winget_id=TheDancingDeveloper.rustTorrent

skip() {
  echo "::notice::$channel: $1 is not set; skipping submission of $tag"
  exit 0
}

# put_file <repo> <branch> <path> <local-file> <message>
# Create or update one file through the contents API, so no large clone
# (winget-pkgs) is needed.
put_file() {
  local repo=$1 branch=$2 path=$3 file=$4 message=$5 sha
  sha=$(gh api "repos/$repo/contents/$path?ref=$branch" -q .sha 2>/dev/null || true)
  local args=(-X PUT "repos/$repo/contents/$path" -f "message=$message" -f "branch=$branch"
    -f "content=$(base64 -w0 < "$file")")
  if [[ -n "$sha" ]]; then
    args+=(-f "sha=$sha")
  fi
  gh api "${args[@]}" > /dev/null
}

# fork_pr <upstream> <base> <dest-prefix> <title> <file>...
# Sync the token owner's fork, write the files on a fresh branch, and open a
# pull request against upstream.
fork_pr() {
  local upstream=$1 base=$2 prefix=$3 title=$4
  shift 4
  local owner fork branch base_sha f
  owner=$(gh api user -q .login)
  fork="$owner/${upstream#*/}"
  gh repo fork "$upstream" --clone=false >/dev/null 2>&1 || true
  gh api -X POST "repos/$fork/merge-upstream" -f "branch=$base" >/dev/null
  branch="rtbit-$tag"
  base_sha=$(gh api "repos/$fork/git/ref/heads/$base" -q .object.sha)
  gh api -X POST "repos/$fork/git/refs" -f "ref=refs/heads/$branch" -f "sha=$base_sha" >/dev/null
  for f in "$@"; do
    put_file "$fork" "$branch" "${prefix:+$prefix/}$(basename "$f")" "$f" "$title"
  done
  gh pr create --repo "$upstream" --base "$base" --head "$owner:$branch" \
    --title "$title" --body "Automated submission from the signed rustTorrent $tag release."
}

case "$channel" in
  aur)
    [[ -n "${AUR_SSH_KEY:-}" ]] || skip AUR_SSH_KEY
    [[ -n "${AUR_KNOWN_HOSTS:-}" ]] || { echo "::error::aur: AUR_KNOWN_HOSTS is required with AUR_SSH_KEY"; exit 1; }
    work=$(mktemp -d)
    install -d -m 700 "$work/ssh"
    printf '%s\n' "$AUR_SSH_KEY" > "$work/ssh/id"
    chmod 600 "$work/ssh/id"
    printf '%s\n' "$AUR_KNOWN_HOSTS" > "$work/ssh/known_hosts"
    export GIT_SSH_COMMAND="ssh -i $work/ssh/id -o UserKnownHostsFile=$work/ssh/known_hosts -o StrictHostKeyChecking=yes"
    git clone ssh://aur@aur.archlinux.org/rtbit-bin.git "$work/aur"
    cp "$dir/PKGBUILD" "$work/aur/PKGBUILD"
    # .SRCINFO must come from makepkg itself; run it in an Arch container as
    # an unprivileged user.
    docker run --rm -v "$work/aur:/pkg" archlinux:base-devel bash -c \
      'useradd -m b && cp /pkg/PKGBUILD /home/b/ && cd /home/b && su b -c "makepkg --printsrcinfo" > /pkg/.SRCINFO'
    git -C "$work/aur" add PKGBUILD .SRCINFO
    if git -C "$work/aur" diff --cached --quiet; then
      echo "aur: already at $tag"
      exit 0
    fi
    git -C "$work/aur" -c user.name="rustTorrent release" -c user.email="admin@rusttorrent.dev" \
      commit -m "Update to $version"
    git -C "$work/aur" push origin HEAD:master
    ;;
  homebrew)
    [[ -n "${HOMEBREW_TAP_TOKEN:-}" ]] || skip HOMEBREW_TAP_TOKEN
    export GH_TOKEN=$HOMEBREW_TAP_TOKEN
    tap=${HOMEBREW_TAP:-TheDancingDeveloper-org/homebrew-tap}
    tap_branch=$(gh api "repos/$tap" -q .default_branch)
    put_file "$tap" "$tap_branch" Formula/rtbit.rb "$dir/rtbit.rb" "rtbit $version"
    ;;
  winget)
    [[ -n "${WINGET_TOKEN:-}" ]] || skip WINGET_TOKEN
    export GH_TOKEN=$WINGET_TOKEN
    fork_pr microsoft/winget-pkgs master \
      "manifests/t/${winget_id%%.*}/${winget_id#*.}/$version" \
      "New version: $winget_id version $version" "$dir"/*.yaml
    ;;
  flatpak)
    [[ -n "${FLATHUB_TOKEN:-}" ]] || skip FLATHUB_TOKEN
    export GH_TOKEN=$FLATHUB_TOKEN
    fork_pr "flathub/$app_id" master "" "Update to $version" "$dir"/*
    ;;
  *)
    echo "unknown channel: $channel" >&2
    exit 2
    ;;
esac

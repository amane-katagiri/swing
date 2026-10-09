# shellcheck shell=sh disable=SC2034 # the variables are used by the scripts that source this file
set -eu

start=$(pwd)
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

die() {
  echo "error: $*" >&2
  exit 1
}

parse_version() {
  [ $# -ge 1 ] || die "missing <version> (for example 0.1.3)"
  version=${1#v}
  printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' ||
    die "not a version: $1"
  tag=v$version
}

package_version() {
  sed -n 's/^version = "\(.*\)"$/\1/p' "$1" | head -n 1
}

require_clean_main() {
  [ "$(git rev-parse --abbrev-ref HEAD)" = main ] || die "not on main"
  [ -z "$(git status --porcelain)" ] || die "the working tree has changes"
  git fetch -q origin main
  git merge-base --is-ancestor origin/main HEAD || die "main is behind origin/main"
}

commit() {
  if [ -n "${SWING_RELEASE_TRAILER:-}" ]; then
    git commit -q -m "$1" -m "$SWING_RELEASE_TRAILER"
  else
    git commit -q -m "$1"
  fi
}

release_is_draft() {
  draft=$(gh release view "$tag" --json isDraft --jq .isDraft 2>/dev/null) || die "there is no release $tag"
  [ "$draft" = true ]
}

check_workflows="release macos-check windows-check windows-installer-check homebrew-check"

require_versions() {
  for f in Cargo.toml tray/Cargo.toml; do
    [ "$(package_version "$f")" = "$version" ] || die "$f is not $version; run bump.sh first"
  done
}

latest_check_run() {
  gh run list --workflow "$1.yml" --commit "$2" --event workflow_dispatch --limit 1 \
    --json databaseId,status,conclusion --jq '.[0] | "\(.databaseId) \(.status) \(.conclusion)"'
}

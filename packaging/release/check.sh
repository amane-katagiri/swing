#!/bin/sh
# usage: check.sh <version>
. "$(dirname "$0")/common.sh"
parse_version "$@"
require_clean_main
require_versions
sha=$(git rev-parse HEAD)
branch=release-$version
git push -q -f origin "HEAD:refs/heads/$branch"
for w in $check_workflows; do
  gh workflow run "$w.yml" --ref "$branch"
done
failed=
for w in $check_workflows; do
  run=
  for _ in $(seq 60); do
    run=$(latest_check_run "$w" "$sha" | cut -d' ' -f1)
    [ -n "$run" ] && [ "$run" != null ] && break
    sleep 5
  done
  [ -n "$run" ] && [ "$run" != null ] || die "the $w run on $branch did not start"
  if gh run watch "$run" --exit-status >/dev/null 2>&1; then
    echo "ok: $w ($run)"
  else
    echo "FAILED: $w ($run)"
    failed="$failed $w"
  fi
done
git push -q origin --delete "$branch"
[ -z "$failed" ] || die "failed:$failed"
echo "all checks passed on $sha"

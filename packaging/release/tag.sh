#!/bin/sh
# usage: tag.sh <version>
. "$(dirname "$0")/common.sh"
parse_version "$@"
require_clean_main
for f in Cargo.toml tray/Cargo.toml; do
  [ "$(package_version "$f")" = "$version" ] || die "$f is not $version; run bump.sh first"
done
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "$tag already exists"
git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null && die "$tag already exists on origin"
git push -q origin main
git tag -a "$tag" -m "$tag"
git push -q origin "$tag"
echo "pushed $tag; the release workflow will create a draft release"

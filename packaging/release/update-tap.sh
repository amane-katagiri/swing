#!/bin/sh
# usage: update-tap.sh <version>
. "$(dirname "$0")/common.sh"
parse_version "$@"
! release_is_draft || die "$tag is still a draft; publish it first (the formula downloads its archives)"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
gh release download "$tag" --pattern swing.rb --dir "$work"
grep -q "/releases/download/$tag/" "$work/swing.rb" || die "swing.rb of $tag does not point at $tag"
gh repo clone amane-katagiri/homebrew-swing "$work/tap" -- -q
cp "$work/swing.rb" "$work/tap/Formula/swing.rb"
cd "$work/tap"
if [ -z "$(git status --porcelain)" ]; then
  echo "the tap already has the formula of $tag"
  exit 0
fi
git add Formula/swing.rb
commit "Update the swing formula for $tag"
git push -q origin HEAD
echo "updated the tap to $tag"

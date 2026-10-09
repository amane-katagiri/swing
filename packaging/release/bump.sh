#!/bin/sh
# usage: bump.sh <version>
. "$(dirname "$0")/common.sh"
parse_version "$@"
require_clean_main
for f in Cargo.toml tray/Cargo.toml; do
  [ "$(package_version "$f")" != "$version" ] || die "$f is already $version"
  awk -v v="$version" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' "$f" >"$f.new"
  mv "$f.new" "$f"
  [ "$(package_version "$f")" = "$version" ] || die "could not set the version in $f"
done
cargo update -q --workspace --offline
git add Cargo.toml tray/Cargo.toml Cargo.lock
commit "Bump the version to $version"
git log --oneline -1

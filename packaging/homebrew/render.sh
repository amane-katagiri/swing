#!/bin/sh
set -eu

if [ $# -lt 3 ] || [ $# -gt 4 ]; then
  echo "usage: $0 <tag> <SHA256SUMS> <output swing.rb> [<download url base>]" >&2
  exit 2
fi

tag=$1
sums=$2
out=$3
base=${4:-https://github.com/amane-katagiri/swing/releases/download/$tag}
here=$(cd "$(dirname "$0")" && pwd)

case $tag in
  '' | *[!0-9A-Za-z._/+-]*)
    echo "invalid tag: $tag" >&2
    exit 1
    ;;
esac
case $base in
  *[\"\\#[:space:]]*)
    echo "invalid download url base: $base" >&2
    exit 1
    ;;
esac

sha() {
  file="swing-$tag-$1-apple-darwin.tar.gz"
  hash=$(awk -v f="$file" '{ n = $2; sub(/^\*/, "", n) } n == f { print $1; exit }' "$sums")
  case $hash in
    *[!0-9a-f]* | "") ;;
    *) [ ${#hash} -eq 64 ] && echo "$hash" && return ;;
  esac
  echo "no SHA-256 for $file in $sums" >&2
  exit 1
}

escape() {
  printf '%s' "$1" | sed 's/[|&]/\\&/g'
}

arm=$(sha aarch64)
intel=$(sha x86_64)

sed \
  -e "s|@TAG@|$(escape "$tag")|g" \
  -e "s|@URL_BASE@|$(escape "${base%/}")|g" \
  -e "s|@SHA256_AARCH64@|$arm|g" \
  -e "s|@SHA256_X86_64@|$intel|g" \
  "$here/swing.rb.in" > "$out"

if grep -q '@[A-Z0-9_]*@' "$out"; then
  echo "unreplaced placeholder in $out" >&2
  exit 1
fi

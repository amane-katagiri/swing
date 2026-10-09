#!/bin/sh
# usage: notes.sh <version> <notes.md>
. "$(dirname "$0")/common.sh"
parse_version "$@"
[ $# -eq 2 ] || die "usage: $0 <version> <notes.md>"
notes=$2
case $notes in /*) ;; *) notes=$start/$notes ;; esac
[ -s "$notes" ] || die "no notes in $2"
release_is_draft || die "$tag is already published"
gh release edit "$tag" --notes-file "$notes"
echo "updated the notes of the draft $tag"

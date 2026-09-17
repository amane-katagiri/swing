#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 <site> <url> <dir> [message]" >&2
  echo "  site  d-tag identifying the site (e.g. ama.ne.jp)" >&2
  echo "  url   https URL of the site (e.g. https://ama.ne.jp/)" >&2
  echo "  dir   directory to publish (e.g. ./public)" >&2
  echo "  message  optional update note for readers (event content)" >&2
  exit 1
}

[ "$#" -eq 3 ] || [ "$#" -eq 4 ] || usage

SITE="$1"
URL="$2"
DIR="$3"
MESSAGE="${4:-}"

for bin in ipfs nak; do
  command -v "$bin" >/dev/null 2>&1 || {
    echo "error: '$bin' command not found in PATH" >&2
    exit 1
  }
done

: "${NOSTR_SEC:?NOSTR_SEC (nsec or hex secret key) must be set}"
NOSTR_RELAYS="${NOSTR_RELAYS:-wss://relay.damus.io wss://nos.lol wss://relay.primal.net wss://yabu.me wss://relay-jp.nostr.wirednet.jp}"
SITE_EVENT_KIND="${SITE_EVENT_KIND:-35980}"

[ -d "$DIR" ] || {
  echo "error: '$DIR' is not a directory" >&2
  exit 1
}

echo "Site: $URL"
echo

echo "IPFS"
CID=$(ipfs add -Qr --cid-version=1 "$DIR")
echo "  CID: $CID"
echo "  added"

# `ipfs files stat` also accepts immutable /ipfs/<cid> paths, not just MFS paths.
SIZE=$(ipfs files stat --size "/ipfs/$CID")
echo "  pinned"
echo

echo "Nostr"
# shellcheck disable=SC2086  # NOSTR_RELAYS is intentionally word-split into multiple args
EVENT_JSON=$(nak event \
  -k "$SITE_EVENT_KIND" \
  -d "$SITE" \
  -t cid="$CID" \
  -t url="$URL" \
  -t size="$SIZE" \
  -t alt="SWING site announcement: $SITE" \
  -c "$MESSAGE" \
  --sec "$NOSTR_SEC" \
  $NOSTR_RELAYS)
echo

EVENT_ID=$(printf '%s' "$EVENT_JSON" | grep -o '"id":"[^"]*"' | head -n1 | cut -d'"' -f4 || true)

echo "Published."
if [ -n "$EVENT_ID" ]; then
  echo "Event ID: $EVENT_ID"
fi

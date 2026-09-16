#!/bin/sh
set -e

: "${SWING_GATEWAY_HOSTS:?set SWING_GATEWAY_HOSTS to use the gateway profile}"
SWING_GATEWAY_HOST_LIST=$(echo "$SWING_GATEWAY_HOSTS" | tr ',' ' ')
export SWING_GATEWAY_HOST_LIST
exec caddy run --config /etc/caddy/Caddyfile --adapter caddyfile

#!/bin/sh
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
env_file="$root/docker/demo/demo.env"
seeded="$root/docker/demo/.seeded"

compose() {
  files="-f $root/compose.yaml -f $root/docker/demo/compose.yaml"
  if [ "${SWING_DEMO_NIP05:-0}" = 1 ]; then
    files="$files -f $root/docker/demo/nip05.yaml"
  fi
  # --env-file keeps the repository's real .env out of compose interpolation.
  docker compose --project-directory "$root" --env-file "$env_file" $files "$@"
}

write_env() {
  out=$(docker run --rm --network none --entrypoint swing swing-demo-mirror key generate)
  secret=$(printf '%s\n' "$out" | sed -n 's/^hex (secret): //p')
  public=$(printf '%s\n' "$out" | sed -n 's/^hex (public): //p')
  [ -n "$secret" ] && [ -n "$public" ] || { echo "key generation failed" >&2; exit 1; }
  chmod 600 "$env_file"
  cat > "$env_file" <<ENV
SWING_NOSTR_SECRET_KEY=$secret
SWING_NOSTR_RELAYS=ws://relay:8080
SWING_MIRROR_SET=swing-demo
SWING_MAX_TOTAL_STORAGE=2GB
SWING_DASHBOARD_GATEWAY=http://localhost:18080
SWING_NIP05=off
SWING_DEMO_SELF_PUBLIC=$public
ENV
}

case "${1:-}" in
  up)
    [ -f "$env_file" ] || : > "$env_file"
    compose build mirror
    grep -q '^SWING_NOSTR_SECRET_KEY=.' "$env_file" || write_env
    compose up -d
    if [ "${2:-}" = --seed ] && [ ! -f "$seeded" ]; then
      "$root/docker/demo/seed.sh"
      touch "$seeded"
    fi
    echo "dashboard: http://127.0.0.1:18082/"
    ;;
  seed)
    "$root/docker/demo/seed.sh"
    touch "$seeded"
    ;;
  down)
    [ -f "$env_file" ] || : > "$env_file"
    compose down -v
    rm -f "$env_file" "$root/docker/demo/personas.env" "$seeded"
    ;;
  "")
    echo "usage: $0 up [--seed] | seed | down | <docker compose args>" >&2
    exit 2
    ;;
  *)
    [ -f "$env_file" ] || : > "$env_file"
    compose "$@"
    ;;
esac

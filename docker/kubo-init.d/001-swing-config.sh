#!/bin/sh
set -e

ipfs config Datastore.StorageMax "${SWING_KUBO_STORAGE_MAX:?}"
ipfs config Provide.Strategy "${SWING_KUBO_PROVIDE_STRATEGY:?}"

ipfs config --json Gateway.NoFetch true
ipfs config --json Gateway.NoDNSLink true

gateways=""
for host in $(echo "${SWING_GATEWAY_HOSTS:-}" | tr ',' ' '); do
  case "$host" in
    *[!a-z0-9.-]* | .* | *. | *..*)
      echo "SWING_GATEWAY_HOSTS: invalid hostname: $host" >&2
      exit 1
      ;;
  esac
  gateways="${gateways:+$gateways,}\"$host\":{\"Paths\":[],\"UseSubdomains\":false,\"NoDNSLink\":false}"
done
ipfs config --json Gateway.PublicGateways "{$gateways}"

#!/bin/sh
set -ef

ipfs config Datastore.StorageMax "${SWING_KUBO_STORAGE_MAX:?}"
ipfs config Provide.Strategy "${SWING_KUBO_PROVIDE_STRATEGY:?}"

ipfs config --json Gateway.NoFetch true
ipfs config --json Gateway.NoDNSLink true
ipfs config --json Gateway.HTTPHeaders "{\"Content-Security-Policy\":[\"connect-src 'self' https: wss:; form-action 'self' https:\"]}"

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
for host in 127.0.0.1 ::1 "*.localhost"; do
  gateways="${gateways:+$gateways,}\"$host\":{\"Paths\":[],\"UseSubdomains\":false,\"NoDNSLink\":true}"
done
ipfs config --json Gateway.PublicGateways "{$gateways}"

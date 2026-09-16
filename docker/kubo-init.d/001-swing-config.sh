#!/bin/sh
set -e

ipfs config Datastore.StorageMax "${SWING_KUBO_STORAGE_MAX:?}"
ipfs config Provide.Strategy "${SWING_KUBO_PROVIDE_STRATEGY:?}"

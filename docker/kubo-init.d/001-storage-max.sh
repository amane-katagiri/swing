#!/bin/sh
set -e

ipfs config Datastore.StorageMax "${SWING_KUBO_STORAGE_MAX:?}"

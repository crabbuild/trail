#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
IMAGE=${RUST_IMAGE:-rust:bookworm}
TARGET_DIR=${TRAIL_DOCKER_TARGET_DIR:-/target}
CARGO_HOME_DIR=${TRAIL_DOCKER_CARGO_HOME:-/cargo-home}
CARGO_CACHE_VOLUME=${TRAIL_DOCKER_CARGO_CACHE_VOLUME:-trail-fuse-cow-cargo}
TARGET_CACHE_VOLUME=${TRAIL_DOCKER_TARGET_CACHE_VOLUME:-trail-fuse-cow-target}
NPM_CACHE_VOLUME=${TRAIL_DOCKER_NPM_CACHE_VOLUME:-trail-fuse-cow-npm}

docker run --rm --privileged \
  -v "$ROOT":/work \
  -v "$CARGO_CACHE_VOLUME":"$CARGO_HOME_DIR" \
  -v "$TARGET_CACHE_VOLUME":"$TARGET_DIR" \
  -v "$NPM_CACHE_VOLUME":/npm-cache \
  -w /work \
  -e CARGO_TARGET_DIR="$TARGET_DIR" \
  -e CARGO_HOME="$CARGO_HOME_DIR" \
  -e npm_config_cache=/npm-cache \
  "$IMAGE" \
  bash -lc '
set -euo pipefail
export PATH=/usr/local/cargo/bin:/usr/bin:$PATH
apt-get update -qq
apt-get install -y -qq --no-install-recommends nodejs npm ca-certificates >/dev/null
node --version
npm --version
test -e /dev/fuse
TRAIL_RUN_FUSE_NODE_LAYER_TESTS=1 \
  cargo test -p trail \
    fuse_large_root_shares_real_node_layer_but_isolates_install_and_clean \
    -- --nocapture
'

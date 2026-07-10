#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
IMAGE=${RUST_IMAGE:-rust:bookworm}
TARGET_DIR=${TRAIL_DOCKER_TARGET_DIR:-/target}
CARGO_HOME_DIR=${TRAIL_DOCKER_CARGO_HOME:-/cargo-home}
CARGO_CACHE_VOLUME=${TRAIL_DOCKER_CARGO_CACHE_VOLUME:-trail-overlay-cow-cargo}
TARGET_CACHE_VOLUME=${TRAIL_DOCKER_TARGET_CACHE_VOLUME:-trail-overlay-cow-target}

docker run --rm --privileged \
  -v "$ROOT":/work \
  -v "$CARGO_CACHE_VOLUME":"$CARGO_HOME_DIR" \
  -v "$TARGET_CACHE_VOLUME":"$TARGET_DIR" \
  -w /work \
  -e CARGO_TARGET_DIR="$TARGET_DIR" \
  -e CARGO_HOME="$CARGO_HOME_DIR" \
  "$IMAGE" \
  bash -lc '
set -euo pipefail
export PATH=/usr/local/cargo/bin:/usr/bin:$PATH
sccache_version=0.16.0
curl -fsSL \
  "https://github.com/mozilla/sccache/releases/download/v${sccache_version}/sccache-v${sccache_version}-x86_64-unknown-linux-musl.tar.gz" \
  | tar -xz -C /tmp
install "/tmp/sccache-v${sccache_version}-x86_64-unknown-linux-musl/sccache" /usr/local/bin/sccache
sccache --version
TRAIL_RUN_FUSE_COW_TESTS=1 \
  cargo test -p trail \
    cargo_target_seed_reuses_compiler_results_with_private_writable_targets \
    -- --nocapture
printf "shared-compiler-results=true private-target-clean=true\n"
'

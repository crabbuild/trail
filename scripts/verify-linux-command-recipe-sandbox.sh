#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

docker run --rm --privileged \
  -v "${repo_root}:/work" \
  -v trail-fuse-cow-cargo:/cargo-home \
  -v trail-fuse-cow-target:/target \
  -w /work \
  -e CARGO_HOME=/cargo-home \
  -e CARGO_TARGET_DIR=/target \
  rust:bookworm \
  bash -lc 'export PATH=/usr/local/cargo/bin:/usr/bin:$PATH; scripts/verify-linux-command-recipe-native.sh'

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
peer_manifest="$repo_root/tools/acp-v1-reference-peer/Cargo.toml"
peer_target="$repo_root/target/acp-reference"

cargo build --manifest-path "$peer_manifest" --target-dir "$peer_target"
peer="$peer_target/debug/acp-v1-reference-peer"
if [[ "${OS:-}" == "Windows_NT" ]]; then
  peer="$peer.exe"
fi

trail_target="${CARGO_TARGET_DIR:-$repo_root/target}"
cargo build -p trail --bin trail
export TRAIL_TEST_BIN="$trail_target/debug/trail"
export TRAIL_ACP_REFERENCE_PEER="$peer"
cargo test -p trail --test acp_interop -- --nocapture

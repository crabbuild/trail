#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
: "${CARGO_TARGET_DIR:?set a unique CARGO_TARGET_DIR beneath /Volumes/Workspace/crabbuild-target}"
: "${TRAIL_SCALE_EVIDENCE_DIR:?set TRAIL_SCALE_EVIDENCE_DIR beneath /Volumes/Workspace}"
case "$CARGO_TARGET_DIR" in
  /Volumes/Workspace/crabbuild-target/*) ;;
  *) echo "CARGO_TARGET_DIR must be beneath /Volumes/Workspace/crabbuild-target" >&2; exit 2 ;;
esac
case "$TRAIL_SCALE_EVIDENCE_DIR" in
  /Volumes/Workspace/*) ;;
  *) echo "TRAIL_SCALE_EVIDENCE_DIR must be beneath /Volumes/Workspace" >&2; exit 2 ;;
esac
mkdir -p "$TRAIL_SCALE_EVIDENCE_DIR"

for experiment in "10000 1" "100000 5" "1000000 20"; do
  read -r paths lanes <<<"$experiment"
  TRAIL_RUN_MILLION_PATH_VIEW_TEST=1 \
  TRAIL_SCALE_PATHS="$paths" \
  TRAIL_SCALE_LANES="$lanes" \
    cargo test -p trail --lib large_path_multi_view_scale_acceptance --locked -- --nocapture
done

# Complement the path/lane matrix with the correctness experiments whose
# runtime is independent of synthetic path count. These cover reusable
# parent/child layers, private-output promotion, interrupted publication,
# pressure GC, and owning-host remount/isolation semantics.
cargo test -p trail --test lane_environment_inheritance lane_fork_inherits_verified_immutable_layer_with_fresh_private_uppers --locked
cargo test -p trail --lib manual_private_output_promotion_is_journaled_and_preserves_private_bytes --locked
cargo test -p trail --lib killing_cache_publish_at_each_durable_phase_preserves_source_and_recovers --locked
cargo test -p trail --lib cache_gc_never_selects_pinned_layers_and_reclaims_unpinned_layers --locked

case "$(uname -s)" in
  Darwin)
    cargo test -p trail --lib nfs_adapter_runs_shared_mounted_view_suite --locked
    ;;
  Linux)
    cargo test -p trail --lib fuse_adapter_runs_shared_mounted_view_suite --locked
    ;;
  MINGW*|MSYS*|CYGWIN*)
    cargo test -p trail --lib dokan_adapter_runs_shared_mounted_view_suite --locked
    ;;
esac

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  : "${RUNNER_TEMP:?GitHub Actions did not provide RUNNER_TEMP}"
  CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${RUNNER_TEMP}/trail-artifact-real-tool-target}"
  export CARGO_TARGET_DIR
else
  : "${CARGO_TARGET_DIR:?set a checkout-specific CARGO_TARGET_DIR beneath /Volumes/Workspace/crabbuild-target}"
  if [[ "${CARGO_TARGET_DIR}" != /Volumes/Workspace/crabbuild-target/* ]]; then
    printf '%s\n' 'CARGO_TARGET_DIR must be beneath /Volumes/Workspace/crabbuild-target' >&2
    exit 2
  fi
fi

for tool in cargo git node npm cmake python3; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    printf 'required real-tool gate dependency is unavailable: %s\n' "${tool}" >&2
    exit 2
  fi
done

case "$(uname -s)" in
  Linux)
    if [[ "${TRAIL_RUN_FUSE_COW_TESTS:-}" != "1" ]]; then
      printf '%s\n' 'real path-bound gates require TRAIL_RUN_FUSE_COW_TESTS=1 on Linux' >&2
      exit 2
    fi
    ;;
  Darwin)
    if [[ "${TRAIL_RUN_NFS_COW_TESTS:-}" != "1" ]]; then
      printf '%s\n' 'real path-bound gates require TRAIL_RUN_NFS_COW_TESTS=1 on macOS' >&2
      exit 2
    fi
    ;;
  *)
    printf '%s\n' 'portable artifact real-tool gate currently requires Linux or macOS' >&2
    exit 2
    ;;
esac

tests=(
  db::lane::workspace_environment::tests::host_resolver_executes_cargo_in_isolated_staging_and_reuses_snapshot
  db::lane::workspace_node::tests::manifest_only_npm_uses_managed_lock_and_preserves_seed_cache_isolation
  db::lane::workspace_cargo::tests::cargo_adapter_builds_once_and_reuses_one_immutable_target_seed
  db::lane::workspace_python::tests::real_python_venvs_embed_lane_paths_and_remain_isolated
  db::lane::workspace_cmake::tests::real_cmake_configure_build_and_clean_stay_lane_private
  db::lane::workspace_plugin::tests::protocol_v2_bazel_nix_like_stores_remain_metadata_only_after_host_normalization
  db::lane::workspace_recipe::tests::maven_gradle_like_and_unknown_custom_shapes_use_repository_v2_components
  db::lane::source_export::tests::source_export_execution_checkpoints_normal_source_and_reports_git_handoff
)

for test_name in "${tests[@]}"; do
  cargo test -p trail --lib "${test_name}" --locked -- --exact --nocapture
done

cargo test -p trail --test e2e \
  next_and_vite_v2_components_compose_through_native_cli_sandbox \
  --locked -- --exact --nocapture

printf '%s\n' 'artifact real-tool gates: passed'

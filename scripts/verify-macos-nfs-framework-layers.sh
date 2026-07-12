#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS is required for the native nfs-cow framework benchmark" >&2
  exit 2
fi

export TRAIL_RUN_NFS_FRAMEWORK_BENCH=1
if [[ $# -gt 0 ]]; then
  export TRAIL_NFS_FRAMEWORK_FILTER="$1"
fi

cargo test -p trail nfs_large_nextjs_and_vite_layers_build_and_bulk_replace -- --nocapture

#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
TRAIL_RUN_MILLION_PATH_VIEW_TEST=1 \
  cargo test -p trail million_path_twenty_view_scale_acceptance -- --nocapture

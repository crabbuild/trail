#!/usr/bin/env bash
set -euo pipefail

# Release-quality terminal UX baseline. The Rust test covers renderer hot paths;
# this small probe covers binary startup without adding a benchmark dependency.
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

bin="${TRAIL_BIN:-target/release/trail}"
if [[ ! -x "$bin" ]]; then
  cargo build --release -p trail
fi

python3 - "$bin" <<'PY'
import statistics
import subprocess
import sys
import time

binary = sys.argv[1]
samples = []
for _ in range(5):
    start = time.perf_counter()
    subprocess.run([binary, "--help"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    samples.append((time.perf_counter() - start) * 1000)
median = statistics.median(samples)
print(f"terminal startup baseline: median={median:.1f}ms samples={','.join(f'{value:.1f}' for value in samples)}")
if median > 250:
    raise SystemExit(f"startup regression: {median:.1f}ms exceeds 250ms")
PY

cargo test --release -p trail terminal_render_baseline -- --ignored --nocapture

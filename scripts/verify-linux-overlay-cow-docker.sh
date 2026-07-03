#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
IMAGE=${RUST_IMAGE:-rust:bookworm}
TARGET_DIR=${CRABDB_DOCKER_TARGET_DIR:-/target}
CARGO_HOME_DIR=${CRABDB_DOCKER_CARGO_HOME:-/cargo-home}
CARGO_CACHE_VOLUME=${CRABDB_DOCKER_CARGO_CACHE_VOLUME:-crabdb-overlay-cow-cargo}
TARGET_CACHE_VOLUME=${CRABDB_DOCKER_TARGET_CACHE_VOLUME:-crabdb-overlay-cow-target}

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
export PATH=/usr/local/cargo/bin:$PATH

rustc --version
test -e /dev/fuse

cargo build -p crabdb

tmp=$(mktemp -d /tmp/crabdb-linux-overlay.XXXXXX)
printf "hello\n" > "$tmp/README.md"
mkdir -p "$tmp/src"
printf "pub fn answer() -> u8 { 42 }\n" > "$tmp/src/lib.rs"

"$CARGO_TARGET_DIR/debug/crabdb" --workspace "$tmp" init --working-tree >/tmp/crabdb-overlay-init.out
"$CARGO_TARGET_DIR/debug/crabdb" --workspace "$tmp" agent start \
  --provider custom \
  --workdir-mode overlay-cow \
  -- bash -lc '"'"'
set -euo pipefail
test -f README.md
test -f src/lib.rs
test ! -e .crabdb
test "$(cat README.md)" = "hello"
echo "agent-fs-type=$(stat -f -c %T .)"
printf "changed\n" >> README.md
mkdir notes
printf "new\n" > notes/todo.txt
rm src/lib.rs
'"'"' >/tmp/crabdb-overlay-agent.out

"$CARGO_TARGET_DIR/debug/crabdb" --workspace "$tmp" agent changes latest --json >/tmp/crabdb-overlay-changes.json
cat /tmp/crabdb-overlay-agent.out
python3 - <<'"'"'PY'"'"'
import json
from pathlib import Path

data = json.loads(Path("/tmp/crabdb-overlay-changes.json").read_text())
paths = sorted(item["path"] for item in data["total_changed_paths"])
print("changed-paths=" + ",".join(paths))
assert paths == ["README.md", "notes/todo.txt", "src/lib.rs"], paths

workdir = Path(data["task"]["workdir"])
assert workdir.is_dir(), workdir
assert not any(workdir.iterdir()), "overlay mountpoint should be empty after unmount"

upper = Path(data["task"]["workdir"]).parents[1] / "overlay-cow" / data["lane"] / "upper"
assert (upper / "README.md").is_file(), upper
assert (upper / "notes" / "todo.txt").is_file(), upper
assert (upper / ".crabdb" / "overlay-whiteouts.json").is_file(), upper
PY
'

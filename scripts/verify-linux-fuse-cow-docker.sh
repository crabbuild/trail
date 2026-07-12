#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
IMAGE=${RUST_IMAGE:-rust:bookworm}
TARGET_DIR=${TRAIL_DOCKER_TARGET_DIR:-/target}
CARGO_HOME_DIR=${TRAIL_DOCKER_CARGO_HOME:-/cargo-home}
CARGO_CACHE_VOLUME=${TRAIL_DOCKER_CARGO_CACHE_VOLUME:-trail-fuse-cow-cargo}
TARGET_CACHE_VOLUME=${TRAIL_DOCKER_TARGET_CACHE_VOLUME:-trail-fuse-cow-target}

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

TRAIL_RUN_FUSE_COW_TESTS=1 cargo test -p trail \
  fuse_adapter_runs_shared_mounted_view_suite -- --nocapture
cargo build -p trail

tmp=$(mktemp -d /tmp/trail-linux-fuse-cow.XXXXXX)
printf "hello\n" > "$tmp/README.md"
mkdir -p "$tmp/src"
printf "pub fn answer() -> u8 { 42 }\n" > "$tmp/src/lib.rs"

"$CARGO_TARGET_DIR/debug/trail" --workspace "$tmp" init --working-tree >/tmp/trail-fuse-init.out
"$CARGO_TARGET_DIR/debug/trail" --workspace "$tmp" agent start \
  --provider custom \
  --workdir-mode fuse-cow \
  -- bash -lc '"'"'
set -euo pipefail
test -f README.md
test -f src/lib.rs
test ! -e .trail
test "$(cat README.md)" = "hello"
echo "agent-fs-type=$(stat -f -c %T .)"
printf "changed\n" >> README.md
mkdir notes
printf "new\n" > notes/todo.txt
rm src/lib.rs
'"'"' >/tmp/trail-fuse-agent.out

"$CARGO_TARGET_DIR/debug/trail" --workspace "$tmp" agent changes latest --json >/tmp/trail-fuse-changes.json
cat /tmp/trail-fuse-agent.out
python3 - <<'"'"'PY'"'"'
import json
from pathlib import Path

data = json.loads(Path("/tmp/trail-fuse-changes.json").read_text())
paths = sorted(item["path"] for item in data["total_changed_paths"])
print("changed-paths=" + ",".join(paths))
assert paths == ["README.md", "notes/todo.txt", "src/lib.rs"], paths

workdir = Path(data["task"]["workdir"])
assert workdir.is_dir(), workdir
assert not any(workdir.iterdir()), "FUSE COW mountpoint should be empty after unmount"

db_dir = workdir.parents[1]
views = sorted((db_dir / "views").iterdir())
assert len(views) == 1, views
view = views[0]
source_upper = view / "source-upper"
assert (source_upper / "README.md").is_file(), source_upper
assert (source_upper / "notes" / "todo.txt").is_file(), source_upper
assert (view / "meta" / "source-whiteouts.json").is_file(), view
PY

"$CARGO_TARGET_DIR/debug/trail" --workspace "$tmp" lane spawn persistent \
  --from main --workdir-mode fuse-cow >/tmp/trail-fuse-spawn.out
workdir=$("$CARGO_TARGET_DIR/debug/trail" --workspace "$tmp" lane workdir persistent --json | \
  python3 -c '"'"'import json,sys; print(json.load(sys.stdin)["workdir"])'"'"')
"$CARGO_TARGET_DIR/debug/trail" --workspace "$tmp" lane mount persistent \
  >/tmp/trail-fuse-mount.out 2>&1 &
mount_pid=$!
for _ in $(seq 1 100); do
  test -f "$workdir/README.md" && break
  sleep 0.1
done
test -f "$workdir/README.md"
printf "persistent\n" > "$workdir/persistent.txt"
"$CARGO_TARGET_DIR/debug/trail" --workspace "$tmp" lane unmount persistent \
  >/tmp/trail-fuse-unmount.out
wait "$mount_pid"
test ! -e "$workdir/README.md"
"$CARGO_TARGET_DIR/debug/trail" --workspace "$tmp" lane checkpoint persistent \
  -m "persistent lifecycle" --json >/tmp/trail-fuse-checkpoint.json
python3 - <<'"'"'PY'"'"'
import json
from pathlib import Path

checkpoint = json.loads(Path("/tmp/trail-fuse-checkpoint.json").read_text())
assert checkpoint["source_paths"] == ["persistent.txt"], checkpoint
print("persistent-mount-checkpoint=" + checkpoint["root_id"])
PY
'

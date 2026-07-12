#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OPENCLAW_PATH=${OPENCLAW_PATH:-/Users/haipingfu/Github/openclaw}
IMAGE=${RUST_IMAGE:-rust:bookworm}
TARGET_DIR=${TRAIL_DOCKER_TARGET_DIR:-/target}
CARGO_HOME_DIR=${TRAIL_DOCKER_CARGO_HOME:-/cargo-home}
CARGO_CACHE_VOLUME=${TRAIL_DOCKER_CARGO_CACHE_VOLUME:-trail-fuse-cow-cargo}
TARGET_CACHE_VOLUME=${TRAIL_DOCKER_TARGET_CACHE_VOLUME:-trail-fuse-cow-target}
LINUX_CACHE_VOLUME=${TRAIL_LINUX_CACHE_VOLUME:-trail-linux-source-cache}
LINUX_REPO_URL=${LINUX_REPO_URL:-https://github.com/torvalds/linux.git}
LINUX_REF=${LINUX_REF:-}
RUN_OPENCLAW=${TRAIL_PERF_RUN_OPENCLAW:-1}
RUN_LINUX=${TRAIL_PERF_RUN_LINUX:-1}
KEEP_BENCH=${TRAIL_PERF_KEEP_BENCH:-0}
HOST_BENCH_ROOT=${TRAIL_PERF_HOST_BENCH_ROOT:-}

if [[ "$RUN_OPENCLAW" == "1" && ! -d "$OPENCLAW_PATH/.git" ]]; then
  echo "OpenClaw Git checkout not found at $OPENCLAW_PATH" >&2
  exit 1
fi

if [[ -n "$HOST_BENCH_ROOT" ]]; then
  mkdir -p "$HOST_BENCH_ROOT"
  KEEP_BENCH=1
fi

run_docker() {
  if [[ -n "$HOST_BENCH_ROOT" ]]; then
    docker run --rm -i --privileged \
      -v "$ROOT":/work \
      -v "$OPENCLAW_PATH":/host-openclaw:ro \
      -v "$CARGO_CACHE_VOLUME":"$CARGO_HOME_DIR" \
      -v "$TARGET_CACHE_VOLUME":"$TARGET_DIR" \
      -v "$LINUX_CACHE_VOLUME":/linux-cache \
      -v "$HOST_BENCH_ROOT":/bench-host \
      -w /work \
      -e CARGO_TARGET_DIR="$TARGET_DIR" \
      -e CARGO_HOME="$CARGO_HOME_DIR" \
      -e LINUX_REPO_URL="$LINUX_REPO_URL" \
      -e LINUX_REF="$LINUX_REF" \
      -e RUN_OPENCLAW="$RUN_OPENCLAW" \
      -e RUN_LINUX="$RUN_LINUX" \
      -e KEEP_BENCH="$KEEP_BENCH" \
      -e BENCH_ROOT=/bench-host \
      "$IMAGE" \
      bash -s
  else
    docker run --rm -i --privileged \
      -v "$ROOT":/work \
      -v "$OPENCLAW_PATH":/host-openclaw:ro \
      -v "$CARGO_CACHE_VOLUME":"$CARGO_HOME_DIR" \
      -v "$TARGET_CACHE_VOLUME":"$TARGET_DIR" \
      -v "$LINUX_CACHE_VOLUME":/linux-cache \
      -w /work \
      -e CARGO_TARGET_DIR="$TARGET_DIR" \
      -e CARGO_HOME="$CARGO_HOME_DIR" \
      -e LINUX_REPO_URL="$LINUX_REPO_URL" \
      -e LINUX_REF="$LINUX_REF" \
      -e RUN_OPENCLAW="$RUN_OPENCLAW" \
      -e RUN_LINUX="$RUN_LINUX" \
      -e KEEP_BENCH="$KEEP_BENCH" \
      "$IMAGE" \
      bash -s
  fi
}

run_docker <<'DOCKER_BASH'
set -euo pipefail
export PATH=/usr/local/cargo/bin:$PATH

ms() {
  date +%s%3N
}

run_timed() {
  local __result_var=$1
  shift
  local start end
  start=$(ms)
  "$@"
  end=$(ms)
  printf -v "$__result_var" '%s' "$((end - start))"
}

du_bytes() {
  local path=$1
  if [[ -e "$path" ]]; then
    du -sb "$path" | awk '{print $1}'
  else
    printf '0\n'
  fi
}

choose_edit_file() {
  local repo=$1
  local candidate mode
  for candidate in README.md README README.rst README.txt MAINTAINERS package.json Cargo.toml; do
    if git -C "$repo" ls-files --error-unmatch "$candidate" >/dev/null 2>&1; then
      mode=$(git -C "$repo" ls-files -s -- "$candidate" | awk '{print $1}')
      if [[ "$mode" == "100644" || "$mode" == "100755" ]]; then
        printf '%s\n' "$candidate"
        return 0
      fi
    fi
  done
  while IFS= read -r candidate; do
    mode=$(git -C "$repo" ls-files -s -- "$candidate" | awk '{print $1}')
    if [[ "$mode" == "100644" || "$mode" == "100755" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done < <(git -C "$repo" ls-files | grep -E '(\.md|\.txt|\.rs|\.c|\.h|\.ts|\.js|\.json|\.toml)$' || true)
  return 1
}

append_result_json() {
  python3 - "$RESULTS" "$@" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
pairs = sys.argv[2:]
item = {}
for pair in pairs:
    key, raw = pair.split("=", 1)
    if raw.isdigit():
        item[key] = int(raw)
    elif raw in {"true", "false"}:
        item[key] = raw == "true"
    else:
        item[key] = raw
with path.open("a", encoding="utf-8") as f:
    f.write(json.dumps(item, sort_keys=True) + "\n")
PY
}

print_result_table() {
  python3 - "$RESULTS" <<'PY'
import json
import sys
from pathlib import Path

rows = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines() if line.strip()]
if not rows:
    raise SystemExit

headers = [
    "repo",
    "files",
    "clone_ms",
    "init_ms",
    "agent_ms",
    "dry_run_ms",
    "land_ms",
    "db_mb",
    "worktrees_kb",
    "fuse_view_kb",
    "commit",
]
print("\t".join(headers))
for row in rows:
    print("\t".join(
        str(value)
        for value in [
            row["repo"],
            row["tracked_files"],
            row["clone_ms"],
            row["init_ms"],
            row["agent_ms"],
            row["land_dry_run_ms"],
            row["land_ms"],
            round(row["trail_bytes"] / 1024 / 1024, 1),
            round(row["worktrees_bytes"] / 1024, 1),
            round(row["fuse_bytes"] / 1024, 1),
            row["head_after"][:12],
        ]
    ))
PY
}

verify_tools() {
  rustc --version
  cargo --version
  git --version
  python3 --version
  test -e /dev/fuse
}

configure_git() {
  git config --global user.name "Trail Perf"
  git config --global user.email "trail-perf@example.invalid"
  git config --global protocol.file.allow always
  git config --global --add safe.directory /host-openclaw || true
}

clone_openclaw() {
  local dest=$1
  rm -rf "$dest"
  git clone --shared /host-openclaw "$dest"
}

prepare_linux_cache() {
  if [[ "${TRAIL_REFRESH_LINUX:-0}" == "1" ]]; then
    rm -rf /linux-cache/linux.git
  fi
  if [[ ! -d /linux-cache/linux.git ]]; then
    rm -rf /linux-cache/linux.git.tmp
    git clone --bare --depth=1 "$LINUX_REPO_URL" /linux-cache/linux.git.tmp
    mv /linux-cache/linux.git.tmp /linux-cache/linux.git
  fi
}

clone_linux() {
  local dest=$1
  rm -rf "$dest"
  prepare_linux_cache
  git clone --shared /linux-cache/linux.git "$dest"
  if [[ -n "$LINUX_REF" ]]; then
    git -C "$dest" fetch --depth=1 origin "$LINUX_REF"
    git -C "$dest" checkout --detach FETCH_HEAD
  fi
}

benchmark_repo() {
  local name=$1
  local clone_kind=$2
  local repo="$BENCH_ROOT/$name"
  local clone_ms init_ms agent_ms changes_ms land_dry_run_ms land_ms
  local tracked_files head_before head_after edit_file new_file marker
  local agent_out agent_err changes_json land_json status_tracked
  local trail_bytes worktrees_bytes fuse_bytes

  echo "== $name: clone =="
  if [[ "$clone_kind" == "openclaw" ]]; then
    run_timed clone_ms clone_openclaw "$repo"
  elif [[ "$clone_kind" == "linux" ]]; then
    run_timed clone_ms clone_linux "$repo"
  else
    echo "unknown clone kind: $clone_kind" >&2
    exit 1
  fi
  git -C "$repo" config user.name "Trail Perf"
  git -C "$repo" config user.email "trail-perf@example.invalid"
  git -C "$repo" reset --hard HEAD >/dev/null
  git -C "$repo" clean -fdx >/dev/null

  tracked_files=$(git -C "$repo" ls-files | wc -l | awk '{print $1}')
  head_before=$(git -C "$repo" rev-parse HEAD)
  edit_file=$(choose_edit_file "$repo")
  new_file="trail-perf/${name}-fuse.txt"
  marker="trail-fuse-perf-${name}-$(date +%s)-$$"
  agent_out="$BENCH_ROOT/${name}.agent.out"
  agent_err="$BENCH_ROOT/${name}.agent.err"
  changes_json="$BENCH_ROOT/${name}.changes.json"
  land_json="$BENCH_ROOT/${name}.land.json"

  echo "== $name: trail init --from-git ($tracked_files files) =="
  run_timed init_ms "$TRAIL" --workspace "$repo" init --from-git --force
  "$TRAIL" --workspace "$repo" config set recording.ignore_gitignored false >/dev/null
  status_tracked=$(git -C "$repo" status --short --untracked-files=no)
  if [[ -n "$status_tracked" ]]; then
    echo "tracked Git status changed after init:" >&2
    printf '%s\n' "$status_tracked" >&2
    exit 1
  fi

  echo "== $name: FUSE COW agent edit =="
  export PERF_EDIT_FILE="$edit_file"
  export PERF_NEW_FILE="$new_file"
  export PERF_MARKER="$marker"
  run_timed agent_ms "$TRAIL" --workspace "$repo" agent start \
    --provider custom \
    --workdir-mode fuse-cow \
    -- bash -lc '
set -euo pipefail
test -f "$PERF_EDIT_FILE"
printf "\n%s\n" "$PERF_MARKER" >> "$PERF_EDIT_FILE"
mkdir -p "$(dirname "$PERF_NEW_FILE")"
printf "%s\n" "$PERF_MARKER" > "$PERF_NEW_FILE"
echo "agent-fs-type=$(stat -f -c %T .)"
grep -F "$PERF_MARKER" "$PERF_EDIT_FILE" >/dev/null
test -f "$PERF_NEW_FILE"
' >"$agent_out" 2>"$agent_err"
  grep -F "agent-fs-type=fuseblk" "$agent_out" >/dev/null

  run_timed changes_ms "$TRAIL" --workspace "$repo" agent changes latest --json >"$changes_json"
  python3 - "$changes_json" "$edit_file" "$new_file" <<'PY'
import json
import sys
from pathlib import Path

data = json.loads(Path(sys.argv[1]).read_text())
paths = sorted(item["path"] for item in data["total_changed_paths"])
expected = sorted([sys.argv[2], sys.argv[3]])
if paths != expected:
    raise SystemExit(f"unexpected changed paths: {paths!r}, expected {expected!r}")
workdir = Path(data["task"]["workdir"])
if not workdir.is_dir():
    raise SystemExit(f"missing lane workdir mountpoint: {workdir}")
if any(workdir.iterdir()):
    raise SystemExit(f"FUSE COW mountpoint should be empty after unmount: {workdir}")
PY

  echo "== $name: agent land dry-run =="
  run_timed land_dry_run_ms "$TRAIL" --workspace "$repo" agent land latest \
    --dry-run \
    --json >"$BENCH_ROOT/${name}.land-dry-run.json"
  python3 - "$BENCH_ROOT/${name}.land-dry-run.json" "$edit_file" "$new_file" <<'PY'
import json
import sys
from pathlib import Path

data = json.loads(Path(sys.argv[1]).read_text())
paths = sorted(item["path"] for item in (data.get("merge") or {}).get("changed_paths") or [])
expected = sorted([sys.argv[2], sys.argv[3]])
if paths != expected:
    raise SystemExit(f"unexpected dry-run merge paths: {paths!r}, expected {expected!r}")
PY

  echo "== $name: agent land =="
  run_timed land_ms "$TRAIL" --workspace "$repo" agent land latest \
    -m "Trail FUSE COW perf test for $name" \
    --json >"$land_json"
  head_after=$(git -C "$repo" rev-parse HEAD)
  if [[ "$head_before" == "$head_after" ]]; then
    echo "$name: Git HEAD did not advance after agent land" >&2
    exit 1
  fi
  status_tracked=$(git -C "$repo" status --short --untracked-files=no)
  if [[ -n "$status_tracked" ]]; then
    echo "$name: tracked Git status is dirty after land:" >&2
    printf '%s\n' "$status_tracked" >&2
    exit 1
  fi
  grep -F "$marker" "$repo/$edit_file" >/dev/null
  grep -F "$marker" "$repo/$new_file" >/dev/null
  git -C "$repo" show --name-only --format='landed-commit=%H%nsubject=%s' HEAD | sed "s/^/$name: /"

  trail_bytes=$(du_bytes "$repo/.trail")
  worktrees_bytes=$(du_bytes "$repo/.trail/worktrees")
  fuse_bytes=$(du_bytes "$repo/.trail/views")
  append_result_json \
    "repo=$name" \
    "tracked_files=$tracked_files" \
    "clone_ms=$clone_ms" \
    "init_ms=$init_ms" \
    "agent_ms=$agent_ms" \
    "changes_ms=$changes_ms" \
    "land_dry_run_ms=$land_dry_run_ms" \
    "land_ms=$land_ms" \
    "trail_bytes=$trail_bytes" \
    "worktrees_bytes=$worktrees_bytes" \
    "fuse_bytes=$fuse_bytes" \
    "head_before=$head_before" \
    "head_after=$head_after" \
    "edit_file=$edit_file" \
    "new_file=$new_file"
}

verify_tools
configure_git
cargo build -p trail
TRAIL="$CARGO_TARGET_DIR/debug/trail"

BENCH_ROOT=${BENCH_ROOT:-$(mktemp -d /tmp/trail-fuse-perf.XXXXXX)}
mkdir -p "$BENCH_ROOT"
RESULTS="$BENCH_ROOT/results.jsonl"
echo "bench-root=$BENCH_ROOT"

cleanup() {
  if [[ "${KEEP_BENCH:-0}" != "1" ]]; then
    rm -rf "$BENCH_ROOT"
  fi
}
trap cleanup EXIT

if [[ "${RUN_OPENCLAW:-1}" == "1" ]]; then
  benchmark_repo openclaw openclaw
fi

if [[ "${RUN_LINUX:-1}" == "1" ]]; then
  benchmark_repo linux-kernel linux
fi

echo "== summary =="
print_result_table
echo "results-jsonl=$RESULTS"
if [[ "${KEEP_BENCH:-0}" == "1" ]]; then
  echo "kept-bench-root=$BENCH_ROOT"
fi
DOCKER_BASH

#!/usr/bin/env bash
set -euo pipefail

BIN="${CRABDB_BIN:-}"
SCALES="${CRABDB_SCALE_FILES:-10000}"
BASE_DIR="${CRABDB_SCALE_BASE:-/Volumes/Workspace}"
RUN_LABEL="${CRABDB_SCALE_LABEL:-$(date +%Y%m%d-%H%M%S)}"
RUN_MATERIALIZED="${CRABDB_SCALE_MATERIALIZED:-1}"
RUN_BACKUP="${CRABDB_SCALE_BACKUP:-1}"
RUN_DAEMON="${CRABDB_SCALE_DAEMON:-1}"

if [ -z "$BIN" ]; then
  cargo build -p crabdb --release >/dev/null
  BIN="$(pwd)/target/release/crabdb"
fi

if [ ! -x "$BIN" ]; then
  printf 'crabdb binary is not executable: %s\n' "$BIN" >&2
  exit 2
fi

RUN_ROOT="$BASE_DIR/crabdb-cli-scale-$RUN_LABEL"
mkdir -p "$RUN_ROOT"

run_timed() {
  local scale="$1"
  local name="$2"
  shift 2
  local out_dir="$RUN_ROOT/$scale/out"
  local results="$RUN_ROOT/$scale/results.tsv"
  mkdir -p "$out_dir"
  local stdout="$out_dir/$name.stdout"
  local stderr="$out_dir/$name.stderr"
  set +e
  if /usr/bin/time -lp true >/dev/null 2>"$out_dir/.time-probe" ; then
    /usr/bin/time -lp "$@" >"$stdout" 2>"$stderr"
  else
    /usr/bin/time -f 'real %e\nmaximum resident set size kb %M' "$@" >"$stdout" 2>"$stderr"
  fi
  local code=$?
  set -e
  local real rss
  real="$(awk '/^real / {v=$2} END {print v+0}' "$stderr")"
  rss="$(awk '
    /maximum resident set size kb/ {v=$NF * 1024}
    /maximum resident set size/ && $0 !~ / kb / {
      for (i=1;i<=NF;i++) if ($i ~ /^[0-9]+$/) v=$i
    }
    END {print v+0}
  ' "$stderr")"
  printf '%s\t%s\t%s\t%s\n' "$name" "$real" "$rss" "$code" >> "$results"
  printf 'scale=%s %-36s %8ss rss=%s exit=%s\n' "$scale" "$name" "$real" "$rss" "$code"
  if [ "$code" -ne 0 ]; then
    tail -80 "$stderr" >&2 || true
    tail -80 "$stdout" >&2 || true
    exit "$code"
  fi
}

repo_source_bytes() {
  python3 - "$1" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
total = 0
count = 0
for path in root.rglob("*"):
    if ".crabdb" in path.parts:
        continue
    if path.is_file():
        total += path.stat().st_size
        count += 1
print(f"{count}\t{total}")
PY
}

sqlite_bytes() {
  local db="$1/.crabdb/index/crabdb.sqlite"
  if [ -f "$db" ]; then
    stat -f '%z' "$db" 2>/dev/null || stat -c '%s' "$db"
  else
    printf '0'
  fi
}

object_count() {
  local db="$1/.crabdb/index/crabdb.sqlite"
  if [ -f "$db" ]; then
    sqlite3 "$db" 'SELECT COUNT(*) FROM objects;' 2>/dev/null || printf '0'
  else
    printf '0'
  fi
}

free_loopback_port() {
  python3 - <<'PY'
import socket
sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

wait_for_daemon() {
  local url="$1"
  python3 - "$url" <<'PY'
import sys, time, urllib.request
url = sys.argv[1]
deadline = time.time() + 180
while time.time() < deadline:
    try:
        with urllib.request.urlopen(url + "/v1/health", timeout=0.5) as response:
            if response.status == 200:
                sys.exit(0)
    except Exception:
        time.sleep(0.1)
print("daemon did not become ready", file=sys.stderr)
sys.exit(1)
PY
}

wait_for_daemon_hot_cache() {
  local endpoint="$1"
  local url="$2"
  python3 - "$endpoint" "$url" <<'PY'
import json, pathlib, sys, time
endpoint = pathlib.Path(sys.argv[1])
url = sys.argv[2]
deadline = time.time() + 300
while time.time() < deadline:
    try:
        payload = json.loads(endpoint.read_text())
    except Exception:
        time.sleep(0.1)
        continue
    if payload.get("url") == url:
        sys.exit(0)
    time.sleep(0.1)
print("daemon hot cache did not become ready", file=sys.stderr)
sys.exit(1)
PY
}

run_http_timed() {
  local scale="$1"
  local name="$2"
  local base_url="$3"
  local method="$4"
  local path="$5"
  local body="${6:-}"
  run_timed "$scale" "$name" python3 - "$base_url" "$method" "$path" "$body" <<'PY'
import pathlib, sys, urllib.request
base_url, method, path, body_path = sys.argv[1:5]
data = None
headers = {}
if body_path:
    data = pathlib.Path(body_path).read_bytes()
    headers["content-type"] = "application/json"
request = urllib.request.Request(base_url + path, data=data, method=method, headers=headers)
with urllib.request.urlopen(request, timeout=30) as response:
    sys.stdout.buffer.write(response.read())
PY
}

daemon_rss_bytes() {
  local pid="$1"
  if ps -p "$pid" >/dev/null 2>&1; then
    ps -o rss= -p "$pid" | awk '{print $1 * 1024}'
  else
    printf '0'
  fi
}

for scale in ${SCALES//,/ }; do
  case "$scale" in
    ''|*[!0-9]*)
      printf 'invalid scale: %s\n' "$scale" >&2
      exit 2
      ;;
  esac

  WORK="$RUN_ROOT/$scale"
  REPO="$WORK/repo"
  RESULTS="$WORK/results.tsv"
  rm -rf "$WORK"
  mkdir -p "$REPO" "$WORK/out"
  printf 'name\treal_seconds\tmax_rss_bytes\texit_code\n' > "$RESULTS"

  run_timed "$scale" generate_repo python3 - "$REPO" "$scale" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
files = int(sys.argv[2])
per_dir = 100
dirs = (files + per_dir - 1) // per_dir
for d in range(dirs):
    pkg = root / f"pkg_{d:05d}"
    pkg.mkdir(parents=True, exist_ok=True)
    for f in range(per_dir):
        idx = d * per_dir + f
        if idx >= files:
            break
        path = pkg / f"module_{f:03d}.rs"
        path.write_text(
            f"// package {d:05d} module {f:03d}\n"
            f"pub fn value_{idx:08d}() -> usize {{\n"
            f"    {idx}\n"
            "}\n"
        )
if files > 2:
    shared = root / "shared"
    shared.mkdir(parents=True, exist_ok=True)
    (shared / "helper.rs").write_text("pub fn helper_value() -> usize { 42 }\n")
    (root / "pkg_00000" / "module_002.rs").write_text(
        "#[path = \"../shared/helper.rs\"]\n"
        "mod helper;\n"
        "pub fn value_00000002() -> usize {\n"
        "    helper::helper_value()\n"
        "}\n"
    )
(root / "README.md").write_text(f"# CrabDB CLI scale {files}\n")
(root / ".gitignore").write_text("target/\nnode_modules/\n.DS_Store\n")
PY

  run_timed "$scale" init_working_tree "$BIN" --workspace "$REPO" --json init --working-tree
  run_timed "$scale" status_clean "$BIN" --workspace "$REPO" --json status
  run_timed "$scale" doctor_clean "$BIN" --workspace "$REPO" --json doctor
  run_timed "$scale" fsck_clean "$BIN" --workspace "$REPO" --json fsck

  run_timed "$scale" mutate_worktree python3 - "$REPO" "$scale" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
files = int(sys.argv[2])
edit_count = max(1, min(100, files // 100))
for i in range(edit_count):
    idx = (i * 7919) % files
    d, f = divmod(idx, 100)
    path = root / f"pkg_{d:05d}" / f"module_{f:03d}.rs"
    with path.open("a") as fh:
        fh.write(f"\n// scale edit {i}\npub const SCALE_EDIT_{i}: usize = {i};\n")
for i in range(max(1, min(25, files // 1000))):
    d = (i * 3571) % max(1, (files + 99) // 100)
    path = root / f"pkg_{d:05d}" / f"new_scale_file_{i:03d}.rs"
    path.write_text(f"pub fn new_scale_{i}() -> usize {{ {i} }}\n")
PY

  run_timed "$scale" status_dirty "$BIN" --workspace "$REPO" --json status
  run_timed "$scale" diff_dirty "$BIN" --workspace "$REPO" --json diff --dirty
  run_timed "$scale" record_dirty "$BIN" --workspace "$REPO" --json record -m "scale dirty record"

  DAEMON_RSS=0
  if [ "$RUN_DAEMON" = "1" ]; then
    DAEMON_PORT="$(free_loopback_port)"
    DAEMON_URL="http://127.0.0.1:$DAEMON_PORT"
    "$BIN" --workspace "$REPO" --quiet daemon --host 127.0.0.1 --port "$DAEMON_PORT" --no-auth \
      >"$WORK/out/daemon.stdout" 2>"$WORK/out/daemon.stderr" &
    DAEMON_PID=$!
    trap 'kill "$DAEMON_PID" >/dev/null 2>&1 || true' EXIT
    wait_for_daemon "$DAEMON_URL"
    wait_for_daemon_hot_cache "$REPO/.crabdb/daemon.json" "$DAEMON_URL"
    run_http_timed "$scale" daemon_status "$DAEMON_URL" GET /v1/status
    python3 - "$WORK/daemon-spawn.json" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "name": "daemonbot",
    "from_ref": "main",
    "materialize": False,
}))
PY
    run_http_timed "$scale" daemon_agent_spawn "$DAEMON_URL" POST /v1/agents "$WORK/daemon-spawn.json"
    python3 - "$REPO" "$WORK/daemon-patch.json" "$scale" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
edits = []
for i in range(max(1, min(25, files // 400))):
    idx = (i * 5393 + 31) % files
    d, f = divmod(idx, 100)
    rel = pathlib.Path(f"pkg_{d:05d}") / f"module_{f:03d}.rs"
    edits.append({
        "op": "write",
        "path": str(rel),
        "content": (root / rel).read_text() + f"\n// daemonbot {i}\n",
    })
json.dump({"message": "scale daemonbot", "edits": edits}, out.open("w"))
PY
    run_http_timed "$scale" daemon_agent_patch "$DAEMON_URL" POST /v1/agents/daemonbot/patches "$WORK/daemon-patch.json"
    python3 - "$WORK/daemon-read.json" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "path": "README.md",
}))
PY
    run_http_timed "$scale" daemon_agent_read "$DAEMON_URL" POST /v1/agents/daemonbot/read-file "$WORK/daemon-read.json"
    run_http_timed "$scale" daemon_agent_readiness "$DAEMON_URL" GET /v1/agents/daemonbot/readiness
    python3 - "$WORK/daemon-merge.json" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "agent": "daemonbot",
    "dry_run": True,
}))
PY
    run_http_timed "$scale" daemon_merge_dry_run "$DAEMON_URL" POST /v1/branches/main/merge-agent "$WORK/daemon-merge.json"
    run_timed "$scale" daemon_auto_cli_status "$BIN" --workspace "$REPO" --json status
    run_timed "$scale" daemon_auto_cli_agent_readiness "$BIN" --workspace "$REPO" --json agent readiness daemonbot
    run_timed "$scale" daemon_cli_status "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json status
    run_timed "$scale" daemon_cli_diff_dirty "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json diff --dirty
    run_timed "$scale" daemon_cli_agent_spawn "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json agent spawn daemonclibot --from main --no-materialize
    python3 - "$REPO" "$WORK/daemon-cli-patch.json" "$scale" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
edits = []
for i in range(max(1, min(25, files // 400))):
    idx = (i * 6599 + 37) % files
    d, f = divmod(idx, 100)
    rel = pathlib.Path(f"pkg_{d:05d}") / f"module_{f:03d}.rs"
    edits.append({
        "op": "write",
        "path": str(rel),
        "content": (root / rel).read_text() + f"\n// daemonclibot {i}\n",
    })
json.dump({"message": "scale daemonclibot", "edits": edits}, out.open("w"))
PY
    run_timed "$scale" daemon_cli_agent_patch "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json agent apply-patch daemonclibot --patch "$WORK/daemon-cli-patch.json"
    run_timed "$scale" daemon_cli_agent_read "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json agent read daemonclibot README.md
    run_timed "$scale" daemon_cli_agent_diff "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json agent diff daemonclibot
    run_timed "$scale" daemon_cli_agent_readiness "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json agent readiness daemonclibot
    run_timed "$scale" daemon_cli_merge_dry_run "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json merge-agent daemonclibot --into main --dry-run
    run_timed "$scale" daemon_cli_merge_queue_list "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json merge-queue list
    run_timed "$scale" daemon_cli_merge_queue_run_empty "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json merge-queue run
    DAEMON_RSS="$(daemon_rss_bytes "$DAEMON_PID")"
    kill "$DAEMON_PID" >/dev/null 2>&1 || true
    wait "$DAEMON_PID" 2>/dev/null || true
    trap - EXIT
  fi

  run_timed "$scale" agent_spawn_headless "$BIN" --workspace "$REPO" --json agent spawn patchbot --from main --no-materialize
  run_timed "$scale" agent_read_headless "$BIN" --workspace "$REPO" --json agent read patchbot README.md
  python3 - "$REPO" "$WORK/patchbot.json" "$scale" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
edits = []
for i in range(max(1, min(50, files // 200))):
    idx = (i * 6151 + 17) % files
    d, f = divmod(idx, 100)
    rel = pathlib.Path(f"pkg_{d:05d}") / f"module_{f:03d}.rs"
    edits.append({
        "op": "write",
        "path": str(rel),
        "content": (root / rel).read_text() + f"\n// patchbot {i}\n",
    })
json.dump({"message": "scale patchbot", "edits": edits}, out.open("w"))
PY
  run_timed "$scale" agent_apply_patch "$BIN" --workspace "$REPO" --json agent apply-patch patchbot --patch "$WORK/patchbot.json"
  run_timed "$scale" agent_readiness "$BIN" --workspace "$REPO" --json agent readiness patchbot
  run_timed "$scale" merge_agent_dry_run "$BIN" --workspace "$REPO" --json merge-agent patchbot --into main --dry-run
  run_timed "$scale" merge_agent_apply "$BIN" --workspace "$REPO" --json merge-agent patchbot --into main

  run_timed "$scale" agent_spawn_queuebot "$BIN" --workspace "$REPO" --json agent spawn queuebot --from main --no-materialize
  python3 - "$REPO" "$WORK/queuebot.json" "$scale" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
edits = []
for i in range(max(1, min(25, files // 400))):
    idx = (i * 4231 + 23) % files
    d, f = divmod(idx, 100)
    rel = pathlib.Path(f"pkg_{d:05d}") / f"module_{f:03d}.rs"
    edits.append({
        "op": "write",
        "path": str(rel),
        "content": (root / rel).read_text() + f"\n// queuebot {i}\n",
    })
json.dump({"message": "scale queuebot", "edits": edits}, out.open("w"))
PY
  run_timed "$scale" queuebot_apply_patch "$BIN" --workspace "$REPO" --json agent apply-patch queuebot --patch "$WORK/queuebot.json"
  run_timed "$scale" merge_queue_add "$BIN" --workspace "$REPO" --json merge-queue add queuebot --into main
  run_timed "$scale" merge_queue_run "$BIN" --workspace "$REPO" --json merge-queue run

  if [ "$RUN_MATERIALIZED" = "1" ]; then
    run_timed "$scale" agent_spawn_sparse "$BIN" --workspace "$REPO" --json agent spawn sparsebot --from main --paths README.md
    run_timed "$scale" agent_status_sparse "$BIN" --workspace "$REPO" --json agent status sparsebot
    run_timed "$scale" agent_read_sparse_nohydrate "$BIN" --workspace "$REPO" --json agent read sparsebot pkg_00000/module_001.rs
    run_timed "$scale" agent_read_sparse_hydrate "$BIN" --workspace "$REPO" --json agent read sparsebot pkg_00000/module_003.rs --hydrate
    run_timed "$scale" agent_read_sparse_hydrate_neighbors "$BIN" --workspace "$REPO" --json agent read sparsebot pkg_00000/module_002.rs --hydrate --include-neighbors
    run_timed "$scale" agent_sync_sparse_file "$BIN" --workspace "$REPO" --json agent sync-workdir sparsebot --paths pkg_00000/module_000.rs
    run_timed "$scale" agent_sync_sparse_dir "$BIN" --workspace "$REPO" --json agent sync-workdir sparsebot --paths pkg_00000
    run_timed "$scale" agent_status_sparse_hydrated "$BIN" --workspace "$REPO" --json agent status sparsebot
    run_timed "$scale" agent_spawn_materialized "$BIN" --workspace "$REPO" --json agent spawn matbot --from main --materialize
    run_timed "$scale" agent_status_materialized "$BIN" --workspace "$REPO" --json agent status matbot
  fi

  run_timed "$scale" index_rebuild "$BIN" --workspace "$REPO" --json index rebuild
  run_timed "$scale" gc_dry_run "$BIN" --workspace "$REPO" --json gc --dry-run
  if [ "$RUN_BACKUP" = "1" ]; then
    run_timed "$scale" backup_create "$BIN" --workspace "$REPO" --json backup create --overwrite "$WORK/crabdb-backup"
    run_timed "$scale" backup_verify "$BIN" --workspace "$REPO" --json backup verify "$WORK/crabdb-backup"
  fi

  read -r source_file_count source_bytes < <(repo_source_bytes "$REPO")
  {
    printf 'source_file_count\t%s\n' "$source_file_count"
    printf 'source_bytes\t%s\n' "$source_bytes"
    printf 'sqlite_bytes\t%s\n' "$(sqlite_bytes "$REPO")"
    printf 'object_count\t%s\n' "$(object_count "$REPO")"
    printf 'daemon_rss_bytes\t%s\n' "$DAEMON_RSS"
    du -sk "$REPO" "$REPO/.crabdb" 2>/dev/null | awk '{print "du_kb_" $2 "\t" $1}'
  } > "$WORK/metrics.tsv"

  printf 'scale=%s results=%s metrics=%s\n' "$scale" "$RESULTS" "$WORK/metrics.tsv"
done

printf 'run_root=%s\n' "$RUN_ROOT"

#!/usr/bin/env bash
set -euo pipefail

BIN="${TRAIL_BIN:-}"
MODE="${1:-default}"
if [ "$#" -gt 1 ]; then
  printf 'usage: %s [changed-path-ledger]\n' "$0" >&2
  exit 2
fi
if [ "$MODE" = "changed-path-ledger" ]; then
  SCALES="${TRAIL_SCALE_FILES:-${REPO_FILES:-1000,100000,1000000}}"
else
  SCALES="${TRAIL_SCALE_FILES:-10000}"
fi
BASE_DIR="${TRAIL_SCALE_BASE:-/Volumes/Workspace}"
RUN_LABEL="${TRAIL_SCALE_LABEL:-$(date +%Y%m%d-%H%M%S)}"
RUN_MATERIALIZED="${TRAIL_SCALE_MATERIALIZED:-1}"
RUN_BACKUP="${TRAIL_SCALE_BACKUP:-1}"
RUN_DAEMON="${TRAIL_SCALE_DAEMON:-1}"
RUN_GIT_IMPORT="${TRAIL_SCALE_GIT_IMPORT:-1}"
LEDGER_K_VALUES="${TRAIL_CHANGED_PATH_K_VALUES:-0,1,100}"
RUN_LEDGER_COW="${TRAIL_CHANGED_PATH_COW:-auto}"
LEDGER_TAIL_BOUND="${TRAIL_CHANGED_PATH_TAIL_BOUND:-4096}"

if [ -z "$BIN" ]; then
  cargo build -p trail --release >/dev/null
  BIN="$(pwd)/target/release/trail"
fi

if [ ! -x "$BIN" ]; then
  printf 'trail binary is not executable: %s\n' "$BIN" >&2
  exit 2
fi

RUN_ROOT="$BASE_DIR/trail-cli-scale-$RUN_LABEL"
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
  if ! real="$(awk '
    $1 == "real" && $2 ~ /^[0-9]+([.][0-9]+)?$/ { value=$2; matches++ }
    END { if (matches != 1) exit 1; print value }
  ' "$stderr")"; then
    printf '%s: missing or ambiguous /usr/bin/time real measurement\n' "$name" >&2
    tail -80 "$stderr" >&2 || true
    exit 1
  fi
  if ! rss="$(awk '
    /maximum resident set size kb/ {
      if ($NF !~ /^[0-9]+$/) exit 2
      value=$NF * 1024
      matches++
      next
    }
    /maximum resident set size/ {
      found=""
      for (i=1;i<=NF;i++) if ($i ~ /^[0-9]+$/) { found=$i; break }
      if (found == "") exit 2
      value=found
      matches++
    }
    END { if (matches != 1 || value <= 0) exit 1; printf "%.0f\n", value }
  ' "$stderr")"; then
    printf '%s: missing, ambiguous, or zero /usr/bin/time RSS measurement\n' "$name" >&2
    tail -80 "$stderr" >&2 || true
    exit 1
  fi
  printf '%s\t%s\t%s\t%s\n' "$name" "$real" "$rss" "$code" >> "$results"
  printf 'scale=%s %-36s %8ss rss=%s exit=%s\n' "$scale" "$name" "$real" "$rss" "$code"
  if [ "$code" -ne 0 ]; then
    tail -80 "$stderr" >&2 || true
    tail -80 "$stdout" >&2 || true
    exit "$code"
  fi
}

run_timed_expected_error() {
  local scale="$1"
  local name="$2"
  local expected_exit="$3"
  local expected_code="$4"
  shift 4
  run_timed "$scale" "$name" bash -s -- "$expected_exit" "$expected_code" "$@" <<'SH'
set -euo pipefail
expected_exit="$1"
expected_code="$2"
shift 2
stdout="$(mktemp)"
stderr="$(mktemp)"
cleanup() {
  rm -f "$stdout" "$stderr"
}
trap cleanup EXIT
set +e
"$@" >"$stdout" 2>"$stderr"
code=$?
set -e
cat "$stdout"
cat "$stderr" >&2
if [ "$code" -ne "$expected_exit" ]; then
  printf 'expected exit %s, got %s\n' "$expected_exit" "$code" >&2
  exit 1
fi
python3 - "$expected_code" "$stdout" "$stderr" <<'PY'
import json, pathlib, sys

expected = sys.argv[1]

def codes(value):
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "code" and isinstance(child, str):
                yield child
            yield from codes(child)
    elif isinstance(value, list):
        for child in value:
            yield from codes(child)

found = set()
for filename in sys.argv[2:]:
    text = pathlib.Path(filename).read_text(errors="replace")
    candidates = [text, *text.splitlines()]
    for candidate in candidates:
        candidate = candidate.strip()
        if not candidate:
            continue
        try:
            found.update(codes(json.loads(candidate)))
        except json.JSONDecodeError:
            pass
if expected not in found:
    print(f"expected JSON error code {expected!r}, found {sorted(found)!r}", file=sys.stderr)
    sys.exit(1)
PY
SH
}

repo_source_bytes() {
  python3 - "$1" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
total = 0
count = 0
for path in root.rglob("*"):
    if ".trail" in path.parts or ".git" in path.parts:
        continue
    if path.is_file():
        total += path.stat().st_size
        count += 1
print(f"{count}\t{total}")
PY
}

sqlite_bytes() {
  local db="$1/.trail/index/trail.sqlite"
  python3 - "$db" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
print(path.stat().st_size if path.is_file() else 0)
PY
}

object_count() {
  local db="$1/.trail/index/trail.sqlite"
  if [ -f "$db" ]; then
    sqlite3 "$db" 'SELECT COUNT(*) FROM objects;' 2>/dev/null || printf '0'
  else
    printf '0'
  fi
}

object_kind_stats() {
  local repo="$1"
  local prefix="$2"
  local db="$repo/.trail/index/trail.sqlite"
  if [ -f "$db" ]; then
    sqlite3 "$db" \
      'SELECT kind, COUNT(*), COALESCE(SUM(size_bytes), 0) FROM objects GROUP BY kind ORDER BY kind;' \
      2>/dev/null \
      | awk -F '|' -v prefix="$prefix" '{
          name = $1
          gsub(/[^A-Za-z0-9_.-]/, "_", name)
          print "object_kind_" prefix "_" name "_count\t" $2
          print "object_kind_" prefix "_" name "_bytes\t" $3
        }'
  fi
}

dbstat_bytes() {
  local repo="$1"
  local prefix="$2"
  local db="$repo/.trail/index/trail.sqlite"
  if [ -f "$db" ]; then
    sqlite3 "$db" \
      'SELECT name, SUM(pgsize) FROM dbstat GROUP BY name ORDER BY SUM(pgsize) DESC;' \
      2>/dev/null \
      | awk -F '|' -v prefix="$prefix" '{
          name = $1
          gsub(/[^A-Za-z0-9_.-]/, "_", name)
          print "dbstat_" prefix "_" name "\t" $2
        }'
  fi
}

workdir_manifest_bytes() {
  local repo="$1"
  local prefix="$2"
  python3 - "$repo" "$prefix" <<'PY'
import pathlib, sys
repo = pathlib.Path(sys.argv[1])
prefix = sys.argv[2]
patterns = {
    "clean_workdir": ".trail/workdir-manifest.json",
    "sparse_workdir": ".trail/sparse-workdir.json",
}
counts = {name: 0 for name in patterns}
sizes = {name: 0 for name in patterns}
worktrees = repo / ".trail" / "worktrees"
if worktrees.exists():
    for path in worktrees.rglob("*.json"):
        for name, suffix in patterns.items():
            parent, filename = suffix.split("/")
            if path.parent.name == parent and path.name == filename:
                counts[name] += 1
                sizes[name] += path.stat().st_size
for name in sorted(patterns):
    print(f"manifest_{prefix}_{name}_count\t{counts[name]}")
    print(f"manifest_{prefix}_{name}_bytes\t{sizes[name]}")
PY
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

run_without_daemon_endpoint() {
  local scale="$1"
  local name="$2"
  local bin="$3"
  local repo="$4"
  shift 4
  run_timed "$scale" "$name" bash -s -- "$bin" "$repo" "$@" <<'SH'
set -euo pipefail
bin="$1"
repo="$2"
shift 2
endpoint="$repo/.trail/daemon.json"
hidden="$repo/.trail/daemon.json.persisted-snapshot-bench"
restore_endpoint() {
  if [ -f "$hidden" ]; then
    mv "$hidden" "$endpoint"
  fi
}
trap restore_endpoint EXIT
rm -f "$hidden"
if [ -f "$endpoint" ]; then
  mv "$endpoint" "$hidden"
fi
"$bin" --workspace "$repo" --json "$@"
SH
}

daemon_rss_bytes() {
  local pid="$1"
  if ps -p "$pid" >/dev/null 2>&1; then
    ps -o rss= -p "$pid" | awk '{print $1 * 1024}'
  else
    printf '0'
  fi
}

append_changed_path_structural_metrics() {
  local scale="$1"
  local k="$2"
  local operation="$3"
  local name="$4"
  local source="$5"
  local stdout="$6"
  local repo="$7"
  local destination="$8"
  python3 - "$scale" "$k" "$operation" "$name" "$source" "$stdout" "$repo" "$destination" "$LEDGER_TAIL_BOUND" <<'PY'
import json, pathlib, sqlite3, sys

scale, k, operation, name = map(str, sys.argv[1:5])
source = pathlib.Path(sys.argv[5])
stdout = pathlib.Path(sys.argv[6])
repo = pathlib.Path(sys.argv[7])
destination = pathlib.Path(sys.argv[8])
tail_bound = int(sys.argv[9])

def load_json(path):
    return json.loads(path.read_text())

def changed_count(payload, operation):
    key = {
        "workspace_status": "changed_paths",
        "workspace_diff": "files",
        "workspace_record": "changed_paths",
        "materialized_lane_record": "changed_paths",
        "structured_patch": "changed_paths",
        "cow_checkpoint": "source_paths",
    }[operation]
    value = payload.get(key)
    if not isinstance(value, list):
        raise SystemExit(f"{name}: command report omitted list field {key!r}")
    return len(value)

payload = load_json(stdout)
reports = [json.loads(line) for line in source.read_text().splitlines() if line.strip()]
if len(reports) != 1:
    raise SystemExit(f"{name}: expected exactly one operation metrics report, found {len(reports)}")
report = reports[0]
required = {
    "generation", "operation", "outcome", "input_path_count",
    "canonical_path_count", "expanded_path_count", "final_path_count",
    "full_filesystem_walk_count", "bounded_filesystem_walk_count",
    "filesystem_entry_count", "filesystem_stat_count", "filesystem_read_count",
    "filesystem_read_bytes", "filesystem_hash_count", "filesystem_hash_bytes",
    "full_root_range_count", "bounded_root_range_count", "root_range_row_count",
    "root_point_key_count", "prolly_read_call_count", "prolly_read_key_count",
    "prolly_read_value_count", "prolly_read_value_bytes", "prolly_write_call_count",
    "prolly_write_key_count", "prolly_write_value_bytes",
    "prolly_tree_batch_call_count", "prolly_tree_batch_mutation_count",
    "selected_worktree_index_sqlite_accounting_complete",
    "selected_worktree_index_sqlite_accounting_disposition",
    "selected_worktree_index_sqlite_envelope_count",
    "selected_worktree_index_sqlite_not_applicable_count",
    "selected_worktree_index_sqlite_full_scan_count",
    "selected_worktree_index_sqlite_row_read_count",
    "selected_worktree_index_sqlite_row_delete_count",
    "selected_worktree_index_sqlite_row_upsert_count",
    "selected_worktree_index_sqlite_statement_count",
    "selected_worktree_index_sqlite_transaction_count", "selection_comparison_count",
    "policy_build_count", "policy_dependency_full_discovery", "policy_dependency_bytes",
    "policy_dependency_file_count", "git_subprocess_count", "git_global_work_count",
    "git_index_refresh_count", "git_trace2_region_count", "git_trace2_bytes",
    "git_fsmonitor_qualification_count", "git_untracked_cache_qualification_count",
    "external_adapter_global_work", "git_index_read_count", "git_index_bytes",
    "git_shared_index_read_count", "git_shared_index_bytes", "git_output_bytes",
    "git_output_record_count", "daemon_snapshot_bytes", "daemon_snapshot_path_count",
    "daemon_cumulative_rewrite_count", "daemon_cumulative_rewrite_bytes",
    "daemon_cumulative_rewrite_count_total", "daemon_cumulative_rewrite_bytes_total",
    "authoritative_candidate_count", "ledger_row_touch_count",
    "observer_tail_record_fold_count", "reconciliation_run_count", "manifest_bytes",
    "manifest_key_comparison_count", "journal_bytes", "upper_work_count",
    "wall_time_ns", "rss_start_bytes", "rss_end_bytes",
    "rss_lifetime_high_water_bytes",
}
missing = sorted(required.difference(report))
if missing:
    raise SystemExit(f"{name}: operation metrics report omitted {', '.join(missing)}")
report["metric_source"] = "operation_scope"
if operation in {"materialized_lane_record", "cow_checkpoint"}:
    if "upper_recovery_walks" not in payload:
        raise SystemExit(f"{name}: command report omitted upper_recovery_walks")
    report["upper_recovery_walks"] = int(payload["upper_recovery_walks"])
if operation == "cow_checkpoint":
    if payload.get("generated_path_accounting") != "journal_interval":
        raise SystemExit(f"{name}: command report omitted journal_interval generated-path accounting")
    report["generated_path_accounting"] = payload["generated_path_accounting"]
if operation in {"materialized_lane_record", "structured_patch"}:
    path_index = payload.get("path_index")
    required_path_index = {
        "mode", "lookup_count", "full_root_path_load_count",
        "full_filesystem_path_scan_count",
    }
    if not isinstance(path_index, dict) or not required_path_index.issubset(path_index):
        raise SystemExit(f"{name}: command report omitted complete path_index metrics")
    report.update({
        "path_index_full_root_path_load_count": int(path_index["full_root_path_load_count"]),
        "path_index_full_filesystem_path_scan_count": int(path_index["full_filesystem_path_scan_count"]),
        "path_index_lookup_count": int(path_index["lookup_count"]),
        "path_index_mode": path_index["mode"],
    })

report.update({
    "benchmark": name,
    "benchmark_operation": operation,
    "repo_files": int(scale),
    "authoritative_input_k": int(k),
    "final_changed_output": changed_count(payload, operation),
    "configured_tail_bound": tail_bound,
})

db = repo / ".trail" / "index" / "trail.sqlite"
report["scope_caps"] = []
if db.is_file():
    connection = sqlite3.connect(db)
    try:
        query = """
            SELECT scope.scope_id,
                   (SELECT COUNT(*) FROM changed_path_entries entry WHERE entry.scope_id=scope.scope_id),
                   scope.max_candidate_rows,
                   (SELECT COUNT(*) FROM changed_path_prefixes prefix WHERE prefix.scope_id=scope.scope_id),
                   scope.max_prefix_rows,
                   COALESCE((SELECT SUM(segment.durable_end_offset)
                             FROM changed_path_observer_segments segment
                             WHERE segment.scope_id=scope.scope_id AND segment.state!='retired'), 0),
                   scope.max_observer_log_bytes,
                   COALESCE((SELECT MAX(segment.durable_end_offset)
                             FROM changed_path_observer_segments segment
                             WHERE segment.scope_id=scope.scope_id AND segment.state!='retired'), 0),
                   scope.max_segment_bytes
            FROM changed_path_scopes scope
            WHERE scope.retired_at IS NULL
            ORDER BY scope.scope_id
        """
        report["scope_caps"] = [
            {
                "scope_id": row[0], "candidate_rows": row[1], "candidate_row_cap": row[2],
                "prefix_rows": row[3], "prefix_row_cap": row[4],
                "observer_log_bytes": row[5], "observer_log_byte_cap": row[6],
                "largest_segment_bytes": row[7], "segment_byte_cap": row[8],
            }
            for row in connection.execute(query)
        ]
    finally:
        connection.close()

with destination.open("a") as handle:
    handle.write(json.dumps(report, sort_keys=True) + "\n")
PY
}

run_changed_path_scoped() {
  local scale="$1"
  local k="$2"
  local operation="$3"
  local name="ledger_${operation}_k${k}"
  local repo="$4"
  shift 4
  local metrics="$RUN_ROOT/$scale/changed-path-operation-metrics.jsonl"
  local segment="$RUN_ROOT/$scale/out/$name.metrics.jsonl"
  local before after expected_operation
  before="$(python3 - "$metrics" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
print(path.stat().st_size if path.exists() else 0)
PY
)"
  run_timed "$scale" "$name" "$@"
  after="$(python3 - "$metrics" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
print(path.stat().st_size if path.exists() else 0)
PY
)"
  case "$operation" in
    workspace_status) expected_operation=status ;;
    workspace_diff) expected_operation=diff ;;
    workspace_record) expected_operation=record ;;
    materialized_lane_record) expected_operation=materialized_lane_record ;;
    structured_patch) expected_operation=structured_patch ;;
    cow_checkpoint) expected_operation=cow_checkpoint ;;
    *) printf 'unsupported scoped operation: %s\n' "$operation" >&2; return 2 ;;
  esac
  python3 - "$metrics" "$before" "$after" "$expected_operation" "$segment" <<'PY'
import json, pathlib, sys
source, start, end, operation, destination = pathlib.Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]), sys.argv[4], pathlib.Path(sys.argv[5])
if end <= start:
    raise SystemExit(f"{operation}: operation metrics sidecar did not grow")
with source.open("rb") as handle:
    handle.seek(start)
    data = handle.read(end - start).decode()
reports = [json.loads(line) for line in data.splitlines() if line.strip()]
matches = [report for report in reports if report.get("operation") == operation]
if len(matches) != 1:
    raise SystemExit(f"{operation}: expected one new matching metrics report, found {len(matches)} among {len(reports)} new reports")
destination.write_text(json.dumps(matches[0], sort_keys=True) + "\n")
PY
  append_changed_path_structural_metrics \
    "$scale" "$k" "$operation" "$name" "$segment" \
    "$RUN_ROOT/$scale/out/$name.stdout" "$repo" \
    "$RUN_ROOT/$scale/structural-metrics.jsonl"
}

json_path_set() {
  local operation="$1"
  local path="$2"
  python3 - "$operation" "$path" <<'PY'
import json, pathlib, sys
operation, filename = sys.argv[1:]
payload = json.loads(pathlib.Path(filename).read_text())
key = {
    "workspace_status": "changed_paths",
    "workspace_diff": "files",
    "workspace_record": "changed_paths",
    "materialized_lane_record": "changed_paths",
    "structured_patch": "changed_paths",
    "cow_checkpoint": "source_paths",
}[operation]
values = payload.get(key)
if not isinstance(values, list):
    raise SystemExit(f"oracle report omitted {key!r}")
paths = []
for value in values:
    if isinstance(value, str):
        paths.append(value)
    elif isinstance(value, dict) and isinstance(value.get("path"), str):
        paths.append(value["path"])
    else:
        raise SystemExit(f"oracle report has an invalid path entry: {value!r}")
sys.stdout.write("".join(path + "\n" for path in sorted(set(paths))))
PY
}

mutate_changed_path_fixture() {
  local root="$1"
  local k="$2"
  local label="$3"
  python3 - "$root" "$k" "$label" <<'PY'
import pathlib, sys
root, k, label = pathlib.Path(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
for i in range(k):
    path = root / f"pkg_{i // 100:05d}" / f"module_{i % 100:03d}.rs"
    with path.open("a") as handle:
        handle.write(f"\n// changed-path-ledger {label} {i}\n")
PY
}

write_changed_path_oracle_manifest() {
  local root="$1"
  local destination="$2"
  python3 - "$root" "$destination" <<'PY'
import hashlib, json, os, pathlib, stat, sys
root, destination = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
manifest = {}
macos_storage_names = {
    ".DS_Store", ".Spotlight-V100", ".Trashes", ".fseventsd",
    ".metadata_never_index", ".metadata_never_index_unless_rootfs",
    ".metadata_direct_scope_only",
}
def is_platform_storage_noise(name):
    # Match the native NFS-COW adapter's non-user platform metadata filter so
    # AppleDouble/Finder/FSEvents artifacts cannot masquerade as source paths
    # in the independent full-scan oracle.
    return name.startswith("._") or name in macos_storage_names
for directory, names, files in os.walk(root):
    relative_directory = pathlib.Path(directory).relative_to(root)
    names[:] = [
        name for name in names
        if name not in {".trail", ".git"} and not is_platform_storage_noise(name)
    ]
    for name in files:
        if is_platform_storage_noise(name):
            continue
        path = pathlib.Path(directory) / name
        relative = (relative_directory / name).as_posix()
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            manifest[relative] = {"kind": "symlink", "target": os.readlink(path)}
        elif stat.S_ISREG(metadata.st_mode):
            digest = hashlib.sha256()
            with path.open("rb") as handle:
                for block in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(block)
            manifest[relative] = {
                "kind": "file",
                "sha256": digest.hexdigest(),
                "executable": bool(metadata.st_mode & 0o111),
            }
destination.write_text(json.dumps(manifest, sort_keys=True))
PY
}

run_changed_path_external_oracle() {
  local scale="$1"
  local k="$2"
  local benchmark="$3"
  local operation="$4"
  local measured="$5"
  local root="$6"
  local baseline="$7"
  local update_baseline="${8:-0}"
  local oracle_dir="$RUN_ROOT/$scale/oracle"
  local measured_paths="$oracle_dir/$benchmark.measured.paths"
  local oracle_paths="$oracle_dir/$benchmark.oracle.paths"
  local current="$oracle_dir/$benchmark.current-manifest.json"
  local started finished
  started="$(python3 -c 'import time; print(time.monotonic_ns())')"
  write_changed_path_oracle_manifest "$root" "$current"
  python3 - "$baseline" "$current" "$oracle_paths" <<'PY'
import json, pathlib, sys
before = json.loads(pathlib.Path(sys.argv[1]).read_text())
after = json.loads(pathlib.Path(sys.argv[2]).read_text())
changed = sorted(path for path in before.keys() | after.keys() if before.get(path) != after.get(path))
pathlib.Path(sys.argv[3]).write_text("".join(path + "\n" for path in changed))
PY
  finished="$(python3 -c 'import time; print(time.monotonic_ns())')"
  json_path_set "$operation" "$measured" >"$measured_paths"
  if ! cmp -s "$measured_paths" "$oracle_paths"; then
    diff -u "$oracle_paths" "$measured_paths" >&2 || true
    printf '%s: measured output differs from the external full-scan oracle\n' "$benchmark" >&2
    printf '%s\t%s\t%s\t%s\t%s\t%s\t0\n' \
      "$benchmark" "$scale" "$k" "$((finished - started))" \
      "$(wc -l < "$measured_paths" | tr -d ' ')" \
      "$(wc -l < "$oracle_paths" | tr -d ' ')" \
      >> "$RUN_ROOT/$scale/oracle-results.tsv"
    return 1
  fi
  if [ "$update_baseline" = "1" ]; then
    mv "$current" "$baseline"
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t1\n' \
    "$benchmark" "$scale" "$k" "$((finished - started))" \
    "$(wc -l < "$measured_paths" | tr -d ' ')" \
    "$(wc -l < "$oracle_paths" | tr -d ' ')" \
    >> "$RUN_ROOT/$scale/oracle-results.tsv"
}

prepare_changed_path_external_oracle() {
  local scale="$1"
  local benchmark="$2"
  local root="$3"
  local baseline="$4"
  local oracle_dir="$RUN_ROOT/$scale/oracle"
  local current="$oracle_dir/$benchmark.current-manifest.json"
  local started finished
  started="$(python3 -c 'import time; print(time.monotonic_ns())')"
  write_changed_path_oracle_manifest "$root" "$current"
  python3 - "$baseline" "$current" "$oracle_dir/$benchmark.oracle.paths" <<'PY'
import json, pathlib, sys
before = json.loads(pathlib.Path(sys.argv[1]).read_text())
after = json.loads(pathlib.Path(sys.argv[2]).read_text())
changed = sorted(path for path in before.keys() | after.keys() if before.get(path) != after.get(path))
pathlib.Path(sys.argv[3]).write_text("".join(path + "\n" for path in changed))
PY
  finished="$(python3 -c 'import time; print(time.monotonic_ns())')"
  printf '%s\n' "$((finished - started))" >"$oracle_dir/$benchmark.oracle-time-ns"
}

# NFS/FUSE COW views inherit an already independently scanned main baseline.
# Rewalking and rehashing every base file through the virtual mount would make
# the benchmark oracle itself O(N) and obscure the checkpoint measurement. Read
# the known benchmark mutations back through the mounted view and derive the
# next external manifest from that baseline instead.
prepare_cow_changed_path_external_oracle() {
  local scale="$1"
  local benchmark="$2"
  local root="$3"
  local baseline="$4"
  local k="$5"
  local oracle_dir="$RUN_ROOT/$scale/oracle"
  local current="$oracle_dir/$benchmark.current-manifest.json"
  local started finished
  started="$(python3 -c 'import time; print(time.monotonic_ns())')"
  python3 - "$baseline" "$root" "$k" "$current" \
    "$oracle_dir/$benchmark.oracle.paths" <<'PY'
import hashlib, json, pathlib, stat, sys
baseline, root, k, current, changed_paths = (
    pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), int(sys.argv[3]),
    pathlib.Path(sys.argv[4]), pathlib.Path(sys.argv[5]),
)
before = json.loads(baseline.read_text())
after = dict(before)
for index in range(k):
    relative = f"cow-{k}-{index:03d}.txt"
    path = root / relative
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode):
        raise SystemExit(f"COW oracle mutation is not a regular file: {relative}")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    after[relative] = {
        "kind": "file",
        "sha256": digest,
        "executable": bool(metadata.st_mode & 0o111),
    }
current.write_text(json.dumps(after, sort_keys=True))
changed = sorted(path for path in before.keys() | after.keys() if before.get(path) != after.get(path))
changed_paths.write_text("".join(path + "\n" for path in changed))
PY
  finished="$(python3 -c 'import time; print(time.monotonic_ns())')"
  printf '%s\n' "$((finished - started))" >"$oracle_dir/$benchmark.oracle-time-ns"
}

finish_changed_path_external_oracle() {
  local scale="$1"
  local k="$2"
  local benchmark="$3"
  local operation="$4"
  local measured="$5"
  local baseline="$6"
  local oracle_dir="$RUN_ROOT/$scale/oracle"
  local measured_paths="$oracle_dir/$benchmark.measured.paths"
  local oracle_paths="$oracle_dir/$benchmark.oracle.paths"
  json_path_set "$operation" "$measured" >"$measured_paths"
  local equal=1
  if ! cmp -s "$measured_paths" "$oracle_paths"; then
    equal=0
    diff -u "$oracle_paths" "$measured_paths" >&2 || true
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$benchmark" "$scale" "$k" "$(cat "$oracle_dir/$benchmark.oracle-time-ns")" \
    "$(wc -l < "$measured_paths" | tr -d ' ')" \
    "$(wc -l < "$oracle_paths" | tr -d ' ')" "$equal" \
    >> "$RUN_ROOT/$scale/oracle-results.tsv"
  if [ "$equal" != "1" ]; then
    printf '%s: measured output differs from the external full-scan oracle\n' "$benchmark" >&2
    return 1
  fi
  mv "$oracle_dir/$benchmark.current-manifest.json" "$baseline"
}

run_changed_path_ledger_mode() {
  for scale in ${SCALES//,/ }; do
    case "$scale" in
      1000|100000|1000000) ;;
      *)
        printf 'changed-path-ledger mode supports scales 1000, 100000, and 1000000; got %s\n' "$scale" >&2
        return 2
        ;;
    esac
    local work="$RUN_ROOT/$scale"
    local repo="$work/repo"
    rm -rf "$work"
    mkdir -p "$repo" "$work/out" "$work/oracle"
    printf 'name\treal_seconds\tmax_rss_bytes\texit_code\n' >"$work/results.tsv"
    printf 'benchmark\trepo_files\tauthoritative_input_k\toracle_time_ns\tmeasured_paths\toracle_paths\tequal\n' \
      >"$work/oracle-results.tsv"
    : >"$work/structural-metrics.jsonl"
    export TRAIL_PERFORMANCE_METRICS=1
    export TRAIL_PERFORMANCE_METRICS_FILE="$work/changed-path-operation-metrics.jsonl"
    : >"$TRAIL_PERFORMANCE_METRICS_FILE"

    run_timed "$scale" ledger_generate_repo python3 - "$repo" "$scale" <<'PY'
import pathlib, sys
root, files = pathlib.Path(sys.argv[1]), int(sys.argv[2])
for index in range(files):
    path = root / f"pkg_{index // 100:05d}" / f"module_{index % 100:03d}.rs"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"pub fn value_{index:08d}() -> usize {{ {index} }}\n")
(root / "README.md").write_text(f"# changed-path ledger scale {files}\n")
(root / ".gitignore").write_text("target/\nnode_modules/\n.DS_Store\n")
PY
    git -C "$repo" init --quiet
    git -C "$repo" config user.name "Trail Scale Benchmark"
    git -C "$repo" config user.email "trail-scale@example.invalid"
    run_timed "$scale" ledger_git_add git -C "$repo" add --all
    run_timed "$scale" ledger_git_commit git -C "$repo" commit --quiet -m "changed-path ledger baseline"
    run_timed "$scale" ledger_init "$BIN" --workspace "$repo" --json init --from-git
    run_timed "$scale" ledger_cold_reconcile \
      "$BIN" --workspace "$repo" --json index reconcile
    cp "$work/out/ledger_cold_reconcile.stdout" \
      "$work/oracle/ledger_workspace_warm.reconcile.json"
    "$BIN" --workspace "$repo" --json status \
      >"$work/oracle/ledger_workspace_warm.status.json"
    local workspace_oracle_baseline="$work/oracle/workspace-baseline.json"
    write_changed_path_oracle_manifest "$repo" "$workspace_oracle_baseline"

    for k in ${LEDGER_K_VALUES//,/ }; do
      case "$k" in
        0|1|100) ;;
        *) printf 'invalid changed-path candidate count: %s\n' "$k" >&2; return 2 ;;
      esac
      mutate_changed_path_fixture "$repo" "$k" "workspace-k$k"
      run_changed_path_scoped "$scale" "$k" workspace_status "$repo" \
        "$BIN" --workspace "$repo" --json status
      run_changed_path_external_oracle "$scale" "$k" "ledger_workspace_status_k$k" \
        workspace_status "$work/out/ledger_workspace_status_k$k.stdout" "$repo" \
        "$workspace_oracle_baseline"
      run_changed_path_scoped "$scale" "$k" workspace_diff "$repo" \
        "$BIN" --workspace "$repo" --json diff --dirty
      run_changed_path_external_oracle "$scale" "$k" "ledger_workspace_diff_k$k" \
        workspace_diff "$work/out/ledger_workspace_diff_k$k.stdout" "$repo" \
        "$workspace_oracle_baseline"
      run_changed_path_scoped "$scale" "$k" workspace_record "$repo" \
        "$BIN" --workspace "$repo" --json record -m "changed-path ledger workspace k=$k"
      run_changed_path_external_oracle "$scale" "$k" "ledger_workspace_record_k$k" \
        workspace_record "$work/out/ledger_workspace_record_k$k.stdout" "$repo" \
        "$workspace_oracle_baseline" 1
    done

    run_timed "$scale" ledger_materialized_spawn \
      "$BIN" --workspace "$repo" --json lane spawn ledger-materialized --from main --materialize
    local materialized_workdir
    materialized_workdir="$(python3 - "$work/out/ledger_materialized_spawn.stdout" <<'PY'
import json, pathlib, sys
print(json.loads(pathlib.Path(sys.argv[1]).read_text()).get("workdir") or "")
PY
)"
    if [ -z "$materialized_workdir" ]; then
      printf 'changed-path materialized lane did not return a workdir\n' >&2
      return 1
    fi
    "$BIN" --workspace "$repo" --json index reconcile --lane ledger-materialized \
      >"$work/oracle/ledger_materialized_warm.reconcile.json"
    local materialized_oracle_baseline="$work/oracle/materialized-baseline.json"
    write_changed_path_oracle_manifest "$materialized_workdir" "$materialized_oracle_baseline"
    for k in ${LEDGER_K_VALUES//,/ }; do
      mutate_changed_path_fixture "$materialized_workdir" "$k" "materialized-k$k"
      run_changed_path_scoped "$scale" "$k" materialized_lane_record "$repo" \
        "$BIN" --workspace "$repo" --json lane record ledger-materialized \
        -m "changed-path ledger materialized k=$k"
      run_changed_path_external_oracle "$scale" "$k" "ledger_materialized_lane_record_k$k" \
        materialized_lane_record "$work/out/ledger_materialized_lane_record_k$k.stdout" \
        "$materialized_workdir" "$materialized_oracle_baseline" 1
    done

    for k in ${LEDGER_K_VALUES//,/ }; do
      python3 - "$materialized_workdir" "$work/structured-patch-k$k.json" "$k" <<'PY'
import json, pathlib, sys
root, output, k = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), int(sys.argv[3])
edits = []
for i in range(k):
    relative = pathlib.Path(f"pkg_{i // 100:05d}") / f"module_{i % 100:03d}.rs"
    edits.append({"op": "write", "path": str(relative), "content": (root / relative).read_text() + f"\n// structured patch {k}:{i}\n"})
output.write_text(json.dumps({"allow_stale": True, "message": f"changed-path structured patch k={k}", "edits": edits}))
PY
      run_changed_path_scoped "$scale" "$k" structured_patch "$repo" \
        "$BIN" --workspace "$repo" --json lane apply-patch ledger-materialized \
        --patch "$work/structured-patch-k$k.json"
      run_changed_path_external_oracle "$scale" "$k" "ledger_structured_patch_k$k" \
        structured_patch "$work/out/ledger_structured_patch_k$k.stdout" \
        "$materialized_workdir" "$materialized_oracle_baseline" 1
    done

    local cow_mode=""
    if [ "$RUN_LEDGER_COW" = "1" ] || { [ "$RUN_LEDGER_COW" = "auto" ] && [ -e /dev/fuse ]; }; then
      if [ "$(uname -s)" = "Darwin" ]; then cow_mode="nfs-cow"; else cow_mode="fuse-cow"; fi
    elif [ "$RUN_LEDGER_COW" = "auto" ] && [ "$(uname -s)" = "Darwin" ]; then
      cow_mode="nfs-cow"
    elif [ "$RUN_LEDGER_COW" != "0" ] && [ "$RUN_LEDGER_COW" != "auto" ]; then
      printf 'TRAIL_CHANGED_PATH_COW must be 0, 1, or auto\n' >&2
      return 2
    fi
    if [ -n "$cow_mode" ]; then
      run_timed "$scale" ledger_cow_spawn \
        "$BIN" --workspace "$repo" --json lane spawn ledger-cow --from main --workdir-mode "$cow_mode"
      local cow_workdir
      cow_workdir="$(python3 - "$work/out/ledger_cow_spawn.stdout" <<'PY'
import json, pathlib, sys
print(json.loads(pathlib.Path(sys.argv[1]).read_text()).get("workdir") or "")
PY
      )"
      local cow_oracle_baseline="$work/oracle/cow-baseline.json"
      cp "$workspace_oracle_baseline" "$cow_oracle_baseline"
      for k in ${LEDGER_K_VALUES//,/ }; do
        "$BIN" --workspace "$repo" lane mount ledger-cow \
          >"$work/out/ledger_cow_mount_k$k.stdout" 2>"$work/out/ledger_cow_mount_k$k.stderr" &
        local mount_pid=$!
        python3 - "$cow_workdir" <<'PY'
import pathlib, sys, time
root = pathlib.Path(sys.argv[1])
deadline = time.time() + 120
while time.time() < deadline:
    if (root / "README.md").is_file():
        raise SystemExit(0)
    time.sleep(0.1)
raise SystemExit("COW mount did not become ready")
PY
        python3 - "$cow_workdir" "$k" <<'PY'
import pathlib, sys
root, k = pathlib.Path(sys.argv[1]), int(sys.argv[2])
for i in range(k):
    (root / f"cow-{k}-{i:03d}.txt").write_text(f"COW checkpoint {k}:{i}\n")
PY
        prepare_cow_changed_path_external_oracle "$scale" "ledger_cow_checkpoint_k$k" \
          "$cow_workdir" "$cow_oracle_baseline" "$k"
        "$BIN" --workspace "$repo" lane unmount ledger-cow >/dev/null
        wait "$mount_pid"
        run_changed_path_scoped "$scale" "$k" cow_checkpoint "$repo" \
          "$BIN" --workspace "$repo" --json lane checkpoint ledger-cow \
          -m "changed-path ledger COW k=$k"
        finish_changed_path_external_oracle "$scale" "$k" "ledger_cow_checkpoint_k$k" \
          cow_checkpoint "$work/out/ledger_cow_checkpoint_k$k.stdout" "$cow_oracle_baseline"
      done
    else
      printf 'scale=%s changed-path COW checkpoint skipped (native mount unavailable)\n' "$scale"
    fi

    printf 'scale=%s results=%s structural=%s oracle=%s\n' \
      "$scale" "$work/results.tsv" "$work/structural-metrics.jsonl" "$work/oracle-results.tsv"
  done
}

if [ "$MODE" = "changed-path-ledger" ]; then
  run_changed_path_ledger_mode
  printf 'run_root=%s\n' "$RUN_ROOT"
  exit 0
fi
if [ "$MODE" != "default" ]; then
  printf 'unknown benchmark mode: %s\n' "$MODE" >&2
  exit 2
fi

for scale in ${SCALES//,/ }; do
  case "$scale" in
    ''|*[!0-9]*)
      printf 'invalid scale: %s\n' "$scale" >&2
      exit 2
      ;;
  esac

  WORK="$RUN_ROOT/$scale"
  REPO="$WORK/repo"
  EMPTY_REPO="$WORK/empty-root-repo"
  GIT_REPO="$WORK/git-repo"
  GIT_UNMAPPED_REPO="$WORK/git-unmapped-repo"
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
(root / "README.md").write_text(f"# Trail CLI scale {files}\n")
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

  if [ "$RUN_GIT_IMPORT" = "1" ]; then
    if command -v git >/dev/null 2>&1; then
      mkdir -p "$GIT_REPO"
      run_timed "$scale" git_generate_repo python3 - "$GIT_REPO" "$scale" <<'PY'
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
(root / "README.md").write_text(f"# Trail Git import scale {files}\n")
(root / ".gitignore").write_text("target/\nnode_modules/\n.DS_Store\n")
PY
      run_timed "$scale" git_init git -C "$GIT_REPO" init
      run_timed "$scale" git_add_tracked git -C "$GIT_REPO" add .
      git -C "$GIT_REPO" config user.email "trail@example.com"
      git -C "$GIT_REPO" config user.name "Trail"
      run_timed "$scale" git_commit_initial git -C "$GIT_REPO" commit -m "scale initial"
      run_timed "$scale" git_clone_unmapped git clone --no-local --quiet "$GIT_REPO" "$GIT_UNMAPPED_REPO"
      git -C "$GIT_UNMAPPED_REPO" config user.email "trail@example.com"
      git -C "$GIT_UNMAPPED_REPO" config user.name "Trail"
      run_timed "$scale" git_init_from_git "$BIN" --workspace "$GIT_REPO" --json init --from-git
      run_timed "$scale" agent_git_spawn "$BIN" --workspace "$GIT_REPO" --json lane spawn agent-gitapplybot --from main --no-materialize
      python3 - "$GIT_REPO" "$WORK/agent-git-apply.json" "$scale" "$WORK/out/agent_git_spawn.stdout" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
base_change = json.loads(pathlib.Path(sys.argv[4]).read_text())["base_change"]
changed_paths = max(1, min(100, files // 1000))
edits = []
for i in range(changed_paths):
    idx = (i * 7919 + 43) % files
    d, f = divmod(idx, 100)
    rel = pathlib.Path(f"pkg_{d:05d}") / f"module_{f:03d}.rs"
    edits.append({
        "op": "write",
        "path": str(rel),
        "content": (root / rel).read_text() + f"\n// agent Git apply {i}\n",
    })
json.dump({"base_change": base_change, "message": "scale Git apply", "edits": edits}, out.open("w"))
PY
      run_timed "$scale" agent_git_apply_patch "$BIN" --workspace "$GIT_REPO" --json lane apply-patch agent-gitapplybot --patch "$WORK/agent-git-apply.json"
      run_timed "$scale" agent_git_mark_reviewed "$BIN" --workspace "$GIT_REPO" --json agent mark-reviewed latest --note "scale Git apply reviewed"
      run_timed "$scale" agent_git_ready "$BIN" --workspace "$GIT_REPO" --json agent ready latest
      run_timed "$scale" agent_git_apply_dry_run "$BIN" --workspace "$GIT_REPO" --json agent apply latest --dry-run
      run_timed "$scale" agent_git_apply "$BIN" --workspace "$GIT_REPO" --json agent apply latest
      python3 scripts/extract-agent-git-performance.py \
        "$WORK/out/agent_git_apply.stdout" \
        "$WORK/agent-git-metrics.tsv"
      GIT_UNMAPPED_HEAD_BEFORE="$(git -C "$GIT_UNMAPPED_REPO" rev-parse HEAD)"
      GIT_UNMAPPED_INDEX_BEFORE="$(git hash-object "$GIT_UNMAPPED_REPO/.git/index")"
      run_timed "$scale" git_unmapped_init_working_tree "$BIN" --workspace "$GIT_UNMAPPED_REPO" --json init --working-tree
      run_timed "$scale" git_unmapped_mappings "$BIN" --workspace "$GIT_UNMAPPED_REPO" --json git mappings
      python3 - "$WORK/out/git_unmapped_mappings.stdout" <<'PY'
import json, pathlib, sys
payload = json.loads(pathlib.Path(sys.argv[1]).read_text())
if payload != []:
    raise SystemExit(f"working-tree init unexpectedly created Git mappings: {payload!r}")
PY
      run_timed "$scale" agent_git_unmapped_spawn "$BIN" --workspace "$GIT_UNMAPPED_REPO" --json lane spawn agent-gitunmappedbot --from main --no-materialize
      python3 - "$GIT_UNMAPPED_REPO" "$WORK/agent-git-unmapped.json" "$WORK/out/agent_git_unmapped_spawn.stdout" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
base_change = json.loads(pathlib.Path(sys.argv[3]).read_text())["base_change"]
rel = pathlib.Path("pkg_00000/module_000.rs")
json.dump({
    "base_change": base_change,
    "message": "scale missing Git mapping",
    "edits": [{
        "op": "write",
        "path": str(rel),
        "content": (root / rel).read_text() + "\n// missing Git mapping\n",
    }],
}, out.open("w"))
PY
      run_timed "$scale" agent_git_unmapped_apply_patch "$BIN" --workspace "$GIT_UNMAPPED_REPO" --json lane apply-patch agent-gitunmappedbot --patch "$WORK/agent-git-unmapped.json"
      run_timed "$scale" agent_git_unmapped_mark_reviewed "$BIN" --workspace "$GIT_UNMAPPED_REPO" --json agent mark-reviewed latest --note "scale missing mapping reviewed"
      run_timed "$scale" agent_git_unmapped_ready "$BIN" --workspace "$GIT_UNMAPPED_REPO" --json agent ready latest
      run_timed_expected_error "$scale" agent_git_apply_missing_mapping 10 GIT_MAPPING_REQUIRED "$BIN" --workspace "$GIT_UNMAPPED_REPO" --json agent apply latest --dry-run
      if [ "$(git -C "$GIT_UNMAPPED_REPO" rev-parse HEAD)" != "$GIT_UNMAPPED_HEAD_BEFORE" ]; then
        printf 'missing-mapping apply changed Git HEAD\n' >&2
        exit 1
      fi
      if [ "$(git hash-object "$GIT_UNMAPPED_REPO/.git/index")" != "$GIT_UNMAPPED_INDEX_BEFORE" ]; then
        printf 'missing-mapping apply changed Git index\n' >&2
        exit 1
      fi
      run_timed "$scale" git_unmapped_mappings_after_apply "$BIN" --workspace "$GIT_UNMAPPED_REPO" --json git mappings
      python3 - "$WORK/out/git_unmapped_mappings_after_apply.stdout" <<'PY'
import json, pathlib, sys
payload = json.loads(pathlib.Path(sys.argv[1]).read_text())
if payload != []:
    raise SystemExit(f"missing-mapping apply unexpectedly wrote mappings: {payload!r}")
PY
      run_timed "$scale" git_mutate_tracked python3 - "$GIT_REPO" "$scale" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
files = int(sys.argv[2])
edit_count = max(1, min(100, files // 100))
for i in range(edit_count):
    idx = (i * 7919) % files
    d, f = divmod(idx, 100)
    path = root / f"pkg_{d:05d}" / f"module_{f:03d}.rs"
    with path.open("a") as fh:
        fh.write(f"\n// git import update edit {i}\npub const IMPORT_EDIT_{i}: usize = {i};\n")
new_path = root / "pkg_00000" / "new_git_tracked.rs"
new_path.write_text("pub fn new_git_tracked() -> usize { 1 }\n")
(root / "scratch-untracked.txt").write_text("not tracked by git import\n")
if files > 101:
    (root / "pkg_00001" / "module_001.rs").unlink()
PY
      run_timed "$scale" git_add_new_tracked git -C "$GIT_REPO" add pkg_00000/new_git_tracked.rs
      run_timed "$scale" git_dirty_status "$BIN" --workspace "$GIT_REPO" --json status
      run_timed "$scale" git_dirty_diff "$BIN" --workspace "$GIT_REPO" --json diff --dirty
      run_timed "$scale" git_import_update "$BIN" --workspace "$GIT_REPO" --json git import-update -m "scale git import update"
      run_timed "$scale" git_import_update_noop "$BIN" --workspace "$GIT_REPO" --json git import-update -m "scale git import update noop"
      run_timed "$scale" git_status_after_import "$BIN" --workspace "$GIT_REPO" --json status
      rm -f "$GIT_REPO/scratch-untracked.txt"
      run_timed "$scale" git_mutate_for_dirty_record python3 - "$GIT_REPO" "$scale" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
files = int(sys.argv[2])
edit_count = max(1, min(25, files // 400))
for i in range(edit_count):
    idx = (i * 6151 + 11) % files
    d, f = divmod(idx, 100)
    path = root / f"pkg_{d:05d}" / f"module_{f:03d}.rs"
    with path.open("a") as fh:
        fh.write(f"\n// git dirty record edit {i}\n")
(root / "pkg_00000" / "git_dirty_record_new.rs").write_text("pub fn git_dirty_record_new() -> usize { 1 }\n")
(root / "git_dirty_record_untracked.txt").write_text("record me through git dirty shortcut\n")
PY
      run_timed "$scale" git_add_dirty_record_new git -C "$GIT_REPO" add pkg_00000/git_dirty_record_new.rs
      run_timed "$scale" git_dirty_record "$BIN" --workspace "$GIT_REPO" --json record -m "scale git dirty record"
      run_timed "$scale" git_status_after_dirty_record "$BIN" --workspace "$GIT_REPO" --json status
    else
      printf 'scale=%s %-36s %8s rss=%s exit=%s\n' "$scale" git_import_update "skipped" 0 0
    fi
  fi

  DAEMON_RSS=0
  if [ "$RUN_DAEMON" = "1" ]; then
    DAEMON_PORT="$(free_loopback_port)"
    DAEMON_URL="http://127.0.0.1:$DAEMON_PORT"
    "$BIN" --workspace "$REPO" --quiet daemon --host 127.0.0.1 --port "$DAEMON_PORT" --no-auth \
      >"$WORK/out/daemon.stdout" 2>"$WORK/out/daemon.stderr" &
    DAEMON_PID=$!
    trap 'kill "$DAEMON_PID" >/dev/null 2>&1 || true' EXIT
    run_timed "$scale" daemon_wait_for_health python3 - "$DAEMON_URL" <<'PY'
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
    run_timed "$scale" daemon_wait_for_hot_cache python3 - "$REPO/.trail/daemon.json" "$DAEMON_URL" <<'PY'
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
    run_http_timed "$scale" daemon_status "$DAEMON_URL" GET /v1/status
    run_without_daemon_endpoint "$scale" daemon_persisted_snapshot_status "$BIN" "$REPO" status
    run_without_daemon_endpoint "$scale" daemon_persisted_snapshot_record_clean "$BIN" "$REPO" record -m "scale persisted snapshot clean record"
    python3 - "$WORK/daemon-spawn.json" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "name": "daemonbot",
    "from_ref": "main",
    "materialize": False,
}))
PY
    run_http_timed "$scale" daemon_agent_spawn "$DAEMON_URL" POST /v1/lanes "$WORK/daemon-spawn.json"
    python3 - "$REPO" "$WORK/daemon-patch.json" "$scale" "$WORK/out/daemon_agent_spawn.stdout" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
base_change = json.loads(pathlib.Path(sys.argv[4]).read_text())["base_change"]
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
json.dump({"base_change": base_change, "message": "scale daemonbot", "edits": edits}, out.open("w"))
PY
    run_http_timed "$scale" daemon_agent_patch "$DAEMON_URL" POST /v1/lanes/daemonbot/patches "$WORK/daemon-patch.json"
    python3 - "$WORK/daemon-read.json" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "path": "README.md",
}))
PY
    run_http_timed "$scale" daemon_agent_read "$DAEMON_URL" POST /v1/lanes/daemonbot/read-file "$WORK/daemon-read.json"
    run_http_timed "$scale" daemon_agent_readiness "$DAEMON_URL" GET /v1/lanes/daemonbot/readiness
    python3 - "$WORK/daemon-merge.json" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "into": "main",
    "dry_run": True,
}))
PY
    run_http_timed "$scale" daemon_merge_dry_run "$DAEMON_URL" POST /v1/lanes/daemonbot/merge "$WORK/daemon-merge.json"
    run_timed "$scale" daemon_auto_cli_status "$BIN" --workspace "$REPO" --json status
    run_timed "$scale" daemon_auto_cli_agent_readiness "$BIN" --workspace "$REPO" --json lane readiness daemonbot
    run_timed "$scale" daemon_cli_status "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json status
    run_timed "$scale" daemon_cli_diff_dirty "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json diff --dirty
    run_timed "$scale" daemon_cli_mutate_worktree python3 - "$REPO" "$scale" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
files = int(sys.argv[2])
for i in range(max(1, min(25, files // 400))):
    idx = (i * 1871 + 7) % files
    d, f = divmod(idx, 100)
    path = root / f"pkg_{d:05d}" / f"module_{f:03d}.rs"
    with path.open("a") as fh:
        fh.write(f"\n// daemon CLI workspace record {i}\n")
PY
    run_without_daemon_endpoint "$scale" daemon_persisted_snapshot_diff_dirty "$BIN" "$REPO" diff --dirty
    run_timed "$scale" daemon_cli_record_dirty "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json record -m "scale daemon CLI dirty record"
    run_timed "$scale" daemon_cli_status_after_record "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json status
    run_timed "$scale" daemon_cli_agent_spawn "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane spawn daemonclibot --from main --no-materialize
    run_timed "$scale" daemon_cli_agent_list "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane list
    run_timed "$scale" daemon_cli_agent_show "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane show daemonclibot
    run_timed "$scale" daemon_cli_agent_workdir "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane workdir daemonclibot
    run_timed "$scale" daemon_cli_agent_claim "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane claim daemonclibot README.md --ttl-secs 120
    run_timed "$scale" daemon_cli_lease_acquire "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lease acquire daemonclibot --path pkg_00000/module_000.rs --ttl-secs 120
    DAEMON_CLI_LEASE_ID="$(python3 - "$WORK/out/daemon_cli_lease_acquire.stdout" <<'PY'
import json, pathlib, sys
print(json.loads(pathlib.Path(sys.argv[1]).read_text())["lease"]["lease_id"])
PY
)"
    run_timed "$scale" daemon_cli_lease_list "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lease list
    run_timed "$scale" daemon_cli_lease_release "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lease release "$DAEMON_CLI_LEASE_ID"
    run_timed "$scale" daemon_cli_session_start "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json session start daemonclibot --title "scale daemon CLI session"
    DAEMON_CLI_SESSION_ID="$(python3 - "$WORK/out/daemon_cli_session_start.stdout" <<'PY'
import json, pathlib, sys
print(json.loads(pathlib.Path(sys.argv[1]).read_text())["session"]["session_id"])
PY
)"
    run_timed "$scale" daemon_cli_session_current "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json session current daemonclibot
    run_timed "$scale" daemon_cli_session_list "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json session list --lane daemonclibot
    run_timed "$scale" daemon_cli_session_context "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json session context "$DAEMON_CLI_SESSION_ID" --limit 5
    run_timed "$scale" daemon_cli_approval_request "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json approvals request daemonclibot --action scale-check --summary "scale daemon CLI approval" --session "$DAEMON_CLI_SESSION_ID" --payload-json '{"scale":"daemon-cli"}'
    DAEMON_CLI_APPROVAL_ID="$(python3 - "$WORK/out/daemon_cli_approval_request.stdout" <<'PY'
import json, pathlib, sys
print(json.loads(pathlib.Path(sys.argv[1]).read_text())["approval"]["approval_id"])
PY
)"
    run_timed "$scale" daemon_cli_approval_list "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json approvals list --lane daemonclibot --status pending
    run_timed "$scale" daemon_cli_approval_decide "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json approvals decide "$DAEMON_CLI_APPROVAL_ID" --decision approved --reviewer scale
    run_timed "$scale" daemon_cli_session_end "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json session end "$DAEMON_CLI_SESSION_ID" --status completed
    run_timed "$scale" daemon_cli_agent_turn_start "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane turn start daemonclibot --from main --title "scale daemon CLI turn"
    DAEMON_CLI_TURN_ID="$(python3 - "$WORK/out/daemon_cli_agent_turn_start.stdout" <<'PY'
import json, pathlib, sys
print(json.loads(pathlib.Path(sys.argv[1]).read_text())["turn"]["turn_id"])
PY
)"
    run_timed "$scale" daemon_cli_agent_turn_message "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane turn message "$DAEMON_CLI_TURN_ID" --role user --text "scale daemon CLI turn message"
    run_timed "$scale" daemon_cli_agent_turn_event "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane turn event "$DAEMON_CLI_TURN_ID" --event-type checkpoint --payload-json '{"scale":"daemon-cli-turn"}'
    python3 - "$WORK/daemon-cli-turn-patch.json" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "message": "scale daemon CLI turn patch",
    "edits": [{
        "op": "write",
        "path": "README.md",
        "content": "# Trail daemon CLI turn scale\n",
    }],
}))
PY
    run_timed "$scale" daemon_cli_agent_turn_patch "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane turn apply-patch "$DAEMON_CLI_TURN_ID" --patch "$WORK/daemon-cli-turn-patch.json"
    run_timed "$scale" daemon_cli_agent_turn_show "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane turn show "$DAEMON_CLI_TURN_ID"
    run_timed "$scale" daemon_cli_agent_trace_start "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane trace start "$DAEMON_CLI_TURN_ID" --type tool_call --name daemon-cli-scale
    read -r DAEMON_CLI_SPAN_ID DAEMON_CLI_TRACE_ID < <(python3 - "$WORK/out/daemon_cli_agent_trace_start.stdout" <<'PY'
import json, pathlib, sys
span = json.loads(pathlib.Path(sys.argv[1]).read_text())["span"]
print(span["span_id"], span["trace_id"])
PY
)
    run_timed "$scale" daemon_cli_agent_trace_end "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane trace end "$DAEMON_CLI_SPAN_ID" --status completed
    run_timed "$scale" daemon_cli_agent_trace_list "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane trace list --turn "$DAEMON_CLI_TURN_ID" --limit 20
    run_timed "$scale" daemon_cli_agent_trace_summary "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane trace summary --trace-id "$DAEMON_CLI_TRACE_ID" --slowest 3
    run_timed "$scale" daemon_cli_agent_trace_show "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane trace show "$DAEMON_CLI_SPAN_ID"
    run_timed "$scale" daemon_cli_agent_turn_end "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane turn end "$DAEMON_CLI_TURN_ID" --status completed
    python3 - "$REPO" "$WORK/daemon-cli-patch.json" "$scale" "$WORK/out/daemon_cli_agent_turn_patch.stdout" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
base_change = json.loads(pathlib.Path(sys.argv[4]).read_text())["operation"]
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
json.dump({"base_change": base_change, "message": "scale daemonclibot", "edits": edits}, out.open("w"))
PY
    run_timed "$scale" daemon_cli_agent_patch "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane apply-patch daemonclibot --patch "$WORK/daemon-cli-patch.json"
    run_timed "$scale" daemon_cli_agent_read "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane read daemonclibot README.md
    run_timed "$scale" daemon_cli_agent_diff "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane diff daemonclibot
    run_timed "$scale" daemon_cli_why "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json why README.md:1
    run_timed "$scale" daemon_cli_history "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json history README.md
    run_timed "$scale" daemon_cli_code_from "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json code-from daemonclibot
    run_timed "$scale" daemon_cli_agent_readiness "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane readiness daemonclibot
    run_timed "$scale" daemon_cli_agent_contribution "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane contribution daemonclibot
    run_timed "$scale" daemon_cli_agent_gates "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane gates daemonclibot
    run_timed "$scale" daemon_cli_agent_events "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane events --lane daemonclibot
    run_timed "$scale" daemon_cli_agent_timeline "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane timeline daemonclibot --limit 20
    run_timed "$scale" daemon_cli_timeline "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json timeline --lane daemonclibot --limit 20
    run_timed "$scale" daemon_cli_agent_handoff "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane handoff daemonclibot
    run_timed "$scale" daemon_cli_merge_dry_run "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane merge daemonclibot --into main --dry-run
    run_timed "$scale" daemon_cli_lane_merge_queue_list "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane merge-queue list
    run_timed "$scale" daemon_cli_lane_merge_queue_run_empty "$BIN" --workspace "$REPO" --daemon-url "$DAEMON_URL" --json lane merge-queue run
    DAEMON_RSS="$(daemon_rss_bytes "$DAEMON_PID")"
    kill "$DAEMON_PID" >/dev/null 2>&1 || true
    wait "$DAEMON_PID" 2>/dev/null || true
    trap - EXIT
  fi

  run_timed "$scale" agent_spawn_headless "$BIN" --workspace "$REPO" --json lane spawn patchbot --from main --no-materialize
  run_timed "$scale" agent_read_headless "$BIN" --workspace "$REPO" --json lane read patchbot README.md
  python3 - "$REPO" "$WORK/patchbot.json" "$scale" "$WORK/out/agent_spawn_headless.stdout" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
base_change = json.loads(pathlib.Path(sys.argv[4]).read_text())["base_change"]
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
json.dump({"base_change": base_change, "message": "scale patchbot", "edits": edits}, out.open("w"))
PY
  run_timed "$scale" agent_apply_patch "$BIN" --workspace "$REPO" --json lane apply-patch patchbot --patch "$WORK/patchbot.json"
  python3 scripts/extract-path-index-performance.py \
    "$WORK/out/agent_apply_patch.stdout" patch "$WORK/path-index-patch.tsv"
  run_timed "$scale" agent_readiness "$BIN" --workspace "$REPO" --json lane readiness patchbot
  run_timed "$scale" merge_agent_dry_run "$BIN" --workspace "$REPO" --json lane merge patchbot --into main --direct --dry-run
  run_timed "$scale" merge_agent_apply "$BIN" --workspace "$REPO" --json lane merge patchbot --into main --direct

  run_timed "$scale" path_index_rename_spawn "$BIN" --workspace "$REPO" --json lane spawn pathindexrename --from main --no-materialize
  python3 - "$REPO" "$WORK/path-index-rename.json" "$WORK/out/path_index_rename_spawn.stdout" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
base_change = json.loads(pathlib.Path(sys.argv[3]).read_text())["base_change"]
module = pathlib.Path("pkg_00000/module_000.rs")
json.dump({
    "base_change": base_change,
    "message": "path-index delete/add and case rename",
    "edits": [
        {"op": "rename", "from": "README.md", "to": "readme.md"},
        {"op": "delete", "path": str(module)},
        {
            "op": "write",
            "path": str(module),
            "content": (root / module).read_text() + "\n// delete then add\n",
        },
    ],
}, out.open("w"))
PY
  run_timed "$scale" path_index_rename_patch "$BIN" --workspace "$REPO" --json lane apply-patch pathindexrename --patch "$WORK/path-index-rename.json"
  python3 scripts/extract-path-index-performance.py \
    "$WORK/out/path_index_rename_patch.stdout" rename_patch "$WORK/path-index-rename.tsv"

  mkdir -p "$EMPTY_REPO"
  run_timed "$scale" path_index_empty_init "$BIN" --workspace "$EMPTY_REPO" --json init
  run_timed "$scale" path_index_empty_spawn "$BIN" --workspace "$EMPTY_REPO" --json lane spawn pathindexempty --from main --no-materialize
  python3 - "$WORK/path-index-empty.json" "$WORK/out/path_index_empty_spawn.stdout" <<'PY'
import json, pathlib, sys
out = pathlib.Path(sys.argv[1])
base_change = json.loads(pathlib.Path(sys.argv[2]).read_text())["base_change"]
json.dump({
    "base_change": base_change,
    "message": "path-index first file",
    "edits": [{"op": "write", "path": "FIRST.md", "content": "first file\n"}],
}, out.open("w"))
PY
  run_timed "$scale" path_index_empty_patch "$BIN" --workspace "$EMPTY_REPO" --json lane apply-patch pathindexempty --patch "$WORK/path-index-empty.json"
  python3 scripts/extract-path-index-performance.py \
    "$WORK/out/path_index_empty_patch.stdout" empty_root_patch "$WORK/path-index-empty.tsv"

  run_timed "$scale" path_index_record_spawn "$BIN" --workspace "$REPO" --json lane spawn pathindexrecord --from main --paths README.md
  PATH_INDEX_RECORD_WORKDIR="$(python3 - "$WORK/out/path_index_record_spawn.stdout" <<'PY'
import json, pathlib, sys
value = json.loads(pathlib.Path(sys.argv[1]).read_text()).get("workdir")
print(value or "")
PY
)"
  if [ -z "$PATH_INDEX_RECORD_WORKDIR" ]; then
    printf 'bounded path-index record lane did not materialize a workdir\n' >&2
    exit 1
  fi
  run_timed "$scale" path_index_record_mutate python3 - "$PATH_INDEX_RECORD_WORKDIR/README.md" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
path.write_text(path.read_text() + "\npath-index bounded record\n")
PY
  run_timed "$scale" path_index_record "$BIN" --workspace "$REPO" --json lane record pathindexrecord -m "path-index bounded record"
  python3 scripts/extract-path-index-performance.py \
    "$WORK/out/path_index_record.stdout" record "$WORK/path-index-record.tsv"

  run_timed "$scale" agent_spawn_queuebot "$BIN" --workspace "$REPO" --json lane spawn queuebot --from main --no-materialize
  python3 - "$REPO" "$WORK/queuebot.json" "$scale" "$WORK/out/agent_spawn_queuebot.stdout" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = int(sys.argv[3])
base_change = json.loads(pathlib.Path(sys.argv[4]).read_text())["base_change"]
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
json.dump({"base_change": base_change, "message": "scale queuebot", "edits": edits}, out.open("w"))
PY
  run_timed "$scale" queuebot_apply_patch "$BIN" --workspace "$REPO" --json lane apply-patch queuebot --patch "$WORK/queuebot.json"
  run_timed "$scale" lane_merge_queue_add "$BIN" --workspace "$REPO" --json lane merge-queue add queuebot --into main
  run_timed "$scale" lane_merge_queue_run "$BIN" --workspace "$REPO" --json lane merge-queue run

  if [ "$RUN_MATERIALIZED" = "1" ]; then
    run_timed "$scale" agent_spawn_sparse "$BIN" --workspace "$REPO" --json lane spawn sparsebot --from main --paths README.md
    run_timed "$scale" agent_status_sparse "$BIN" --workspace "$REPO" --json lane status sparsebot
    run_timed "$scale" agent_read_sparse_nohydrate "$BIN" --workspace "$REPO" --json lane read sparsebot pkg_00000/module_001.rs
    run_timed "$scale" agent_read_sparse_hydrate "$BIN" --workspace "$REPO" --json lane read sparsebot pkg_00000/module_003.rs --hydrate
    run_timed "$scale" agent_read_sparse_hydrate_neighbors "$BIN" --workspace "$REPO" --json lane read sparsebot pkg_00000/module_002.rs --hydrate --include-neighbors
    run_timed "$scale" agent_sync_sparse_file "$BIN" --workspace "$REPO" --json lane sync-workdir sparsebot --paths pkg_00000/module_000.rs
    run_timed "$scale" agent_sync_sparse_dir "$BIN" --workspace "$REPO" --json lane sync-workdir sparsebot --paths pkg_00000
    run_timed "$scale" agent_status_sparse_hydrated "$BIN" --workspace "$REPO" --json lane status sparsebot
    run_timed "$scale" agent_spawn_materialized "$BIN" --workspace "$REPO" --json lane spawn matbot --from main --materialize
    run_timed "$scale" agent_status_materialized "$BIN" --workspace "$REPO" --json lane status matbot
    MATBOT_WORKDIR="$(python3 - "$WORK/out/agent_spawn_materialized.stdout" <<'PY'
import json, pathlib, sys
value = json.loads(pathlib.Path(sys.argv[1]).read_text()).get("workdir")
print(value or "")
PY
)"
    if [ -n "$MATBOT_WORKDIR" ]; then
      run_timed "$scale" mutate_agent_materialized_workdir python3 - "$MATBOT_WORKDIR" "$scale" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
files = int(sys.argv[2])
for i in range(max(1, min(25, files // 400))):
    idx = (i * 2371 + 13) % files
    d, f = divmod(idx, 100)
    path = root / f"pkg_{d:05d}" / f"module_{f:03d}.rs"
    with path.open("a") as fh:
        fh.write(f"\n// materialized record {i}\n")
PY
      run_timed "$scale" agent_record_materialized "$BIN" --workspace "$REPO" --json lane record matbot -m "scale materialized record"
      run_timed "$scale" agent_status_materialized_recorded "$BIN" --workspace "$REPO" --json lane status matbot
    fi
  fi

  run_timed "$scale" index_rebuild "$BIN" --workspace "$REPO" --json index rebuild
  run_timed "$scale" gc_dry_run "$BIN" --workspace "$REPO" --json gc --dry-run
  if [ "$RUN_BACKUP" = "1" ]; then
    run_timed "$scale" backup_create "$BIN" --workspace "$REPO" --json backup create --overwrite "$WORK/trail-backup"
    run_timed "$scale" backup_verify "$BIN" --workspace "$REPO" --json backup verify "$WORK/trail-backup"
  fi

  read -r source_file_count source_bytes < <(repo_source_bytes "$REPO")
  {
    printf 'source_file_count\t%s\n' "$source_file_count"
    printf 'source_bytes\t%s\n' "$source_bytes"
    printf 'sqlite_bytes\t%s\n' "$(sqlite_bytes "$REPO")"
    printf 'object_count\t%s\n' "$(object_count "$REPO")"
    object_kind_stats "$REPO" "repo"
    dbstat_bytes "$REPO" "repo"
    workdir_manifest_bytes "$REPO" "repo"
    if [ -d "$GIT_REPO/.trail" ]; then
      read -r git_source_file_count git_source_bytes < <(repo_source_bytes "$GIT_REPO")
      printf 'git_source_file_count\t%s\n' "$git_source_file_count"
      printf 'git_source_bytes\t%s\n' "$git_source_bytes"
      printf 'git_sqlite_bytes\t%s\n' "$(sqlite_bytes "$GIT_REPO")"
      printf 'git_object_count\t%s\n' "$(object_count "$GIT_REPO")"
      object_kind_stats "$GIT_REPO" "git"
      dbstat_bytes "$GIT_REPO" "git"
      workdir_manifest_bytes "$GIT_REPO" "git"
    fi
    if [ -f "$WORK/agent-git-metrics.tsv" ]; then
      cat "$WORK/agent-git-metrics.tsv"
    fi
    cat "$WORK/path-index-"*.tsv
    printf 'daemon_rss_bytes\t%s\n' "$DAEMON_RSS"
    du -sk "$REPO" "$REPO/.trail" 2>/dev/null | awk '{print "du_kb_" $2 "\t" $1}'
  } > "$WORK/metrics.tsv"

  printf 'scale=%s results=%s metrics=%s\n' "$scale" "$RESULTS" "$WORK/metrics.tsv"
done

printf 'run_root=%s\n' "$RUN_ROOT"

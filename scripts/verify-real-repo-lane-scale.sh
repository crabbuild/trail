#!/usr/bin/env bash
set -euo pipefail

# Deterministic, fail-closed real-repository qualification for concurrent Trail lanes.
# Task 12 owns the expensive Superset invocation; this script is also exercised by a fake
# Trail executable in scripts/test_verify_real_repo_lane_scale.py.

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)
FAULT_PROJECT_ROOT=${TRAIL_FAULT_PROJECT_ROOT:-$PROJECT_ROOT}
TRAIL_SCALE_COMMAND_RUNNER=${TRAIL_SCALE_COMMAND_RUNNER:-$SCRIPT_DIR/scale-command-runner.py}

fault_probe() {
  local scenario=${1:-}
  local test_target test_name durable_phase committed retry_result test_output test_count probe_dir probe_code
  local -a cargo_command
  case "$scenario" in
    after_reservation|after_materialization|after_association|after_reconciliation|after_marker|after_spawn_event)
      test_target=lane_initialization_faults
      test_name=identical_spawn_resumes_at_every_durable_crash_cut
      durable_phase=${scenario#after_}
      case "$scenario" in after_association|after_reconciliation|after_marker|after_spawn_event) committed=true ;; *) committed=false ;; esac
      retry_result=resumed_same_initialization
      ;;
    daemon_death)
      test_target=changed_path_ledger_daemon
      test_name=killed_daemon_is_replaced_and_full_reconciliation_captures_offline_change
      durable_phase=control committed=false retry_result=recovered_once ;;
    response_loss_after_association|response_loss_after_readiness)
      test_target=changed_path_ledger_daemon
      test_name=external_lane_spawn_ignores_daemon_response_delay_without_duplicate_fallback
      durable_phase=control committed=true retry_result=resumed_same_initialization ;;
    pid_reuse)
      test_target=changed_path_ledger_daemon
      test_name=forged_dead_process_identity_cannot_replace_a_live_observer_owner
      durable_phase=control committed=false retry_result=refused_without_mutation ;;
    lock_holder_crash)
      test_target=changed_path_ledger_daemon
      test_name=crash_after_persisting_ledger_owner_is_automatically_recovered
      durable_phase=control committed=false retry_result=recovered_once ;;
    policy_churn)
      test_target=changed_path_ledger_daemon
      test_name=live_policy_invalidation_self_restarts_and_reconciles
      durable_phase=control committed=false retry_result=recovered_once ;;
    filesystem_replacement)
      case "$(uname -s)" in
        Darwin) test_target=changed_path_ledger_macos; test_name=every_root_revalidation_failure_revokes_globally ;;
        Linux) test_target=changed_path_ledger_linux; test_name=owner_death_and_root_replacement_cannot_prove_clean ;;
        *) echo "filesystem replacement fault probe is unsupported on $(uname -s)" >&2; return 64 ;;
      esac
      durable_phase=control committed=false retry_result=refused_without_mutation ;;
    disk_full|permissions_failure|fsync_failure)
      test_target=lane_initialization_faults
      test_name=io_failures_never_advance_past_or_delete_the_durable_artifact
      durable_phase=reserved committed=false retry_result=refused_without_mutation ;;
    conflicting_lanes)
      test_target=e2e
      test_name=lane_merge_queue_pauses_on_conflict
      durable_phase=control committed=false retry_result=refused_without_mutation ;;
    dirty_git_export_refusal)
      # The main harness performs the real dirty-worktree refusal. This focused mapped
      # export test additionally proves that the mapped-delta policy is the selected path.
      test_target=lib
      test_name=db::merge::git_export::tests::mapped_git_export_requires_preexisting_clean_mapping
      durable_phase=control committed=false retry_result=refused_without_mutation ;;
    *) echo "unsupported fault scenario: $scenario" >&2; return 64 ;;
  esac
  [[ -f $TRAIL_SCALE_COMMAND_RUNNER ]] || { echo "bounded command runner is missing" >&2; return 1; }
  if [[ $test_target == lib ]]; then
    cargo_command=(cargo test --manifest-path "$FAULT_PROJECT_ROOT/Cargo.toml" -p trail "$test_name" --lib -- --exact --nocapture)
  else
    cargo_command=(cargo test --manifest-path "$FAULT_PROJECT_ROOT/Cargo.toml" -p trail --test "$test_target" "$test_name" -- --exact --nocapture)
  fi
  probe_dir=$(mktemp -d "${TMPDIR:-/tmp}/trail-scale-fault.XXXXXX") || return 1
  set +e
  python3 "$TRAIL_SCALE_COMMAND_RUNNER" --timeout-seconds "${TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS:-300}" \
    --max-output-bytes "${TRAIL_SCALE_MAX_OUTPUT_BYTES:-16777216}" --stdout "$probe_dir/stdout" \
    --stderr "$probe_dir/stderr" --rss "$probe_dir/rss" --meta "$probe_dir/meta" -- "${cargo_command[@]}"
  probe_code=$?
  set -e
  test_output=$(cat "$probe_dir/stdout" "$probe_dir/stderr")
  rm -rf -- "$probe_dir"
  if (( probe_code != 0 )); then
    printf '%s\n' "$test_output" >&2
    return 1
  fi
  printf '%s\n' "$test_output" >&2
  test_count=$(TRAIL_FAULT_TEST_OUTPUT="$test_output" python3 - "$test_name" <<'PY'
import os, re, sys
name=sys.argv[1]
text=os.environ["TRAIL_FAULT_TEST_OUTPUT"]
summaries=re.findall(r"test result: ok\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;", text)
named=len(re.findall(r"^test " + re.escape(name) + r" \.\.\. ok$", text, re.M))
if summaries != [("1","0","0")] or named != 1:
    raise SystemExit(f"focused fault probe did not execute exactly one named test: name={name} summaries={summaries} named={named}")
print(1)
PY
) || {
    echo "focused fault probe did not execute exactly one test" >&2
    return 1
  }
  python3 - "$scenario" "$durable_phase" "$committed" "$retry_result" "$test_target" "$test_name" "$test_count" <<'PY'
import json, sys
scenario, phase, committed, retry, target, name, count = sys.argv[1:]
print(json.dumps({
    "scenario": scenario, "expected_code": "PASS", "actual_code": "PASS",
    "durable_phase": phase, "committed": committed == "true",
    "retry_result": retry, "integrity_result": "focused_test_exit_0", "leaked_resource_count": 0,
    "initialization_id": "", "retry_initialization_id": "",
    "test_target": target, "test_name": name, "test_count": int(count),
}, sort_keys=True))
PY
}

if [[ ${1:-} == "--fault-probe" ]]; then
  [[ $# -eq 2 ]] || { echo "usage: $0 --fault-probe SCENARIO" >&2; exit 64; }
  fault_probe "$2"
  exit
fi
[[ $# -eq 0 ]] || { echo "usage: $0" >&2; exit 64; }

TRAIL_BIN=${TRAIL_BIN:-$PROJECT_ROOT/target/release/trail}
TRAIL_SCALE_REPO=${TRAIL_SCALE_REPO:-}
TRAIL_SCALE_LANES=${TRAIL_SCALE_LANES:-64}
TRAIL_SCALE_FILES_PER_LANE=${TRAIL_SCALE_FILES_PER_LANE:-50}
TRAIL_SCALE_CONCURRENCY=${TRAIL_SCALE_CONCURRENCY:-64}
TRAIL_SCALE_FAULT_PHASE=${TRAIL_SCALE_FAULT_PHASE:-all}
TRAIL_SCALE_LATENCY_CEILING_SECONDS=${TRAIL_SCALE_LATENCY_CEILING_SECONDS:-120}
TRAIL_SCALE_RUN_ID=${TRAIL_SCALE_RUN_ID:-scale-$(date -u +%Y%m%dT%H%M%SZ)-$$}
TRAIL_SCALE_GIT_REF=${TRAIL_SCALE_GIT_REF:-refs/heads/codex/trail-scale-$TRAIL_SCALE_RUN_ID}
TRAIL_SCALE_OUTPUT=${TRAIL_SCALE_OUTPUT:-}
TRAIL_SCALE_FAULT_DRIVER=${TRAIL_SCALE_FAULT_DRIVER:-$0}
TRAIL_SCALE_EXPECTED_BINARY_SHA256=${TRAIL_SCALE_EXPECTED_BINARY_SHA256:-}
TRAIL_SCALE_EXPECTED_SOURCE_COMMIT=${TRAIL_SCALE_EXPECTED_SOURCE_COMMIT:-}
TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256=${TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256:-}
TRAIL_SCALE_FAULT_ATTESTATION=${TRAIL_SCALE_FAULT_ATTESTATION:-}
TRAIL_SCALE_EXPECTED_FAULT_ATTESTATION_SHA256=${TRAIL_SCALE_EXPECTED_FAULT_ATTESTATION_SHA256:-}
TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS=${TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS:-300}
TRAIL_SCALE_MAX_OUTPUT_BYTES=${TRAIL_SCALE_MAX_OUTPUT_BYTES:-16777216}
TRAIL_SCALE_DISPOSABLE_WORKSPACE=${TRAIL_SCALE_DISPOSABLE_WORKSPACE:-}
TRAIL_SCALE_DISPOSABLE_OWNER_FILE=${TRAIL_SCALE_DISPOSABLE_OWNER_FILE:-}

die() { echo "verify-real-repo-lane-scale: $*" >&2; exit 64; }
is_uint() { [[ $1 =~ ^[0-9]+$ ]]; }
is_number() { [[ $1 =~ ^[0-9]+([.][0-9]+)?$ ]]; }

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else sha256sum "$1" | awk '{print $1}'; fi
}

device_id() { python3 - "$1" <<'PY'
import os, sys
path=os.path.abspath(sys.argv[1])
while not os.path.exists(path):
    parent=os.path.dirname(path)
    if parent == path: raise SystemExit(f"no existing ancestor for {sys.argv[1]}")
    path=parent
print(os.stat(path).st_dev)
PY
}

capture_resource_inventory() {
  local output=$1
  python3 - "$db_path" "$TRAIL_SCALE_REPO" "$TRAIL_SCALE_OUTPUT" "$output" <<'PY'
import hashlib, json, os, pathlib, sqlite3, stat, subprocess, sys
db_path, repo, output, destination=sys.argv[1:]

tables = {
    "lanes": ("lanes", ["lane_id", "name"]),
    "lane_branches": ("lane_branches", ["lane_id", "ref_name", "status", "workdir", "base_change", "head_change"]),
    "lane_refs": ("refs", ["name", "change_id", "root_id", "operation_id", "generation"]),
    "merge_queue": ("lane_merge_queue", ["queue_id", "lane_id", "target_ref", "status"]),
    "initializations": ("lane_initializations", ["initialization_id", "lane_name", "lane_id", "request_fingerprint", "phase", "workdir", "materialization_json"]),
    "workspace_views": ("workspace_views", ["view_id", "lane_id", "backend", "mountpoint", "source_upper", "generated_upper", "scratch_upper", "meta_dir", "journal_path", "status", "owner_pid"]),
    "leases": ("leases", ["lease_id", "lane_id", "ref_name", "path", "mode", "expires_at"]),
    "observer_owners": ("changed_path_observer_owners", ["scope_id", "lease_state", "daemon_pid"]),
}
resources={}
with sqlite3.connect(f"file:{pathlib.Path(db_path).resolve()}?mode=ro", uri=True) as db:
    db.row_factory=sqlite3.Row
    existing={row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    for key,(table,wanted) in tables.items():
        if table not in existing:
            raise SystemExit(f"required Trail inventory table is missing: {table}")
        columns={row[1] for row in db.execute(f"PRAGMA table_info({table})")}
        missing=set(wanted)-columns
        if missing:
            raise SystemExit(f"cannot inventory {table}: missing columns {sorted(missing)}")
        rows=[dict(row) for row in db.execute(f"SELECT {','.join(wanted)} FROM {table}")]
        if key == "lane_refs": rows=[row for row in rows if str(row.get("name","")).startswith("refs/lanes/")]
        resources[key]=sorted(rows, key=lambda row: json.dumps(row, sort_keys=True, separators=(",",":")))

lock_paths=[]; socket_paths=[]
trail_root=pathlib.Path(repo)/".trail"
if trail_root.exists():
    for base, dirs, files in os.walk(trail_root):
        for name in [*dirs, *files]:
            path=pathlib.Path(base)/name
            try: mode=path.lstat().st_mode
            except FileNotFoundError: continue
            relative=str(path.relative_to(trail_root))
            lowered=name.lower()
            if stat.S_ISSOCK(mode) or "socket" in lowered or lowered.endswith(".sock") or "tombstone" in lowered:
                socket_paths.append(relative)
            if lowered.endswith(".lock") or lowered == "lock" or lowered.startswith("lock."):
                lock_paths.append(relative)
resources["lock_paths"]=sorted(set(lock_paths))
resources["socket_paths"]=sorted(set(socket_paths))

roots=[str(pathlib.Path(repo).resolve()), str(pathlib.Path(output).resolve())]
mount_paths=[]
try:
    mount_output=subprocess.check_output(["mount"], text=True, errors="replace")
except (OSError, subprocess.CalledProcessError) as error:
    raise SystemExit(f"cannot inventory mounts: {error}")
for line in mount_output.splitlines():
    marker=" on "
    if marker not in line: continue
    mounted=line.split(marker,1)[1].split(" (",1)[0]
    if any(mounted == root or mounted.startswith(root+os.sep) for root in roots): mount_paths.append(mounted)
resources["mount_paths"]=sorted(set(mount_paths))

workdirs=[]
for row in resources["initializations"]:
    if row.get("workdir"): workdirs.append(row["workdir"])
for row in resources["workspace_views"]:
    if row.get("mountpoint"): workdirs.append(row["mountpoint"])
workdir_root=pathlib.Path(output)/"workdirs"
if workdir_root.is_dir(): workdirs.extend(str(path.resolve()) for path in workdir_root.iterdir())
resources["workdir_paths"]=sorted(set(workdirs))
materialization_journals=[]
journal_root=pathlib.Path(repo)/".trail/materialization-operations"
if journal_root.exists():
    for path in sorted(journal_root.rglob("*")):
        relative=str(path.relative_to(journal_root))
        info=path.lstat()
        if stat.S_ISREG(info.st_mode):
            kind="regular"; digest=hashlib.sha256(path.read_bytes()).hexdigest()
        elif stat.S_ISDIR(info.st_mode): kind="directory"; digest=None
        elif stat.S_ISLNK(info.st_mode): kind="symlink"; digest=None
        else: kind="other"; digest=None
        materialization_journals.append({"path":relative,"kind":kind,"size_bytes":info.st_size,"sha256":digest})
resources["materialization_journals"]=materialization_journals
pathlib.Path(destination).write_text(json.dumps({"schema_version":1,"resources":resources},sort_keys=True)+"\n",encoding="utf-8")
PY
}

snapshot_untracked() {
  local output=$1
  python3 - "$TRAIL_SCALE_REPO" "$output" <<'PY'
import hashlib, json, os, stat, subprocess, sys

repo, output = os.fsencode(sys.argv[1]), sys.argv[2]
raw_paths = subprocess.check_output(
    [b"git", b"-C", repo, b"ls-files", b"--others", b"--exclude-standard", b"-z", b"--"]
).split(b"\0")
entries = []
for raw_path in raw_paths:
    if not raw_path or raw_path == b".trail" or raw_path.startswith(b".trail/"):
        continue
    full_path = os.path.join(repo, raw_path)
    before = os.lstat(full_path)
    if stat.S_ISREG(before.st_mode):
        kind = "regular"
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(full_path, flags)
        try:
            opened = os.fstat(descriptor)
            if not stat.S_ISREG(opened.st_mode) or (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
                raise SystemExit(f"untracked path changed while snapshotting: {os.fsdecode(raw_path)!r}")
            digest = hashlib.sha256()
            while chunk := os.read(descriptor, 1024 * 1024):
                digest.update(chunk)
            hexdigest = digest.hexdigest()
        finally:
            os.close(descriptor)
    elif stat.S_ISLNK(before.st_mode):
        kind = "symlink"
        hexdigest = hashlib.sha256(os.readlink(full_path)).hexdigest()
    else:
        if stat.S_ISFIFO(before.st_mode): kind = "fifo"
        elif stat.S_ISSOCK(before.st_mode): kind = "socket"
        elif stat.S_ISBLK(before.st_mode): kind = "block_device"
        elif stat.S_ISCHR(before.st_mode): kind = "char_device"
        else: kind = "other"
        identity = f"{stat.S_IFMT(before.st_mode)}:{stat.S_IMODE(before.st_mode)}:{before.st_dev}:{before.st_ino}:{before.st_rdev}"
        hexdigest = hashlib.sha256(identity.encode("ascii")).hexdigest()
    after = os.lstat(full_path)
    if (after.st_dev, after.st_ino, stat.S_IFMT(after.st_mode)) != (before.st_dev, before.st_ino, stat.S_IFMT(before.st_mode)):
        raise SystemExit(f"untracked path changed while snapshotting: {os.fsdecode(raw_path)!r}")
    entries.append({"path": os.fsdecode(raw_path), "type": kind, "digest": hexdigest})
entries.sort(key=lambda entry: os.fsencode(entry["path"]))
with open(output, "w", encoding="utf-8") as stream:
    json.dump({"schema_version": 1, "algorithm": "sha256", "entries": entries}, stream, sort_keys=True)
    stream.write("\n")
PY
}

compare_untracked_snapshots() {
  python3 - "$1" "$2" <<'PY'
import json, sys
baseline = json.load(open(sys.argv[1], encoding="utf-8"))
final = json.load(open(sys.argv[2], encoding="utf-8"))
if baseline != final:
    before = {entry["path"]: entry for entry in baseline.get("entries", [])}
    after = {entry["path"]: entry for entry in final.get("entries", [])}
    added = sorted(after.keys() - before.keys())
    removed = sorted(before.keys() - after.keys())
    modified = sorted(path for path in before.keys() & after.keys() if before[path] != after[path])
    raise SystemExit(f"non-.trail untracked state changed: added={added} removed={removed} modified={modified}")
PY
}

capture_git_path_state() {
  local tree=$1 output=$2
  python3 - "$TRAIL_SCALE_REPO" "$tree" "$output" <<'PY'
import json, subprocess, sys
repo, tree, output=sys.argv[1:]
raw=subprocess.check_output(["git","-C",repo,"ls-tree","-rz","--full-tree",tree])
entries=[]
for record in raw.split(b"\0"):
    if not record: continue
    metadata, path=record.split(b"\t",1)
    mode, kind, object_id=metadata.decode("ascii").split()
    entries.append({"path":path.decode("utf-8","surrogateescape"),"mode":mode,"type":kind,"object":object_id})
entries.sort(key=lambda row: row["path"].encode("utf-8","surrogateescape"))
json.dump({"schema_version":1,"tree":subprocess.check_output(["git","-C",repo,"rev-parse",f"{tree}^{{tree}}"],text=True).strip(),"entries":entries},open(output,"w"),sort_keys=True)
open(output,"a").write("\n")
PY
}

capture_git_path_changes() {
  local baseline=$1 final=$2 output=$3
  python3 - "$TRAIL_SCALE_REPO" "$baseline" "$final" "$output" <<'PY'
import json, subprocess, sys
repo, baseline, final, output=sys.argv[1:]
raw=subprocess.check_output(["git","-C",repo,"diff-tree","--no-commit-id","--name-status","-r","-z","--no-renames",baseline,final])
parts=raw.split(b"\0"); changes=[]
for index in range(0,len(parts)-1,2):
    status=parts[index].decode("ascii"); path=parts[index+1].decode("utf-8","surrogateescape")
    changes.append({"status":status,"path":path})
changes.sort(key=lambda row:(row["path"].encode("utf-8","surrogateescape"),row["status"]))
tree=lambda value: subprocess.check_output(["git","-C",repo,"rev-parse",f"{value}^{{tree}}"],text=True).strip()
json.dump({"schema_version":1,"baseline_tree":tree(baseline),"final_tree":tree(final),"changes":changes},open(output,"w"),sort_keys=True)
open(output,"a").write("\n")
PY
}

[[ -n $TRAIL_SCALE_REPO ]] || die "TRAIL_SCALE_REPO is required"
[[ $TRAIL_SCALE_REPO == /* ]] || die "TRAIL_SCALE_REPO must be absolute"
[[ -d $TRAIL_SCALE_REPO && ! -L $TRAIL_SCALE_REPO ]] || die "TRAIL_SCALE_REPO must be a real directory"
TRAIL_SCALE_REPO=$(CDPATH= cd -- "$TRAIL_SCALE_REPO" && pwd -P)
git -C "$TRAIL_SCALE_REPO" rev-parse --is-inside-work-tree >/dev/null 2>&1 || die "TRAIL_SCALE_REPO must be a Git working tree"
[[ -d $TRAIL_SCALE_REPO/.trail/index ]] || die "TRAIL_SCALE_REPO must already be initialized by Trail"
[[ $TRAIL_BIN == /* ]] || TRAIL_BIN=$PROJECT_ROOT/$TRAIL_BIN
[[ -x $TRAIL_BIN && ! -d $TRAIL_BIN ]] || die "TRAIL_BIN must be an executable file"
for value_name in TRAIL_SCALE_LANES TRAIL_SCALE_FILES_PER_LANE TRAIL_SCALE_CONCURRENCY; do
  value=${!value_name}
  is_uint "$value" && (( value > 0 )) || die "$value_name must be a positive integer"
done
(( TRAIL_SCALE_LANES <= 128 )) || die "TRAIL_SCALE_LANES cannot exceed 128"
(( TRAIL_SCALE_FILES_PER_LANE <= 512 )) || die "TRAIL_SCALE_FILES_PER_LANE cannot exceed 512"
(( TRAIL_SCALE_LANES * TRAIL_SCALE_FILES_PER_LANE <= 65536 )) || die "total edits cannot exceed 65536"
(( TRAIL_SCALE_CONCURRENCY <= TRAIL_SCALE_LANES )) || die "TRAIL_SCALE_CONCURRENCY cannot exceed TRAIL_SCALE_LANES"
is_number "$TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS" || die "TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS must be positive"
python3 - "$TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS" <<'PY' || die "TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS must be positive"
import sys
raise SystemExit(0 if float(sys.argv[1]) > 0 else 1)
PY
is_uint "$TRAIL_SCALE_MAX_OUTPUT_BYTES" && (( TRAIL_SCALE_MAX_OUTPUT_BYTES > 0 )) || die "TRAIL_SCALE_MAX_OUTPUT_BYTES must be a positive integer"
is_number "$TRAIL_SCALE_LATENCY_CEILING_SECONDS" || die "TRAIL_SCALE_LATENCY_CEILING_SECONDS must be positive"
python3 - "$TRAIL_SCALE_LATENCY_CEILING_SECONDS" <<'PY' || die "TRAIL_SCALE_LATENCY_CEILING_SECONDS must be positive"
import sys
raise SystemExit(0 if float(sys.argv[1]) > 0 else 1)
PY
[[ $TRAIL_SCALE_RUN_ID =~ ^[A-Za-z0-9._-]+$ ]] || die "TRAIL_SCALE_RUN_ID contains unsafe characters"
[[ $TRAIL_SCALE_GIT_REF =~ ^refs/heads/codex/[A-Za-z0-9._/-]+$ ]] || die "TRAIL_SCALE_GIT_REF must be a dedicated refs/heads/codex/... ref"
case "$TRAIL_SCALE_FAULT_PHASE" in
  all|after_reservation|after_materialization|after_association|after_reconciliation|after_marker|after_spawn_event|daemon_death|response_loss_after_association|response_loss_after_readiness|pid_reuse|lock_holder_crash|policy_churn|filesystem_replacement|disk_full|permissions_failure|fsync_failure|conflicting_lanes|dirty_git_export_refusal) ;;
  *) die "TRAIL_SCALE_FAULT_PHASE is unsupported" ;;
esac
[[ -x $TRAIL_SCALE_FAULT_DRIVER ]] || die "TRAIL_SCALE_FAULT_DRIVER must be executable"
TRAIL_SCALE_FAULT_DRIVER=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$TRAIL_SCALE_FAULT_DRIVER")
candidate_harness_path=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$0")

if [[ -z $TRAIL_SCALE_OUTPUT ]]; then
  TRAIL_SCALE_OUTPUT=$TRAIL_SCALE_REPO/.trail/benchmarks/real-repo-lane-scale-$TRAIL_SCALE_RUN_ID
fi
[[ $TRAIL_SCALE_OUTPUT == /* ]] || die "TRAIL_SCALE_OUTPUT must be absolute"
TRAIL_SCALE_OUTPUT=$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$TRAIL_SCALE_OUTPUT")
[[ $TRAIL_SCALE_DISPOSABLE_WORKSPACE == 1 ]] || die "qualification requires an explicit disposable workspace contract"
[[ $TRAIL_SCALE_DISPOSABLE_OWNER_FILE == /* && -f $TRAIL_SCALE_DISPOSABLE_OWNER_FILE && ! -L $TRAIL_SCALE_DISPOSABLE_OWNER_FILE ]] || die "TRAIL_SCALE_DISPOSABLE_OWNER_FILE must be an absolute regular non-symlink file"
python3 - "$TRAIL_SCALE_DISPOSABLE_OWNER_FILE" "$TRAIL_SCALE_REPO" "$TRAIL_SCALE_OUTPUT" "$TRAIL_SCALE_RUN_ID" <<'PY' || die "disposable workspace owner contract is invalid"
import json,os,pathlib,platform,re,stat,subprocess,sys
owner,repo,output,run_id=sys.argv[1:]
repo_path=pathlib.Path(repo).resolve()
owner_path=pathlib.Path(owner)
expected_owner=repo_path/".trail"/"scale-disposable-owner.json"
if owner_path != expected_owner: raise SystemExit(f"owner file must use exact reserved path {expected_owner}")
descriptor=os.open(owner,os.O_RDONLY|getattr(os,"O_NOFOLLOW",0))
try:
    info=os.fstat(descriptor)
    if not stat.S_ISREG(info.st_mode): raise SystemExit("owner file is not regular")
    payload=os.read(descriptor,65537)
finally: os.close(descriptor)
if len(payload)>65536: raise SystemExit("owner file exceeds 64 KiB")
value=json.loads(payload.decode("utf-8"))
if set(value)!={"schema_version","kind","canonical_repo","disposable_repo","output","run_id"}: raise SystemExit("owner keys mismatch")
canonical=pathlib.Path(value["canonical_repo"]).resolve()
expected={"schema_version":1,"kind":"trail_scale_disposable_workspace","canonical_repo":str(canonical),"disposable_repo":str(repo_path),"output":str(pathlib.Path(output)),"run_id":run_id}
if value!=expected: raise SystemExit("owner values do not bind this run")
if canonical==repo_path or canonical in repo_path.parents or repo_path in canonical.parents:
    raise SystemExit("canonical and disposable repositories must not overlap")
def real_git_worktree(path):
    git_dir=path/".git"
    if not path.is_dir() or path.is_symlink() or not git_dir.is_dir() or git_dir.is_symlink(): return False
    try:
        inside=subprocess.check_output(["git","-C",str(path),"rev-parse","--is-inside-work-tree"],text=True,stderr=subprocess.DEVNULL,timeout=10).strip()
        top=pathlib.Path(subprocess.check_output(["git","-C",str(path),"rev-parse","--show-toplevel"],text=True,stderr=subprocess.DEVNULL,timeout=10).strip()).resolve()
    except (OSError,subprocess.CalledProcessError,subprocess.TimeoutExpired): return False
    return inside=="true" and top==path
if not real_git_worktree(canonical) or not real_git_worktree(repo_path):
    raise SystemExit("canonical and disposable repositories must be real Git worktrees with real .git directories")
if os.stat(canonical).st_dev != os.stat(repo_path).st_dev:
    raise SystemExit("canonical and disposable repositories must use the same filesystem device")
def filesystem_personality(path):
    device=subprocess.check_output(["df","-P",str(path)],text=True,stderr=subprocess.STDOUT,timeout=10).splitlines()[-1].split()[0]
    result=subprocess.check_output(["diskutil","info",device],text=True,stderr=subprocess.STDOUT,timeout=10)
    match=re.search(r"^\s*File System Personality:\s*(.+?)\s*$",result,re.M)
    if not match: raise SystemExit(f"cannot determine File System Personality for {path}")
    return match.group(1)
system=platform.system()
if system=="Darwin":
    try: personalities=[filesystem_personality(canonical),filesystem_personality(repo_path)]
    except (OSError,subprocess.CalledProcessError,subprocess.TimeoutExpired) as error:
        raise SystemExit(f"cannot authenticate APFS disposable workspace: {error}")
    if any("APFS" not in value.upper() for value in personalities):
        raise SystemExit(f"Darwin disposable workspace requires APFS for both repositories, got {personalities}")
elif system!="Linux": raise SystemExit(f"disposable workspace contract is unsupported on {system}")
PY
[[ ! -e $TRAIL_SCALE_OUTPUT ]] || die "TRAIL_SCALE_OUTPUT already exists"
python3 - "$TRAIL_SCALE_REPO" "$TRAIL_SCALE_OUTPUT" <<'PY' || die "unsafe output placement"
import os, pathlib, sys
repo=pathlib.Path(sys.argv[1]).resolve(); raw_output=pathlib.Path(sys.argv[2]); output=raw_output.resolve()
git=(repo/".git").resolve(); trail=(repo/".trail").resolve()
if output == repo or output == git or git in output.parents:
    raise SystemExit("output placement overlaps repository or Git metadata")
if repo in output.parents and not (output == trail or trail in output.parents):
    raise SystemExit("output placement inside repository must remain under .trail")
ancestor=raw_output
while True:
    if ancestor.exists() and ancestor.is_symlink(): raise SystemExit("output placement has a symlink ancestor")
    if ancestor.exists() or ancestor.parent==ancestor: break
    ancestor=ancestor.parent
PY
set +e
git -C "$TRAIL_SCALE_REPO" show-ref --verify --quiet "$TRAIL_SCALE_GIT_REF"
show_ref_code=$?
set -e
case "$show_ref_code" in
  0) die "dedicated Git ref already exists" ;;
  1) ;;
  *) die "could not inspect dedicated Git ref (git show-ref exit $show_ref_code)" ;;
esac

git -C "$TRAIL_SCALE_REPO" diff --quiet -- || die "tracked Git worktree must be clean before qualification"
git -C "$TRAIL_SCALE_REPO" diff --cached --quiet -- || die "Git index must be clean before qualification"
baseline_git_head=$(git -C "$TRAIL_SCALE_REPO" rev-parse HEAD)
baseline_git_branch=$(git -C "$TRAIL_SCALE_REPO" symbolic-ref --short -q HEAD) || die "detached Git HEAD is not supported"
baseline_git_index=$(git -C "$TRAIL_SCALE_REPO" rev-parse 'HEAD^{tree}')
db_path=$TRAIL_SCALE_REPO/.trail/index/trail.sqlite
[[ -f $db_path ]] || die "Trail database is missing at $db_path"

[[ $TRAIL_SCALE_EXPECTED_BINARY_SHA256 =~ ^[0-9a-f]{64}$ ]] || die "TRAIL_SCALE_EXPECTED_BINARY_SHA256 must be an exact lowercase SHA-256"
[[ $TRAIL_SCALE_EXPECTED_SOURCE_COMMIT =~ ^[0-9a-f]{40,64}$ ]] || die "TRAIL_SCALE_EXPECTED_SOURCE_COMMIT must be an exact source commit"
[[ $TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256 =~ ^[0-9a-f]{64}$ ]] || die "TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256 must be an exact lowercase SHA-256"
candidate_binary_sha256=$(sha256_file "$TRAIL_BIN")
[[ $candidate_binary_sha256 == "$TRAIL_SCALE_EXPECTED_BINARY_SHA256" ]] || die "candidate binary SHA-256 does not match TRAIL_SCALE_EXPECTED_BINARY_SHA256"
candidate_binary_size=$(stat -f %z "$TRAIL_BIN" 2>/dev/null || stat -c %s "$TRAIL_BIN")
trail_source_commit=$(git -C "$PROJECT_ROOT" rev-parse HEAD)
[[ $trail_source_commit == "$TRAIL_SCALE_EXPECTED_SOURCE_COMMIT" ]] || die "source commit does not match TRAIL_SCALE_EXPECTED_SOURCE_COMMIT"
source_status_baseline=$(git -C "$PROJECT_ROOT" status --porcelain=v1 --untracked-files=normal)
source_tracked_status_baseline=$(git -C "$PROJECT_ROOT" status --porcelain=v1 --untracked-files=no)
source_submodules_baseline=$(git -C "$PROJECT_ROOT" submodule status --recursive)
fault_driver_sha256=$(sha256_file "$TRAIL_SCALE_FAULT_DRIVER")
[[ $fault_driver_sha256 == "$TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256" ]] || die "fault driver SHA-256 does not match TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256"
fault_attestation_path=
fault_attestation_sha256=
if [[ $TRAIL_SCALE_FAULT_DRIVER == "$candidate_harness_path" ]]; then
  fault_qualification_kind=candidate_harness
  [[ -z $source_tracked_status_baseline ]] || die "candidate Cargo fault probes require a clean tracked source tree"
  if printf '%s\n' "$source_submodules_baseline" | grep -Eq '^[+-U]'; then die "candidate Cargo fault probes require clean submodules"; fi
  [[ -z $TRAIL_SCALE_FAULT_ATTESTATION && -z $TRAIL_SCALE_EXPECTED_FAULT_ATTESTATION_SHA256 ]] || die "candidate harness must not use external fault attestation"
else
  fault_qualification_kind=external_attestation
  [[ $TRAIL_SCALE_FAULT_ATTESTATION == /* && -f $TRAIL_SCALE_FAULT_ATTESTATION && ! -L $TRAIL_SCALE_FAULT_ATTESTATION ]] || die "external fault driver requires a regular absolute TRAIL_SCALE_FAULT_ATTESTATION"
  [[ $TRAIL_SCALE_EXPECTED_FAULT_ATTESTATION_SHA256 =~ ^[0-9a-f]{64}$ ]] || die "external fault driver requires an exact attestation SHA-256"
  fault_attestation_path=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$TRAIL_SCALE_FAULT_ATTESTATION")
  fault_attestation_sha256=$(sha256_file "$fault_attestation_path")
  [[ $fault_attestation_sha256 == "$TRAIL_SCALE_EXPECTED_FAULT_ATTESTATION_SHA256" ]] || die "fault attestation SHA-256 mismatch"
  python3 - "$fault_attestation_path" "$fault_driver_sha256" "$trail_source_commit" "$candidate_binary_sha256" <<'PY' || die "external fault driver attestation is invalid"
import json, sys
path, driver, source, binary = sys.argv[1:]
value=json.load(open(path, encoding="utf-8"))
expected={"schema_version","kind","fault_driver_sha256","source_commit","binary_sha256","test_contract"}
if set(value) != expected: raise SystemExit("attestation keys mismatch")
if value != {"schema_version":1,"kind":"external_fault_driver","fault_driver_sha256":driver,
             "source_commit":source,"binary_sha256":binary,"test_contract":"trail-task12-exact-one-v1"}:
    raise SystemExit("attestation does not bind the exact candidate/driver contract")
PY
fi
repo_device=$(device_id "$TRAIL_SCALE_REPO")
output_device=$(device_id "$TRAIL_SCALE_OUTPUT")
[[ $repo_device == "$output_device" ]] || die "TRAIL_SCALE_OUTPUT and repository must be on the same device for native-cow"

# Every rejection above and below this point uses only filesystem metadata, read-only Git,
# and SQLite mode=ro. No daemon-backed Trail command is allowed before this block passes.
head_file=$TRAIL_SCALE_REPO/.trail/HEAD
[[ -f $head_file && ! -L $head_file ]] || die "Trail HEAD file is missing or unsafe"
baseline_trail_branch=$(tr -d '\n' < "$head_file")
[[ $baseline_trail_branch =~ ^[A-Za-z0-9._/-]+$ ]] || die "Trail HEAD branch is invalid"
[[ $baseline_trail_branch == main ]] || die "Trail HEAD must be main"
baseline_trail_ref=refs/branches/$baseline_trail_branch
read -r baseline_trail_commit baseline_trail_root < <(python3 - "$db_path" "$baseline_trail_ref" "$baseline_git_head" "$TRAIL_SCALE_LANES" <<'PY'
import sqlite3, sys
db_path, ref_name, head, count=sys.argv[1],sys.argv[2],sys.argv[3],int(sys.argv[4])
with sqlite3.connect(f"file:{db_path}?mode=ro", uri=True) as db:
    current=db.execute("SELECT change_id,root_id FROM refs WHERE name=?",(ref_name,)).fetchone()
    if not current or any(not isinstance(value,str) or not value or any(c.isspace() for c in value) for value in current):
        raise SystemExit("Trail HEAD ref is missing or malformed")
    root=current[1]
    mapped=db.execute("SELECT 1 FROM git_mappings WHERE crab_change=? AND crab_root=? AND git_head=? AND git_dirty=0 LIMIT 1",(current[0],root,head)).fetchone()
    if not mapped: raise SystemExit("mapped_delta baseline is not mapped to current Git HEAD")
    planned=[f"scale-{index:04d}" for index in range(count)]
    placeholders=','.join('?'*len(planned))
    collisions=[row[0] for row in db.execute(f"SELECT name FROM lanes WHERE name IN ({placeholders})",planned)]
    collisions += [row[0] for row in db.execute(f"SELECT lane_name FROM lane_initializations WHERE lane_name IN ({placeholders})",planned)]
    lane_refs=[f"refs/lanes/{name}" for name in planned]
    collisions += [row[0] for row in db.execute(f"SELECT name FROM refs WHERE name IN ({placeholders})",lane_refs)]
    collisions=sorted(set(collisions))
    if collisions: raise SystemExit(f"planned lane already exists: {collisions}")
    pending=[tuple(row) for row in db.execute("SELECT queue_id,status FROM lane_merge_queue WHERE status NOT IN ('merged','failed','cancelled') ORDER BY queue_id")]
    if pending: raise SystemExit(f"nonterminal merge queue work exists: {pending}")
    print(*current)
PY
) || die "mapped_delta/lane/queue preflight failed"

mkdir -p "$TRAIL_SCALE_OUTPUT/commands" "$TRAIL_SCALE_OUTPUT/rows" "$TRAIL_SCALE_OUTPUT/workdirs" "$TRAIL_SCALE_OUTPUT/manifests"
original_candidate_binary=$TRAIL_BIN
candidate_binary_executable=$TRAIL_SCALE_OUTPUT/candidate-trail
python3 - "$original_candidate_binary" "$candidate_binary_executable" "$candidate_binary_sha256" <<'PY' || die "could not create immutable candidate binary copy"
import hashlib, os, stat, sys
source,destination,expected=sys.argv[1:]
fd=os.open(source,os.O_RDONLY|getattr(os,"O_NOFOLLOW",0))
try:
    info=os.fstat(fd)
    if not stat.S_ISREG(info.st_mode): raise SystemExit("candidate is not regular")
    chunks=[]; digest=hashlib.sha256()
    while chunk:=os.read(fd,1024*1024): chunks.append(chunk); digest.update(chunk)
finally: os.close(fd)
if digest.hexdigest()!=expected: raise SystemExit("candidate changed before immutable copy")
out=os.open(destination,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o500)
try:
    for chunk in chunks:
        view=memoryview(chunk)
        while view: view=view[os.write(out,view):]
    os.fsync(out)
finally: os.close(out)
PY
TRAIL_BIN=$candidate_binary_executable
[[ $(sha256_file "$TRAIL_BIN") == "$candidate_binary_sha256" ]] || die "immutable candidate binary digest mismatch"
candidate_binary_version=$("$TRAIL_BIN" --version 2>&1) || die "immutable candidate binary --version failed"
[[ -n $candidate_binary_version && $candidate_binary_version != *$'\n'* ]] || die "candidate binary version must be one non-empty line"
fault_driver_executed_evidence_path=fault-driver-executed
fault_driver_executable=$TRAIL_SCALE_OUTPUT/$fault_driver_executed_evidence_path
python3 - "$TRAIL_SCALE_FAULT_DRIVER" "$fault_driver_executable" "$fault_driver_sha256" <<'PY' || die "could not create exact immutable fault-driver evidence copy"
import hashlib, os, stat, sys
source, destination, expected=sys.argv[1:]
flags=os.O_RDONLY|getattr(os,"O_NOFOLLOW",0)
descriptor=os.open(source,flags)
try:
    info=os.fstat(descriptor)
    if not stat.S_ISREG(info.st_mode): raise SystemExit("fault driver is not regular")
    chunks=[]; digest=hashlib.sha256()
    while True:
        chunk=os.read(descriptor,1024*1024)
        if not chunk: break
        chunks.append(chunk); digest.update(chunk)
finally:
    os.close(descriptor)
if digest.hexdigest()!=expected: raise SystemExit("fault driver changed before immutable copy")
output=os.open(destination,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o500)
try:
    for chunk in chunks:
        view=memoryview(chunk)
        while view:
            written=os.write(output,view)
            view=view[written:]
    os.fsync(output)
finally:
    os.close(output)
os.chmod(destination,0o500)
PY
[[ $(sha256_file "$fault_driver_executable") == "$fault_driver_sha256" ]] || die "immutable fault-driver evidence digest mismatch"
if [[ $fault_qualification_kind == external_attestation ]]; then
  cp -p -- "$fault_attestation_path" "$TRAIL_SCALE_OUTPUT/fault-attestation.json"
fi
snapshot_untracked "$TRAIL_SCALE_OUTPUT/baseline-untracked.json"
capture_resource_inventory "$TRAIL_SCALE_OUTPUT/baseline-resources.json"
capture_git_path_state "$baseline_git_head" "$TRAIL_SCALE_OUTPUT/baseline-path-state.json"
RESULT_COLUMNS=$'command_id\tphase\tlane\twall_seconds\tpeak_rss_bytes\texit_code\tcommitted\tretry_of'
LANE_COLUMNS=$'lane\tinitialization_id\tretry_initialization_id\trequest_fingerprint\tretry_request_fingerprint\tworkdir_mode\tworkdir\tedit_count\trecorded_path_count\tisolation_unexpected_count\tlogical_bytes\tallocated_bytes\texclusive_bytes\tclone_count\tshared_physical_bytes\tshared_extent_bytes\tchanged_bytes\tphysical_sharing\tphysical_sharing_evidence'
FAULT_COLUMNS=$'scenario\texpected_code\tactual_code\tdurable_phase\tcommitted\tretry_result\tintegrity_result\tleaked_resource_count\tinitialization_id\tretry_initialization_id\tevidence_command_id\tevidence_kind\tsource_commit\tbinary_sha256\tbinary_exercised\ttest_target\ttest_name\ttest_count'
printf '%s\n' "$RESULT_COLUMNS" > "$TRAIL_SCALE_OUTPUT/results.tsv"
printf '%s\n' "$LANE_COLUMNS" > "$TRAIL_SCALE_OUTPUT/lanes.tsv"
printf '%s\n' "$FAULT_COLUMNS" > "$TRAIL_SCALE_OUTPUT/faults.tsv"

owned_lanes_dir=$TRAIL_SCALE_OUTPUT/owned-lanes
mkdir -p "$owned_lanes_dir"
dirty_probe_path=
dirty_probe_backup=
dirty_probe_backup_dir=
dirty_probe_marker=
restore_dirty_probe() {
  [[ -n $dirty_probe_path && -n $dirty_probe_backup && -f $dirty_probe_backup ]] || return 0
  local expected=$dirty_probe_backup_dir/expected
  cp -p -- "$dirty_probe_backup" "$expected"
  printf '%s' "$dirty_probe_marker" >> "$expected"
  if ! cmp -s -- "$dirty_probe_path" "$expected"; then
    echo "dirty refusal probe changed unexpectedly; refusing to overwrite tracked path $dirty_probe_path" >&2
    return 1
  fi
  cp -p -- "$dirty_probe_backup" "$dirty_probe_path"
  rm -rf -- "$dirty_probe_backup_dir"
  dirty_probe_path=
  dirty_probe_backup=
  dirty_probe_backup_dir=
  dirty_probe_marker=
}
verify_owned_lane_marker() {
  local marker=$1
  python3 - "$db_path" "$marker" <<'PY'
import json, pathlib, sqlite3, sys
db_path, marker=sys.argv[1:]
expected=json.load(open(marker, encoding="utf-8"))
keys={"lane","lane_id","initialization_id","request_fingerprint","workdir","ref_name"}
if set(expected) != keys: raise SystemExit("owned lane marker schema mismatch")
with sqlite3.connect(f"file:{pathlib.Path(db_path).resolve()}?mode=ro",uri=True) as db:
    row=db.execute("SELECT l.name,l.lane_id,i.initialization_id,i.request_fingerprint,i.workdir,b.ref_name FROM lanes l JOIN lane_initializations i ON i.lane_id=l.lane_id JOIN lane_branches b ON b.lane_id=l.lane_id WHERE l.name=?",(expected["lane"],)).fetchone()
actual=dict(zip(["lane","lane_id","initialization_id","request_fingerprint","workdir","ref_name"],row)) if row else None
if actual != expected:
    raise SystemExit(f"stable ownership changed for {expected['lane']}: expected={expected} actual={actual}")
PY
}
cleanup_on_failure() {
  local status=$?
  if (( status != 0 )); then
    [[ -z ${staging_ref:-} ]] || git -C "$TRAIL_SCALE_REPO" update-ref -d "$staging_ref" >/dev/null 2>&1 || true
    restore_dirty_probe || true
    for marker in "$owned_lanes_dir"/*; do
      [[ -f $marker ]] || continue
      lane=${marker##*/}
      if verify_owned_lane_marker "$marker"; then
        cleanup_prefix=$TRAIL_SCALE_OUTPUT/commands/failure-cleanup-${lane}
        python3 "$SCRIPT_DIR/scale-command-runner.py" --timeout-seconds "$TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS" \
          --max-output-bytes "$TRAIL_SCALE_MAX_OUTPUT_BYTES" --stdout "$cleanup_prefix.stdout" \
          --stderr "$cleanup_prefix.stderr" --rss "$cleanup_prefix.rss" --meta "$cleanup_prefix.supervisor.json" -- \
          "$TRAIL_BIN" --workspace "$TRAIL_SCALE_REPO" --json lane rm "$lane" >/dev/null 2>&1 || true
      else
        echo "refusing failure cleanup for $lane because stable ownership changed" >&2
      fi
    done
    echo "partial evidence retained at $TRAIL_SCALE_OUTPUT" >&2
  fi
  return "$status"
}
trap cleanup_on_failure EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

trail() { "$TRAIL_BIN" --workspace "$TRAIL_SCALE_REPO" --json "$@"; }
now_seconds() { python3 - <<'PY'
import time
print(f"{time.monotonic():.9f}")
PY
}

run_command() {
  local command_id=$1 phase=$2 lane=$3 committed=$4 retry_of=$5 expected_code=$6
  shift 6
  local stdout="$TRAIL_SCALE_OUTPUT/commands/$command_id.stdout"
  local stderr="$TRAIL_SCALE_OUTPUT/commands/$command_id.stderr"
  local json_file="$TRAIL_SCALE_OUTPUT/commands/$command_id.json"
  local rss_file="$TRAIL_SCALE_OUTPUT/commands/$command_id.rss"
  local row_file="$TRAIL_SCALE_OUTPUT/rows/$command_id.tsv"
  local meta_file="$TRAIL_SCALE_OUTPUT/commands/$command_id.supervisor.json"
  local elapsed peak actual_code normalized_code
  if [[ ${1:-} == trail ]]; then
    shift
    set -- "$TRAIL_BIN" --workspace "$TRAIL_SCALE_REPO" --json "$@"
  fi
  set +e
  python3 "$SCRIPT_DIR/scale-command-runner.py" \
    --timeout-seconds "$TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS" \
    --max-output-bytes "$TRAIL_SCALE_MAX_OUTPUT_BYTES" \
    --stdout "$stdout" --stderr "$stderr" --rss "$rss_file" --meta "$meta_file" -- "$@"
  actual_code=$?
  set -e
  read -r elapsed peak < <(python3 - "$meta_file" <<'PY'
import json,sys
v=json.load(open(sys.argv[1])); print(f"{v['elapsed_seconds']:.6f}",v["peak_process_tree_rss_bytes"])
PY
  )
  python3 - "$command_id" "$phase" "$lane" "$actual_code" "$expected_code" "$stdout" "$stderr" "$json_file" <<'PY'
import json, pathlib, sys
command_id, phase, lane, actual, expected, stdout, stderr, output = sys.argv[1:]
raw_stdout = pathlib.Path(stdout).read_text(encoding="utf-8", errors="replace")
raw_stderr = pathlib.Path(stderr).read_text(encoding="utf-8", errors="replace")
payload = None
for raw in (raw_stdout, raw_stderr):
    try:
        payload = json.loads(raw)
        break
    except json.JSONDecodeError:
        pass
pathlib.Path(output).write_text(json.dumps({
    "schema_version": 1, "command_id": command_id, "phase": phase, "lane": lane,
    "actual_exit_code": int(actual), "expected_exit_code": expected, "payload": payload,
}, sort_keys=True) + "\n", encoding="utf-8")
PY
  if [[ $expected_code == any-nonzero ]]; then
    (( actual_code != 0 )) || { echo "$command_id unexpectedly succeeded" >&2; return 1; }
    normalized_code=0
  elif [[ $expected_code =~ ^[0-9]+$ ]]; then
    (( actual_code == expected_code )) || { echo "$command_id exit $actual_code, expected $expected_code" >&2; return 1; }
    normalized_code=0
  else
    echo "invalid expected exit code for $command_id" >&2
    return 1
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$command_id" "$phase" "$lane" "$elapsed" "$peak" "$normalized_code" "$committed" "$retry_of" > "$row_file"
}

json_payload_field() {
  local file=$1 path=$2
  python3 - "$file" "$path" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))["payload"]
for part in sys.argv[2].split("."):
    if not isinstance(value, dict) or part not in value:
        raise SystemExit(f"missing JSON field {sys.argv[2]} in {sys.argv[1]}")
    value = value[part]
if isinstance(value, bool): print("true" if value else "false")
elif value is None: print("")
elif isinstance(value, (dict, list)): print(json.dumps(value, sort_keys=True))
else: print(value)
PY
}

json_payload_paths() {
  local file=$1 path=$2 output=$3
  python3 - "$file" "$path" "$output" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))["payload"]
for part in sys.argv[2].split("."):
    if not isinstance(value, dict) or part not in value: raise SystemExit(f"missing field {sys.argv[2]}")
    value = value[part]
if not isinstance(value, list): raise SystemExit(f"{sys.argv[2]} is not a list")
paths=[]
for item in value:
    if not isinstance(item, dict) or not isinstance(item.get("path"), str): raise SystemExit("path list row is malformed")
    paths.append(item["path"])
if len(paths) != len(set(paths)): raise SystemExit("path list contains duplicates")
open(sys.argv[3], "w", encoding="utf-8").write("".join(p+"\n" for p in sorted(paths)))
PY
}

run_command baseline-status baseline "" true "" 0 trail status
[[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/baseline-status.json" head.name) == "$baseline_trail_ref" ]] || die "Trail head changed after preflight"
[[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/baseline-status.json" head.change_id) == "$baseline_trail_commit" ]] || die "Trail commit changed after preflight"
[[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/baseline-status.json" head.root_id) == "$baseline_trail_root" ]] || die "Trail root changed after preflight"
capture_resource_inventory "$TRAIL_SCALE_OUTPUT/runtime-resources.json"
db_bytes_before=$(stat -f %z "$db_path" 2>/dev/null || stat -c %s "$db_path")
file_bytes() { [[ -f $1 ]] && (stat -f %z "$1" 2>/dev/null || stat -c %s "$1") || echo 0; }
disk_bytes() { du -sk "$1" | awk '{printf "%.0f\n", $1*1024}'; }
db_wal_bytes_before=$(file_bytes "$db_path-wal")
db_shm_bytes_before=$(file_bytes "$db_path-shm")
repo_disk_bytes_before=$(disk_bytes "$TRAIL_SCALE_REPO")
trail_disk_bytes_before=$(disk_bytes "$TRAIL_SCALE_REPO/.trail")
observer_log_bytes_before=$(find "$TRAIL_SCALE_REPO/.trail" -type f \( -name '*observer*.log' -o -name '*changed-path*.log' \) -exec stat -f %z {} \; 2>/dev/null | awk '{sum+=$1} END{print sum+0}')
if [[ -z $observer_log_bytes_before ]]; then
  observer_log_bytes_before=$(find "$TRAIL_SCALE_REPO/.trail" -type f \( -name '*observer*.log' -o -name '*changed-path*.log' \) -printf '%s\n' 2>/dev/null | awk '{sum+=$1} END{print sum+0}')
fi

expected_paths_file=$TRAIL_SCALE_OUTPUT/expected-paths.txt
python3 - "$TRAIL_SCALE_REPO" "$TRAIL_SCALE_OUTPUT/manifests" "$expected_paths_file" "$TRAIL_SCALE_LANES" "$TRAIL_SCALE_FILES_PER_LANE" <<'PY' || die "repository lacks enough safe existing regular tracked files"
import os, pathlib, subprocess, sys
repo, manifests, expected, lanes, files=sys.argv[1],pathlib.Path(sys.argv[2]),pathlib.Path(sys.argv[3]),int(sys.argv[4]),int(sys.argv[5])
paths=[]
for raw in subprocess.check_output(["git","-C",repo,"ls-files","-z","--"]).split(b"\0"):
    if not raw or b"\n" in raw or b"\r" in raw or b"\t" in raw: continue
    relative=os.fsdecode(raw); target=pathlib.Path(repo)/relative
    if target.is_file() and not target.is_symlink(): paths.append(relative)
paths=sorted(set(paths),key=os.fsencode)
needed=lanes*files
if len(paths)<needed: raise SystemExit(f"need {needed} regular tracked files, found {len(paths)}")
selected=paths[:needed]
expected.write_text("".join(path+"\n" for path in sorted(selected,key=os.fsencode)),encoding="utf-8")
for index in range(lanes):
    lane=f"scale-{index:04d}"; allocation=selected[index*files:(index+1)*files]
    (manifests/f"{lane}.expected.txt").write_text("".join(path+"\n" for path in sorted(allocation,key=os.fsencode)),encoding="utf-8")
PY

spawn_one() {
  local index=$1 lane workdir
  lane=$(printf 'scale-%04d' "$index")
  workdir=$TRAIL_SCALE_OUTPUT/workdirs/$lane
  run_command "spawn-$index" spawn "$lane" true "" 0 trail lane spawn "$lane" --from main --workdir-mode native-cow --workdir "$workdir"
  if [[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/spawn-$index.json" resumed) == false ]]; then
    python3 - "$TRAIL_SCALE_OUTPUT/commands/spawn-$index.json" "$owned_lanes_dir/$lane" <<'PY'
import json, sys
payload=json.load(open(sys.argv[1],encoding="utf-8"))["payload"]
keys={"lane":payload.get("lane",payload.get("name")),"lane_id":payload.get("lane_id"),
      "initialization_id":payload.get("initialization_id"),"request_fingerprint":payload.get("request_fingerprint"),
      "workdir":payload.get("workdir"),"ref_name":payload.get("ref_name")}
if not keys["lane"]:
    keys["lane"]=payload.get("ref_name","").removeprefix("refs/lanes/")
if any(not isinstance(value,str) or not value for value in keys.values()):
    raise SystemExit("spawn did not return complete stable ownership")
open(sys.argv[2],"w",encoding="utf-8").write(json.dumps(keys,sort_keys=True)+"\n")
PY
    verify_owned_lane_marker "$owned_lanes_dir/$lane"
  else
    echo "$lane initial spawn did not prove this process created the lane" >&2
    return 1
  fi
}

run_parallel_indices() {
  local function_name=$1 index pid failed=0
  local -a pids=()
  for ((index=0; index<TRAIL_SCALE_LANES; index++)); do
    "$function_name" "$index" &
    pids+=("$!")
    if (( ${#pids[@]} == TRAIL_SCALE_CONCURRENCY || index + 1 == TRAIL_SCALE_LANES )); then
      for pid in "${pids[@]}"; do wait "$pid" || failed=1; done
      pids=()
    fi
  done
  (( failed == 0 ))
}

run_parallel_indices spawn_one || die "one or more concurrent lane spawns failed"
owned_lane_count=0
for marker in "$owned_lanes_dir"/*; do [[ -f $marker ]] && owned_lane_count=$((owned_lane_count + 1)); done
(( owned_lane_count == TRAIL_SCALE_LANES )) || die "not every planned lane is proven run-owned"
for ((index=0; index<TRAIL_SCALE_LANES; index++)); do
  lane=$(printf 'scale-%04d' "$index")
  workdir=$TRAIL_SCALE_OUTPUT/workdirs/$lane
  run_command "spawn-retry-$index" spawn_retry "$lane" true "spawn-$index" 0 trail lane spawn "$lane" --from main --workdir-mode native-cow --workdir "$workdir"
done

lane_workload() {
  local index=$1 lane workdir lane_manifest actual_status actual_record relative target
  lane=$(printf 'scale-%04d' "$index")
  workdir=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/spawn-$index.json" workdir)
  [[ $workdir == "$TRAIL_SCALE_OUTPUT/workdirs/$lane" ]] || { echo "$lane returned unexpected workdir $workdir" >&2; return 1; }
  [[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/spawn-$index.json" requested_workdir_mode) == native-cow ]] || return 1
  [[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/spawn-$index.json" workdir_mode) == native-cow ]] || return 1
  [[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/spawn-$index.json" materialization.copied_files) == 0 ]] || return 1
  lane_manifest=$TRAIL_SCALE_OUTPUT/manifests/$lane.expected.txt
  while IFS= read -r relative; do
    target=$workdir/$relative
    [[ -f $target && ! -L $target ]] || { echo "$lane selected path is not a regular materialized file: $relative" >&2; return 1; }
    printf '\ntrail-scale run=%s lane=%s path=%s\n' "$TRAIL_SCALE_RUN_ID" "$lane" "$relative" >> "$target"
  done < "$lane_manifest"
  run_command "status-$index" status "$lane" true "" 0 trail lane status "$lane"
  actual_status=$TRAIL_SCALE_OUTPUT/manifests/$lane.status.txt
  json_payload_paths "$TRAIL_SCALE_OUTPUT/commands/status-$index.json" workdir_changed_paths "$actual_status"
  cmp -s "$lane_manifest" "$actual_status" || { echo "$lane isolation manifest mismatch" >&2; return 1; }
  run_command "space-$index" space "$lane" true "" 0 trail lane space "$lane"
  python3 - "$TRAIL_SCALE_OUTPUT/commands/space-$index.json" <<'PY' || return 1
import json,sys
v=json.load(open(sys.argv[1]))["payload"]
required={"clone_count","shared_physical_bytes","lane_exclusive_physical_bytes","shared_extent_bytes","changed_since_baseline_bytes","physical_sharing","physical_sharing_evidence","physical_accounting","filesystem_allocated_bytes","logical_visible_bytes"}
missing=required-set(v) if isinstance(v,dict) else required
if missing: raise SystemExit(f"incomplete native-COW space evidence: {sorted(missing)}")
for key in {"clone_count","shared_physical_bytes","lane_exclusive_physical_bytes","filesystem_allocated_bytes","logical_visible_bytes"}:
    if isinstance(v[key],bool) or not isinstance(v[key],int) or v[key]<0: raise SystemExit(f"invalid native-COW metric {key}")
if v["clone_count"]<=0 or isinstance(v["changed_since_baseline_bytes"],bool) or not isinstance(v["changed_since_baseline_bytes"],int) or v["changed_since_baseline_bytes"]<=0:
    raise SystemExit("native-COW evidence must include clones and positive changed bytes")
if not isinstance(v["physical_sharing_evidence"],str) or not v["physical_sharing_evidence"]: raise SystemExit("native-COW sharing evidence is missing")
if v["physical_sharing"]=="verified":
    if isinstance(v["shared_extent_bytes"],bool) or not isinstance(v["shared_extent_bytes"],int) or v["shared_extent_bytes"]<=0 or v["shared_physical_bytes"]<=0:
        raise SystemExit("verified sharing requires positive shared extent/physical bytes")
elif v["physical_sharing"]=="unknown":
    if v["shared_extent_bytes"] is not None or v["shared_physical_bytes"]!=0 or v["lane_exclusive_physical_bytes"]!=0 or "unattributed" not in v["physical_accounting"]:
        raise SystemExit("unknown sharing must remain null/zero and explicitly unattributed")
else: raise SystemExit("native-COW physical sharing is neither verified nor honestly unknown")
PY
  run_command "record-preview-$index" record_preview "$lane" false "" 0 trail lane record "$lane" --preview
  json_payload_paths "$TRAIL_SCALE_OUTPUT/commands/record-preview-$index.json" changed_paths "$TRAIL_SCALE_OUTPUT/manifests/$lane.preview.txt"
  cmp -s "$lane_manifest" "$TRAIL_SCALE_OUTPUT/manifests/$lane.preview.txt" || return 1
  run_command "record-$index" record "$lane" true "" 0 trail lane record "$lane" -m "scale record $lane"
  actual_record=$TRAIL_SCALE_OUTPUT/manifests/$lane.record.txt
  json_payload_paths "$TRAIL_SCALE_OUTPUT/commands/record-$index.json" changed_paths "$actual_record"
  cmp -s "$lane_manifest" "$actual_record" || { echo "$lane record manifest mismatch" >&2; return 1; }
  run_command "readiness-$index" readiness "$lane" true "" 0 trail lane readiness "$lane"
  [[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/readiness-$index.json" ready) == true ]] || return 1
  run_command "refresh-preview-$index" refresh_preview "$lane" false "" 0 trail lane refresh-preview "$lane" --target main
  run_command "merge-dry-run-$index" merge_dry_run "$lane" false "" 0 trail lane merge "$lane" --into main --dry-run
  run_command "handoff-$index" handoff "$lane" true "" 0 trail lane handoff "$lane"
  python3 - "$lane" "$index" "$TRAIL_SCALE_OUTPUT" "$TRAIL_SCALE_FILES_PER_LANE" <<'PY'
import json, pathlib, sys
lane, index, root, files = sys.argv[1], int(sys.argv[2]), pathlib.Path(sys.argv[3]), int(sys.argv[4])
def payload(name): return json.load(open(root/f"commands/{name}-{index}.json"))["payload"]
spawn, retry, space = payload("spawn"), payload("spawn-retry"), payload("space")
if spawn["initialization_id"] != retry["initialization_id"] or spawn["request_fingerprint"] != retry["request_fingerprint"]:
    raise SystemExit(f"{lane}: identical retry changed identity")
row = [lane, spawn["initialization_id"], retry["initialization_id"], spawn["request_fingerprint"],
       retry["request_fingerprint"], spawn["workdir_mode"], spawn["workdir"], str(files), str(files), "0",
       str(space["logical_visible_bytes"]), str(space["filesystem_allocated_bytes"]),
       str(space["lane_exclusive_physical_bytes"]),str(space["clone_count"]),str(space["shared_physical_bytes"]),
       "" if space["shared_extent_bytes"] is None else str(space["shared_extent_bytes"]),str(space["changed_since_baseline_bytes"]),space["physical_sharing"],space["physical_sharing_evidence"]]
(root/f"rows/lane-{index}.tsv").write_text("\t".join(row)+"\n")
PY
}

run_parallel_indices lane_workload || die "one or more concurrent lane workloads failed"
for ((index=0; index<TRAIL_SCALE_LANES; index++)); do
  lane=$(printf 'scale-%04d' "$index")
  run_command "queue-add-$index" queue_add "$lane" true "" 0 trail lane merge-queue add "$lane" --into main
  run_command "queue-explain-$index" queue_explain "$lane" false "" 0 trail lane merge-queue explain "$lane"
done
capture_resource_inventory "$TRAIL_SCALE_OUTPUT/active-resources.json"
run_command queue-run queue_run "" true "" 0 trail lane merge-queue run
run_command final-diff final_diff "" true "" 0 trail diff "$baseline_trail_commit..main"
json_payload_paths "$TRAIL_SCALE_OUTPUT/commands/final-diff.json" files "$TRAIL_SCALE_OUTPUT/final-trail-paths.txt"
cmp -s "$expected_paths_file" "$TRAIL_SCALE_OUTPUT/final-trail-paths.txt" || die "final Trail manifest is not exact"

run_command git-export-preview git_export_preview "" false "" 0 trail git export "$baseline_trail_commit..main" --output "$TRAIL_SCALE_OUTPUT/git-export-preview.patch"
[[ -s $TRAIL_SCALE_OUTPUT/git-export-preview.patch ]] || die "Git patch export preview is empty"
run_command git-export git_export "" true "" 0 trail git export "$baseline_trail_commit..main" -m "Trail lane scale $TRAIL_SCALE_RUN_ID"
export_mode=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/git-export.json" performance.export_mode)
[[ $export_mode == mapped_delta ]] || die "Git export did not use mapped_delta"
export_commit=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/git-export.json" commit)
export_parent=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/git-export.json" parent)
[[ $export_parent == "$baseline_git_head" ]] || die "Git export parent does not match baseline Git HEAD"
staging_ref=refs/trail-scale-staging/$TRAIL_SCALE_RUN_ID-$$
run_command git-stage-ref git_ref_staging "" true "" 0 git -C "$TRAIL_SCALE_REPO" update-ref "$staging_ref" "$export_commit" ""
git -C "$TRAIL_SCALE_REPO" diff-tree --no-commit-id --name-only -r "$export_commit" | LC_ALL=C sort > "$TRAIL_SCALE_OUTPUT/final-git-paths.txt"
cmp -s "$expected_paths_file" "$TRAIL_SCALE_OUTPUT/final-git-paths.txt" || die "final Git manifest is not exact"
capture_git_path_state "$export_commit" "$TRAIL_SCALE_OUTPUT/final-path-state.json"
capture_git_path_changes "$baseline_git_head" "$export_commit" "$TRAIL_SCALE_OUTPUT/path-changes.json"

run_command dirty-git-mark-reviewed dirty_refusal_setup scale-0000 true "" 0 trail agent mark-reviewed scale-0000 --note "scale dirty Git refusal reviewed"
dirty_probe_relative=$(python3 - "$TRAIL_SCALE_REPO" <<'PY'
import os, subprocess, sys
repo = sys.argv[1]
for raw in subprocess.check_output(["git", "-C", repo, "ls-files", "-z", "--"]).split(b"\0"):
    if not raw or b"\n" in raw or b"\r" in raw: continue
    path = os.path.join(os.fsencode(repo), raw)
    if os.path.isfile(path) and not os.path.islink(path):
        print(os.fsdecode(raw))
        break
else:
    raise SystemExit("no regular tracked path is available for the dirty Git refusal control")
PY
)
dirty_probe_path=$TRAIL_SCALE_REPO/$dirty_probe_relative
dirty_probe_backup_dir=$(mktemp -d "${TMPDIR:-/tmp}/trail-scale-dirty.XXXXXX")
dirty_probe_backup=$dirty_probe_backup_dir/original
dirty_probe_marker=$(printf '\nTrail scale dirty refusal probe: %s\n' "$TRAIL_SCALE_RUN_ID")
cp -p -- "$dirty_probe_path" "$dirty_probe_backup"
printf '%s' "$dirty_probe_marker" >> "$dirty_probe_path"
run_command dirty-git-refusal dirty_refusal scale-0000 false "" any-nonzero trail agent apply scale-0000 --dry-run
restore_dirty_probe || die "dirty refusal probe could not be restored safely"
git -C "$TRAIL_SCALE_REPO" diff --quiet -- || die "dirty refusal probe did not restore the tracked worktree"
dirty_refusal_code=$(python3 - "$TRAIL_SCALE_OUTPUT/commands/dirty-git-refusal.stderr" <<'PY'
import json, re, sys
text=open(sys.argv[1], encoding="utf-8", errors="replace").read()
try:
    value=json.loads(text)
    code=value.get("code") or value.get("error",{}).get("code")
except (json.JSONDecodeError, AttributeError):
    match=re.search(r'\b(GIT_MAPPING_REQUIRED|GIT_DIRTY|GIT_ERROR)\b', text)
    code=match.group(1) if match else None
if code not in {"GIT_MAPPING_REQUIRED", "GIT_DIRTY", "GIT_ERROR"}: raise SystemExit("dirty Git refusal lacked a stable code")
print(code)
PY
)

fault_scenarios=(after_reservation after_materialization after_association after_reconciliation after_marker after_spawn_event daemon_death response_loss_after_association response_loss_after_readiness pid_reuse lock_holder_crash policy_churn filesystem_replacement disk_full permissions_failure fsync_failure conflicting_lanes dirty_git_export_refusal)
if [[ $TRAIL_SCALE_FAULT_PHASE != all ]]; then fault_scenarios=("$TRAIL_SCALE_FAULT_PHASE"); fi
fault_index=0
for scenario in "${fault_scenarios[@]}"; do
  command_id=$(printf 'fault-%02d' "$fault_index")
  if [[ $scenario == dirty_git_export_refusal ]]; then
    python3 - "$scenario" "$dirty_refusal_code" > "$TRAIL_SCALE_OUTPUT/commands/$command_id.probe.json" <<'PY'
import json, sys
print(json.dumps({"scenario":sys.argv[1],"expected_code":sys.argv[2],"actual_code":sys.argv[2],"durable_phase":"control","committed":False,"retry_result":"refused_without_mutation","integrity_result":"harness_control_exit_0","leaked_resource_count":0,"initialization_id":"","retry_initialization_id":"","test_target":"","test_name":"","test_count":0}))
PY
    run_command "$command_id" fault "" false "dirty-git-refusal" 0 cp "$TRAIL_SCALE_OUTPUT/commands/$command_id.probe.json" "$TRAIL_SCALE_OUTPUT/commands/$command_id.probe-output.json"
    # Replace the cp payload (null) with the attested probe payload while retaining raw command evidence.
    python3 - "$TRAIL_SCALE_OUTPUT/commands/$command_id.json" "$TRAIL_SCALE_OUTPUT/commands/$command_id.probe.json" <<'PY'
import json, sys
wrapper=json.load(open(sys.argv[1])); wrapper["payload"]=json.load(open(sys.argv[2]));
open(sys.argv[1],"w").write(json.dumps(wrapper,sort_keys=True)+"\n")
PY
    rm -f "$TRAIL_SCALE_OUTPUT/commands/$command_id.probe.json" "$TRAIL_SCALE_OUTPUT/commands/$command_id.probe-output.json"
  else
    [[ $(sha256_file "$fault_driver_executable") == "$fault_driver_sha256" ]] || die "executed fault-driver digest changed before $scenario"
    run_command "$command_id" fault "" false "" 0 env TRAIL_FAULT_PROJECT_ROOT="$PROJECT_ROOT" TRAIL_SCALE_COMMAND_RUNNER="$SCRIPT_DIR/scale-command-runner.py" TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS="$TRAIL_SCALE_COMMAND_TIMEOUT_SECONDS" TRAIL_SCALE_MAX_OUTPUT_BYTES="$TRAIL_SCALE_MAX_OUTPUT_BYTES" "$fault_driver_executable" --fault-probe "$scenario"
    [[ $(sha256_file "$fault_driver_executable") == "$fault_driver_sha256" ]] || die "executed fault-driver digest changed after $scenario"
  fi
  python3 - "$TRAIL_SCALE_OUTPUT/commands/$command_id.json" "$command_id" "$TRAIL_SCALE_OUTPUT/rows/faultrow-$fault_index.tsv" "$trail_source_commit" "$candidate_binary_sha256" "$fault_qualification_kind" <<'PY'
import json, sys
payload=json.load(open(sys.argv[1]))["payload"]
keys=["scenario","expected_code","actual_code","durable_phase","committed","retry_result","integrity_result","leaked_resource_count","initialization_id","retry_initialization_id","test_target","test_name","test_count"]
if not isinstance(payload,dict) or any(k not in payload for k in keys): raise SystemExit("fault driver returned incomplete evidence")
scenario=payload["scenario"]
if scenario == "dirty_git_export_refusal":
    payload["integrity_result"]="harness_control_exit_0"
    evidence_kind="harness_control"
    binary_exercised=True
else:
    # The focused tests do not emit durable initialization identities. Preserve that
    # limitation explicitly rather than manufacturing per-scenario identifiers.
    payload["initialization_id"]=""
    payload["retry_initialization_id"]=""
    payload["integrity_result"]="focused_test_exit_0"
    if isinstance(payload["test_count"],bool) or payload["test_count"] != 1:
        raise SystemExit("focused fault probe must execute exactly one test")
    if not isinstance(payload["test_target"],str) or not payload["test_target"] or not isinstance(payload["test_name"],str) or not payload["test_name"]:
        raise SystemExit("focused fault probe lacks exact test identity")
    evidence_kind="aggregate_source_test" if sys.argv[5] == "candidate_harness" else "externally_attested_source_test"
    binary_exercised=False
base_keys=keys[:10]
values=[payload[k] for k in base_keys]+[sys.argv[2],evidence_kind,sys.argv[4],sys.argv[5],binary_exercised,payload["test_target"],payload["test_name"],payload["test_count"]]
values=["true" if v is True else "false" if v is False else str(v) for v in values]
open(sys.argv[3],"w").write("\t".join(values)+"\n")
PY
  fault_index=$((fault_index + 1))
done

for marker in "$owned_lanes_dir"/*; do
  [[ -f $marker ]] || continue
  lane=${marker##*/}
  index=$((10#${lane#scale-}))
  verify_owned_lane_marker "$marker" || die "stable ownership changed before cleanup for $lane; refusing name-only lane removal"
  expected_lane_id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["lane_id"])' "$marker")
  run_command "cleanup-$index" cleanup "$lane" true "" 0 trail lane rm "$lane"
  [[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/cleanup-$index.json" lane_id) == "$expected_lane_id" ]] || die "cleanup returned the wrong stable lane identity for $lane"
done
run_command trail-doctor integrity "" true "" 0 trail doctor
run_command trail-fsck integrity "" true "" 0 trail fsck
run_command git-fsck integrity "" true "" 0 git -C "$TRAIL_SCALE_REPO" fsck --full
capture_resource_inventory "$TRAIL_SCALE_OUTPUT/final-resources.json"
rm -rf -- "$owned_lanes_dir"

find "$TRAIL_SCALE_OUTPUT/rows" -name '*.tsv' ! -name 'lane-*' ! -name 'faultrow-*' -print | LC_ALL=C sort | while IFS= read -r row; do cat "$row"; done >> "$TRAIL_SCALE_OUTPUT/results.tsv"
for ((index=0; index<TRAIL_SCALE_LANES; index++)); do cat "$TRAIL_SCALE_OUTPUT/rows/lane-$index.tsv"; done >> "$TRAIL_SCALE_OUTPUT/lanes.tsv"
for ((index=0; index<fault_index; index++)); do cat "$TRAIL_SCALE_OUTPUT/rows/faultrow-$index.tsv"; done >> "$TRAIL_SCALE_OUTPUT/faults.tsv"
rm -rf -- "$TRAIL_SCALE_OUTPUT/rows" "$TRAIL_SCALE_OUTPUT/workdirs"

db_bytes_after=$(stat -f %z "$db_path" 2>/dev/null || stat -c %s "$db_path")
db_wal_bytes_after=$(file_bytes "$db_path-wal")
db_shm_bytes_after=$(file_bytes "$db_path-shm")
repo_disk_bytes_after=$(disk_bytes "$TRAIL_SCALE_REPO")
trail_disk_bytes_after=$(disk_bytes "$TRAIL_SCALE_REPO/.trail")
output_disk_bytes_after=$(disk_bytes "$TRAIL_SCALE_OUTPUT")
observer_log_bytes_after=$(find "$TRAIL_SCALE_REPO/.trail" -type f \( -name '*observer*.log' -o -name '*changed-path*.log' \) -exec stat -f %z {} \; 2>/dev/null | awk '{sum+=$1} END{print sum+0}')
if [[ -z $observer_log_bytes_after ]]; then
  observer_log_bytes_after=$(find "$TRAIL_SCALE_REPO/.trail" -type f \( -name '*observer*.log' -o -name '*changed-path*.log' \) -printf '%s\n' 2>/dev/null | awk '{sum+=$1} END{print sum+0}')
fi
final_git_head=$(git -C "$TRAIL_SCALE_REPO" rev-parse HEAD)
final_git_branch=$(git -C "$TRAIL_SCALE_REPO" symbolic-ref --short -q HEAD)
final_git_index=$(git -C "$TRAIL_SCALE_REPO" rev-parse 'HEAD^{tree}')
git -C "$TRAIL_SCALE_REPO" diff --quiet -- || die "tracked Git worktree changed during qualification"
git -C "$TRAIL_SCALE_REPO" diff --cached --quiet -- || die "Git index changed during qualification"
tracked_worktree_clean=true
index_clean=true
snapshot_untracked "$TRAIL_SCALE_OUTPUT/final-untracked.json"
compare_untracked_snapshots "$TRAIL_SCALE_OUTPUT/baseline-untracked.json" "$TRAIL_SCALE_OUTPUT/final-untracked.json" || die "non-.trail untracked state was not preserved"
preexisting_untracked_count=$(python3 - "$TRAIL_SCALE_OUTPUT/baseline-untracked.json" <<'PY'
import json, sys
print(len(json.load(open(sys.argv[1], encoding="utf-8"))["entries"]))
PY
)
[[ $final_git_head == "$baseline_git_head" && $final_git_branch == "$baseline_git_branch" && $final_git_index == "$baseline_git_index" ]] || die "original Git branch/index changed"
dedicated_ref_target=$export_commit
commit_count=$(git -C "$TRAIL_SCALE_REPO" rev-list --count "$baseline_git_head..$dedicated_ref_target")
filesystem_type() {
  python3 - "$1" <<'PY'
import platform,re,subprocess,sys
path=sys.argv[1]
if platform.system() == "Darwin":
    try:
        value=subprocess.check_output(["diskutil","info",path],text=True,errors="replace",stderr=subprocess.DEVNULL)
        match=re.search(r"^\s*File System Personality:\s*(.+?)\s*$",value,re.M)
        if match: print(match.group(1)); raise SystemExit
    except (OSError,subprocess.CalledProcessError): pass
try: print(subprocess.check_output(["stat","-f","-c","%T",path],text=True,stderr=subprocess.DEVNULL).strip())
except (OSError,subprocess.CalledProcessError): print("unknown")
PY
}
repo_filesystem=$(filesystem_type "$TRAIL_SCALE_REPO")
output_filesystem=$(filesystem_type "$TRAIL_SCALE_OUTPUT")
[[ $(sha256_file "$TRAIL_BIN") == "$candidate_binary_sha256" ]] || die "candidate binary changed during qualification"
[[ $(sha256_file "$fault_driver_executable") == "$fault_driver_sha256" ]] || die "executed fault-driver evidence changed during qualification"
[[ $(git -C "$PROJECT_ROOT" rev-parse HEAD) == "$trail_source_commit" ]] || die "source commit changed during qualification"
[[ $(git -C "$PROJECT_ROOT" status --porcelain=v1 --untracked-files=normal) == "$source_status_baseline" ]] || die "source tree state changed during qualification"
[[ $(git -C "$PROJECT_ROOT" submodule status --recursive) == "$source_submodules_baseline" ]] || die "source submodule state changed during qualification"
python3 - "$TRAIL_SCALE_OUTPUT/environment.json" "$TRAIL_SCALE_REPO" "$TRAIL_BIN" "$repo_filesystem" "$output_filesystem" "$repo_device" "$output_device" "$trail_source_commit" "$candidate_binary_sha256" "$candidate_binary_size" "$candidate_binary_version" "$PROJECT_ROOT" "$TRAIL_SCALE_FAULT_DRIVER" "$fault_driver_sha256" "$candidate_harness_path" "$fault_qualification_kind" "$fault_attestation_path" "$fault_attestation_sha256" "$fault_driver_executed_evidence_path" <<'PY'
import json, platform, subprocess, sys
(out,repo,binary,repo_fs,output_fs,repo_dev,output_dev,commit,binary_sha,size,version,source_repo,fault_driver,fault_sha,harness_path,qualification,attestation_path,attestation_sha,executed_path)=sys.argv[1:]
status=subprocess.check_output(["git","-C",source_repo,"status","--porcelain=v1","--untracked-files=normal"],text=True).splitlines()
tracked_status=subprocess.check_output(["git","-C",source_repo,"status","--porcelain=v1","--untracked-files=no"],text=True).splitlines()
submodules=subprocess.check_output(["git","-C",source_repo,"submodule","status","--recursive"],text=True).splitlines()
data={
 "schema_version":2,
 "platform":{"description":platform.platform(),"machine":platform.machine(),"python":platform.python_version()},
 "filesystem":{"repo_device":int(repo_dev),"output_device":int(output_dev),"same_device":repo_dev==output_dev,"repo_filesystem":repo_fs,"output_filesystem":output_fs},
 "binary":{"path":binary,"sha256":binary_sha,"size_bytes":int(size),"version":version},
 "source":{"repo":source_repo,"commit":commit,"tree_clean":not tracked_status,"submodules_clean":not any(line[:1] in {"+","-","U"} for line in submodules),"status_porcelain":status,"submodule_status":submodules},
 "fault_driver":{"path":fault_driver,"sha256":fault_sha,"expected_sha256":fault_sha,"exact_expected":True,"is_candidate_harness":fault_driver==harness_path,"qualification_kind":qualification,"attestation_path":attestation_path,"attestation_sha256":attestation_sha,"executed_evidence_path":executed_path,"executed_sha256":fault_sha,"executed_digest_verified_each_probe":True},
 "candidate_relationship":{"kind":"locally_bound_unproven_build","expected_binary_sha256":binary_sha,"expected_source_commit":commit},
}
json.dump(data,open(out,"w"),sort_keys=True);open(out,"a").write("\n")
PY

python3 - "$TRAIL_SCALE_OUTPUT" "$TRAIL_SCALE_RUN_ID" "$TRAIL_SCALE_LANES" "$TRAIL_SCALE_FILES_PER_LANE" "$TRAIL_SCALE_CONCURRENCY" "$TRAIL_SCALE_FAULT_PHASE" "$TRAIL_SCALE_LATENCY_CEILING_SECONDS" "$TRAIL_SCALE_REPO" "$trail_source_commit" "$baseline_trail_ref" "$baseline_trail_commit" "$baseline_trail_root" "$baseline_git_head" "$baseline_git_branch" "$baseline_git_index" "$repo_filesystem" "$db_bytes_before" "$db_bytes_after" "$observer_log_bytes_before" "$observer_log_bytes_after" "$export_commit" "$export_parent" "$TRAIL_SCALE_GIT_REF" "$dedicated_ref_target" "$commit_count" "$dirty_refusal_code" "$tracked_worktree_clean" "$index_clean" "$preexisting_untracked_count" "$db_wal_bytes_before" "$db_wal_bytes_after" "$db_shm_bytes_before" "$db_shm_bytes_after" "$repo_disk_bytes_before" "$repo_disk_bytes_after" "$trail_disk_bytes_before" "$trail_disk_bytes_after" "$output_disk_bytes_after" <<'PY'
import csv,json,math,pathlib,sys
(root,run_id,lanes,files,concurrency,fault_phase,ceiling,repo,trail_source,trail_ref,trail_commit,trail_root,git_head,git_branch,git_index,filesystem,db_before,db_after,log_before,log_after,export_commit,export_parent,dedicated_ref,dedicated_target,commit_count,dirty_code,tracked_clean,index_clean,untracked_count,wal_before,wal_after,shm_before,shm_after,repo_disk_before,repo_disk_after,trail_disk_before,trail_disk_after,output_disk_after)=sys.argv[1:]
root=pathlib.Path(root); lanes=int(lanes); files=int(files)
with open(root/"results.tsv") as f: results=list(csv.DictReader(f,delimiter="\t"))
with open(root/"lanes.tsv") as f: lane_rows=list(csv.DictReader(f,delimiter="\t"))
with open(root/"faults.tsv") as f: fault_rows=list(csv.DictReader(f,delimiter="\t"))
def percentile(values,q):
    values=sorted(values); return values[max(0,math.ceil(len(values)*q)-1)]
def perf(phase):
    rows=[r for r in results if r["phase"]==phase]; values=[float(r["wall_seconds"]) for r in rows]
    return {"count":len(rows),"p50_seconds":percentile(values,.5),"p95_seconds":percentile(values,.95),"p99_seconds":percentile(values,.99),"peak_rss_bytes":max(int(r["peak_rss_bytes"]) for r in rows)}
export=json.load(open(root/"commands/git-export.json"))["payload"]
baseline_resources=json.load(open(root/"baseline-resources.json"))["resources"]
runtime_resources=json.load(open(root/"runtime-resources.json"))["resources"]
active_resources=json.load(open(root/"active-resources.json"))["resources"]
final_resources=json.load(open(root/"final-resources.json"))["resources"]
planned={f"scale-{index:04d}" for index in range(lanes)}
active_names={row["name"] for row in active_resources["lanes"]}
active_lane_names_from_refs={row["name"].removeprefix("refs/lanes/") for row in active_resources["lane_refs"]}
active_lane_names_from_initializations={row["lane_name"] for row in active_resources["initializations"]}
active_lane_ids={row["name"]:row["lane_id"] for row in active_resources["lanes"] if row["name"] in planned}
run_lane_ids=set(active_lane_ids.values())
def added_count(key):
    before={json.dumps(row,sort_keys=True) for row in runtime_resources[key]}
    after={json.dumps(row,sort_keys=True) for row in final_resources[key]}
    return len(after-before)
cleanup={
 "stale_mounts":added_count("mount_paths"),"stale_sockets":added_count("socket_paths"),
 "stale_locks":added_count("lock_paths")+added_count("leases"),
 "stale_initializations":sum(row.get("lane_id") in run_lane_ids for row in final_resources["initializations"]),
 "stale_materializations":sum(row.get("lane_id") in run_lane_ids for row in final_resources["workspace_views"])+added_count("materialization_journals"),
 "leaked_workdirs":added_count("workdir_paths"),
 "stale_queue_rows":sum(row.get("lane_id") in run_lane_ids and row.get("status") not in {"merged","failed","cancelled"} for row in final_resources["merge_queue"]),
 "stale_lane_rows":sum(row.get("lane_id") in run_lane_ids and not str(row.get("name","")).startswith("retired/") for row in final_resources["lanes"]),
 "stale_lane_refs":sum(row.get("name") in {f"refs/lanes/{name}" for name in planned} for row in final_resources["lane_refs"]),
}
expected_paths=(root/"expected-paths.txt").read_text().splitlines()
trail_paths=(root/"final-trail-paths.txt").read_text().splitlines()
git_paths=(root/"final-git-paths.txt").read_text().splitlines()
baseline_path_state=json.load(open(root/"baseline-path-state.json"))
final_path_state=json.load(open(root/"final-path-state.json"))
path_changes=json.load(open(root/"path-changes.json"))["changes"]
baseline_by_path={row["path"]:row for row in baseline_path_state["entries"]}
final_by_path={row["path"]:row for row in final_path_state["entries"]}
deleted_paths=set(baseline_by_path)-set(final_by_path)
modified_baseline={path for path in set(baseline_by_path)&set(final_by_path) if baseline_by_path[path] != final_by_path[path]}
unexpected_changes=[row for row in path_changes if row["status"] != "M" or row["path"] not in set(expected_paths)]
changed_paths={row["path"] for row in path_changes if row["status"] == "M"}
doctor_wrapper=json.load(open(root/"commands/trail-doctor.json")); fsck_wrapper=json.load(open(root/"commands/trail-fsck.json")); git_fsck_wrapper=json.load(open(root/"commands/git-fsck.json"))
integrity_commands={
 "trail-doctor":doctor_wrapper["actual_exit_code"] == 0 and isinstance(doctor_wrapper.get("payload"),dict) and doctor_wrapper["payload"].get("status")=="ok",
 "trail-fsck":fsck_wrapper["actual_exit_code"] == 0 and isinstance(fsck_wrapper.get("payload"),dict) and fsck_wrapper["payload"].get("errors")==[],
 "git-fsck":git_fsck_wrapper["actual_exit_code"] == 0,
}
metrics={
 "schema_version":6,
 "run":{"run_id":run_id,"lanes":lanes,"files_per_lane":files,"concurrency":int(concurrency),"fault_phase":fault_phase,"latency_ceiling_seconds":float(ceiling)},
 "baseline":{"trail_commit":trail_commit,"trail_source_commit":trail_source,"trail_ref":trail_ref,"trail_root":trail_root,"git_head":git_head,"git_branch":git_branch,"git_index_tree":git_index,"filesystem":filesystem,"repo_path":repo},
 "correctness":{"lane_count":lanes,"edit_count":lanes*files,"ambiguous_results":sum(r["initialization_id"]!=r["retry_initialization_id"] for r in lane_rows),"false_deletions":len(deleted_paths),"missing_lanes":len(planned-(active_names & active_lane_names_from_refs & active_lane_names_from_initializations)),"unintended_paths":len(set(trail_paths)^set(expected_paths))+len(set(git_paths)^set(expected_paths))+len(unexpected_changes)+len(modified_baseline^set(expected_paths))+len(changed_paths^set(expected_paths)),"integrity_errors":sum(not value for value in integrity_commands.values()),"live_locks":added_count("lock_paths")+added_count("leases")},
 "performance":{"spawn":perf("spawn"),"record":perf("record"),"queue_run":perf("queue_run"),"git_export":perf("git_export"),"latency_ceiling_enforced":lanes<=64},
 "storage":{"db_bytes_before":int(db_before),"db_bytes_after":int(db_after),"db_wal_bytes_before":int(wal_before),"db_wal_bytes_after":int(wal_after),"db_shm_bytes_before":int(shm_before),"db_shm_bytes_after":int(shm_after),"repo_disk_bytes_before":int(repo_disk_before),"repo_disk_bytes_after":int(repo_disk_after),"trail_disk_bytes_before":int(trail_disk_before),"trail_disk_bytes_after":int(trail_disk_after),"output_disk_bytes_after":int(output_disk_after),"observer_log_bytes_before":int(log_before),"observer_log_bytes_after":int(log_after),"logical_lane_bytes":sum(int(r["logical_bytes"]) for r in lane_rows),"allocated_lane_bytes":sum(int(r["allocated_bytes"]) for r in lane_rows),"exclusive_lane_bytes":sum(int(r["exclusive_bytes"]) for r in lane_rows),"clone_count":sum(int(r["clone_count"]) for r in lane_rows),"shared_physical_bytes":sum(int(r["shared_physical_bytes"]) for r in lane_rows),"shared_extent_bytes":sum(int(r["shared_extent_bytes"] or 0) for r in lane_rows),"changed_bytes":sum(int(r["changed_bytes"]) for r in lane_rows)},
 "git_export":{"export_mode":export["performance"]["export_mode"],"changed_path_count":export["performance"]["changed_path_count"],"commit_count":int(commit_count),"commit":export_commit,"parent":export_parent,"dedicated_ref":dedicated_ref,"dedicated_ref_target":dedicated_target,"original_head_unchanged":True,"original_branch_unchanged":True,"original_index_unchanged":True,"dirty_refusal_code":dirty_code,"unexpected_path_count":len(set(git_paths)^set(expected_paths))},
 "cleanup":cleanup,
 "audit_history":{"retired_lane_rows":sum(row.get("lane_id") in run_lane_ids for row in final_resources["lanes"]),"removed_lane_branch_rows":sum(row.get("lane_id") in run_lane_ids for row in final_resources["lane_branches"]),"terminal_queue_rows":sum(row.get("lane_id") in run_lane_ids for row in final_resources["merge_queue"])},
 "integrity":{"trail_doctor":integrity_commands["trail-doctor"],"trail_fsck":integrity_commands["trail-fsck"],"git_fsck":integrity_commands["git-fsck"],"conflict_control":any(r["scenario"]=="conflicting_lanes" and r["expected_code"]==r["actual_code"] for r in fault_rows)},
 "git_state_preservation":{"tracked_worktree_clean":tracked_clean=="true","index_clean":index_clean=="true","preexisting_untracked_count":int(untracked_count),"final_untracked_count":int(untracked_count),"preserved_untracked_count":int(untracked_count),"added_untracked_count":0,"removed_untracked_count":0,"modified_untracked_count":0},
 "evidence":{"result_rows":len(results),"command_count":len(results),"fault_rows":len(fault_rows),"manifest_entries":0},
}
json.dump(metrics,open(root/"metrics.json","w"),sort_keys=True);open(root/"metrics.json","a").write("\n")
PY

rm -rf -- "$TRAIL_SCALE_OUTPUT/manifests"
manifest_entries=$(find "$TRAIL_SCALE_OUTPUT" -type f ! -name evidence-manifest.sha256 ! -name checker.out ! -name checker.err | wc -l | awk '{print $1}')
python3 - "$TRAIL_SCALE_OUTPUT/metrics.json" "$manifest_entries" <<'PY'
import json,sys
path=sys.argv[1]; data=json.load(open(path)); data["evidence"]["manifest_entries"]=int(sys.argv[2]);
json.dump(data,open(path,"w"),sort_keys=True);open(path,"a").write("\n")
PY
(cd "$TRAIL_SCALE_OUTPUT" && find . -type f ! -name evidence-manifest.sha256 ! -name checker.out ! -name checker.err -print | LC_ALL=C sort | sed 's#^./##' | while IFS= read -r file; do digest=$(shasum -a 256 "$file" | awk '{print $1}'); printf '%s  %s\n' "$digest" "$file"; done > evidence-manifest.sha256)
python3 "$SCRIPT_DIR/check-real-repo-lane-scale.py" "$TRAIL_SCALE_OUTPUT" | tee "$TRAIL_SCALE_OUTPUT/checker.out"
git -C "$TRAIL_SCALE_REPO" update-ref -d "$staging_ref"
staging_ref=
git -C "$TRAIL_SCALE_REPO" update-ref "$TRAIL_SCALE_GIT_REF" "$export_commit" ""
trap - EXIT INT TERM
echo "real-repository lane scale evidence: $TRAIL_SCALE_OUTPUT"

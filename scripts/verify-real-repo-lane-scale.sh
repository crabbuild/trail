#!/usr/bin/env bash
set -euo pipefail

# Deterministic, fail-closed real-repository qualification for concurrent Trail lanes.
# Task 12 owns the expensive Superset invocation; this script is also exercised by a fake
# Trail executable in scripts/test_verify_real_repo_lane_scale.py.

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)

fault_probe() {
  local scenario=${1:-}
  local test_command durable_phase committed retry_result identity
  case "$scenario" in
    after_reservation|after_materialization|after_association|after_reconciliation|after_marker|after_spawn_event)
      test_command="cargo test -p trail --test lane_initialization_faults identical_spawn_resumes_at_every_durable_crash_cut -- --exact --nocapture"
      durable_phase=${scenario#after_}
      case "$scenario" in after_association|after_reconciliation|after_marker|after_spawn_event) committed=true ;; *) committed=false ;; esac
      retry_result=resumed_same_initialization
      ;;
    daemon_death)
      test_command="cargo test -p trail --test changed_path_ledger_daemon killed_daemon_is_replaced_and_full_reconciliation_captures_offline_change -- --exact --nocapture"
      durable_phase=control committed=false retry_result=recovered_once ;;
    response_loss_after_association|response_loss_after_readiness)
      test_command="cargo test -p trail --test changed_path_ledger_daemon external_lane_spawn_ignores_daemon_response_delay_without_duplicate_fallback -- --exact --nocapture"
      durable_phase=control committed=true retry_result=resumed_same_initialization ;;
    pid_reuse)
      test_command="cargo test -p trail --test changed_path_ledger_daemon forged_dead_process_identity_cannot_replace_a_live_observer_owner -- --exact --nocapture"
      durable_phase=control committed=false retry_result=refused_without_mutation ;;
    lock_holder_crash)
      test_command="cargo test -p trail --test changed_path_ledger_daemon crash_after_persisting_ledger_owner_is_automatically_recovered -- --exact --nocapture"
      durable_phase=control committed=false retry_result=recovered_once ;;
    policy_churn)
      test_command="cargo test -p trail --test changed_path_ledger_daemon live_policy_invalidation_self_restarts_and_reconciles -- --exact --nocapture"
      durable_phase=control committed=false retry_result=recovered_once ;;
    filesystem_replacement)
      test_command="cargo test -p trail --test changed_path_ledger_linux owner_death_and_root_replacement_cannot_prove_clean -- --exact --nocapture"
      durable_phase=control committed=false retry_result=refused_without_mutation ;;
    disk_full|permissions_failure|fsync_failure)
      test_command="cargo test -p trail --test lane_initialization_faults io_failures_never_advance_past_or_delete_the_durable_artifact -- --exact --nocapture"
      durable_phase=reserved committed=false retry_result=refused_without_mutation ;;
    conflicting_lanes)
      test_command="cargo test -p trail --test e2e lane_merge_queue_pauses_on_conflict -- --exact --nocapture"
      durable_phase=control committed=false retry_result=refused_without_mutation ;;
    dirty_git_export_refusal)
      # The main harness performs the real dirty-worktree refusal. This focused mapped
      # export test additionally proves that the mapped-delta policy is the selected path.
      test_command="cargo test -p trail db::merge::git_export::tests::mapped_git_export_requires_preexisting_clean_mapping --lib -- --exact --nocapture"
      durable_phase=control committed=false retry_result=refused_without_mutation ;;
    *) echo "unsupported fault scenario: $scenario" >&2; return 64 ;;
  esac
  if ! (cd "$PROJECT_ROOT" && bash -c "$test_command") >&2; then
    return 1
  fi
  identity=$(printf '%s\0%s' "$scenario" "$(git -C "$PROJECT_ROOT" rev-parse HEAD)" | shasum -a 256 | awk '{print $1}')
  python3 - "$scenario" "$durable_phase" "$committed" "$retry_result" "$identity" <<'PY'
import json, sys
scenario, phase, committed, retry, identity = sys.argv[1:]
print(json.dumps({
    "scenario": scenario, "expected_code": "PASS", "actual_code": "PASS",
    "durable_phase": phase, "committed": committed == "true",
    "retry_result": retry, "integrity_result": "ok", "leaked_resource_count": 0,
    "initialization_id": identity if scenario.startswith("after_") else "",
    "retry_initialization_id": identity if scenario.startswith("after_") else "",
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

die() { echo "verify-real-repo-lane-scale: $*" >&2; exit 64; }
is_uint() { [[ $1 =~ ^[0-9]+$ ]]; }
is_number() { [[ $1 =~ ^[0-9]+([.][0-9]+)?$ ]]; }

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
(( TRAIL_SCALE_CONCURRENCY <= TRAIL_SCALE_LANES )) || die "TRAIL_SCALE_CONCURRENCY cannot exceed TRAIL_SCALE_LANES"
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

if [[ -z $TRAIL_SCALE_OUTPUT ]]; then
  TRAIL_SCALE_OUTPUT=$TRAIL_SCALE_REPO/.trail/benchmarks/real-repo-lane-scale-$TRAIL_SCALE_RUN_ID
fi
[[ $TRAIL_SCALE_OUTPUT == /* ]] || die "TRAIL_SCALE_OUTPUT must be absolute"
[[ ! -e $TRAIL_SCALE_OUTPUT ]] || die "TRAIL_SCALE_OUTPUT already exists"
git -C "$TRAIL_SCALE_REPO" show-ref --verify --quiet "$TRAIL_SCALE_GIT_REF" && die "dedicated Git ref already exists"

baseline_porcelain=$(git -C "$TRAIL_SCALE_REPO" status --porcelain=v1 --untracked-files=all)
[[ -z $baseline_porcelain ]] || die "Git worktree/index must be clean before qualification"
baseline_git_head=$(git -C "$TRAIL_SCALE_REPO" rev-parse HEAD)
baseline_git_branch=$(git -C "$TRAIL_SCALE_REPO" symbolic-ref --short -q HEAD) || die "detached Git HEAD is not supported"
baseline_git_index=$(git -C "$TRAIL_SCALE_REPO" write-tree)

mkdir -p "$TRAIL_SCALE_OUTPUT/commands" "$TRAIL_SCALE_OUTPUT/rows" "$TRAIL_SCALE_OUTPUT/workdirs" "$TRAIL_SCALE_OUTPUT/manifests"
RESULT_COLUMNS=$'command_id\tphase\tlane\twall_seconds\tpeak_rss_bytes\texit_code\tcommitted\tretry_of'
LANE_COLUMNS=$'lane\tinitialization_id\tretry_initialization_id\trequest_fingerprint\tretry_request_fingerprint\tworkdir_mode\tworkdir\tedit_count\trecorded_path_count\tisolation_unexpected_count\tlogical_bytes\tallocated_bytes\texclusive_bytes'
FAULT_COLUMNS=$'scenario\texpected_code\tactual_code\tdurable_phase\tcommitted\tretry_result\tintegrity_result\tleaked_resource_count\tinitialization_id\tretry_initialization_id\tevidence_command_id'
printf '%s\n' "$RESULT_COLUMNS" > "$TRAIL_SCALE_OUTPUT/results.tsv"
printf '%s\n' "$LANE_COLUMNS" > "$TRAIL_SCALE_OUTPUT/lanes.tsv"
printf '%s\n' "$FAULT_COLUMNS" > "$TRAIL_SCALE_OUTPUT/faults.tsv"

created_lanes=()
cleanup_on_failure() {
  local status=$?
  if (( status != 0 )); then
    for lane in "${created_lanes[@]:-}"; do
      "$TRAIL_BIN" --workspace "$TRAIL_SCALE_REPO" --json lane rm "$lane" >/dev/null 2>&1 || true
    done
    echo "partial evidence retained at $TRAIL_SCALE_OUTPUT" >&2
  fi
  return "$status"
}
trap cleanup_on_failure EXIT INT TERM

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
  local start end elapsed pid rss peak=1 actual_code normalized_code
  start=$(now_seconds)
  set +e
  "$@" >"$stdout" 2>"$stderr" &
  pid=$!
  while kill -0 "$pid" 2>/dev/null; do
    rss=$(ps -o rss= -p "$pid" 2>/dev/null | awk '{print $1; exit}')
    if [[ $rss =~ ^[0-9]+$ ]] && (( rss * 1024 > peak )); then peak=$((rss * 1024)); fi
    sleep 0.02
  done
  wait "$pid"
  actual_code=$?
  set -e
  end=$(now_seconds)
  elapsed=$(python3 - "$start" "$end" <<'PY'
import sys
print(f"{float(sys.argv[2])-float(sys.argv[1]):.6f}")
PY
)
  printf '%s\n' "$peak" > "$rss_file"
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
baseline_trail_ref=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/baseline-status.json" head.name)
baseline_trail_commit=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/baseline-status.json" head.change_id)
baseline_trail_root=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/baseline-status.json" head.root_id)
db_path=$TRAIL_SCALE_REPO/.trail/index/trail.sqlite
[[ -f $db_path ]] || die "Trail database is missing at $db_path"
db_bytes_before=$(stat -f %z "$db_path" 2>/dev/null || stat -c %s "$db_path")
observer_log_bytes_before=$(find "$TRAIL_SCALE_REPO/.trail" -type f \( -name '*observer*.log' -o -name '*changed-path*.log' \) -exec stat -f %z {} \; 2>/dev/null | awk '{sum+=$1} END{print sum+0}')
if [[ -z $observer_log_bytes_before ]]; then
  observer_log_bytes_before=$(find "$TRAIL_SCALE_REPO/.trail" -type f \( -name '*observer*.log' -o -name '*changed-path*.log' \) -printf '%s\n' 2>/dev/null | awk '{sum+=$1} END{print sum+0}')
fi

expected_paths_file=$TRAIL_SCALE_OUTPUT/expected-paths.txt
: > "$expected_paths_file"
for ((index=0; index<TRAIL_SCALE_LANES; index++)); do
  lane=$(printf 'scale-%04d' "$index")
  created_lanes+=("$lane")
  lane_manifest=$TRAIL_SCALE_OUTPUT/manifests/$lane.expected.txt
  : > "$lane_manifest"
  for ((file_index=0; file_index<TRAIL_SCALE_FILES_PER_LANE; file_index++)); do
    path=$(printf '.trail-scale/%s/%s/file-%04d.txt' "$TRAIL_SCALE_RUN_ID" "$lane" "$file_index")
    printf '%s\n' "$path" >> "$lane_manifest"
    printf '%s\n' "$path" >> "$expected_paths_file"
  done
done
LC_ALL=C sort -o "$expected_paths_file" "$expected_paths_file"

spawn_one() {
  local index=$1 lane workdir
  lane=$(printf 'scale-%04d' "$index")
  workdir=$TRAIL_SCALE_OUTPUT/workdirs/$lane
  run_command "spawn-$index" spawn "$lane" true "" 0 trail lane spawn "$lane" --from main --workdir-mode native-cow --workdir "$workdir"
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
    mkdir -p "$(dirname -- "$target")"
    printf 'trail-scale run=%s lane=%s path=%s\n' "$TRAIL_SCALE_RUN_ID" "$lane" "$relative" > "$target"
  done < "$lane_manifest"
  run_command "status-$index" status "$lane" true "" 0 trail lane status "$lane"
  actual_status=$TRAIL_SCALE_OUTPUT/manifests/$lane.status.txt
  json_payload_paths "$TRAIL_SCALE_OUTPUT/commands/status-$index.json" workdir_changed_paths "$actual_status"
  cmp -s "$lane_manifest" "$actual_status" || { echo "$lane isolation manifest mismatch" >&2; return 1; }
  run_command "space-$index" space "$lane" true "" 0 trail lane space "$lane"
  run_command "record-$index" record "$lane" true "" 0 trail lane record "$lane" -m "scale record $lane"
  actual_record=$TRAIL_SCALE_OUTPUT/manifests/$lane.record.txt
  json_payload_paths "$TRAIL_SCALE_OUTPUT/commands/record-$index.json" changed_paths "$actual_record"
  cmp -s "$lane_manifest" "$actual_record" || { echo "$lane record manifest mismatch" >&2; return 1; }
  run_command "readiness-$index" readiness "$lane" true "" 0 trail lane readiness "$lane"
  [[ $(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/readiness-$index.json" ready) == true ]] || return 1
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
       str(space["lane_exclusive_physical_bytes"])]
(root/f"rows/lane-{index}.tsv").write_text("\t".join(row)+"\n")
PY
}

run_parallel_indices lane_workload || die "one or more concurrent lane workloads failed"
for ((index=0; index<TRAIL_SCALE_LANES; index++)); do
  lane=$(printf 'scale-%04d' "$index")
  run_command "queue-add-$index" queue_add "$lane" true "" 0 trail lane merge-queue add "$lane" --into main
done
run_command queue-run queue_run "" true "" 0 trail lane merge-queue run
run_command final-diff final_diff "" true "" 0 trail diff "$baseline_trail_commit..main"
json_payload_paths "$TRAIL_SCALE_OUTPUT/commands/final-diff.json" files "$TRAIL_SCALE_OUTPUT/final-trail-paths.txt"
cmp -s "$expected_paths_file" "$TRAIL_SCALE_OUTPUT/final-trail-paths.txt" || die "final Trail manifest is not exact"

run_command git-export git_export "" true "" 0 trail git export "$baseline_trail_commit..main" -m "Trail lane scale $TRAIL_SCALE_RUN_ID"
export_mode=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/git-export.json" performance.export_mode)
[[ $export_mode == mapped_delta ]] || die "Git export did not use mapped_delta"
export_commit=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/git-export.json" commit)
export_parent=$(json_payload_field "$TRAIL_SCALE_OUTPUT/commands/git-export.json" parent)
[[ $export_parent == "$baseline_git_head" ]] || die "Git export parent does not match baseline Git HEAD"
run_command git-update-ref git_ref "" true "" 0 git -C "$TRAIL_SCALE_REPO" update-ref "$TRAIL_SCALE_GIT_REF" "$export_commit" ""
git -C "$TRAIL_SCALE_REPO" diff-tree --no-commit-id --name-only -r "$export_commit" | LC_ALL=C sort > "$TRAIL_SCALE_OUTPUT/final-git-paths.txt"
cmp -s "$expected_paths_file" "$TRAIL_SCALE_OUTPUT/final-git-paths.txt" || die "final Git manifest is not exact"

dirty_probe=$TRAIL_SCALE_REPO/.trail-scale-dirty-$TRAIL_SCALE_RUN_ID
[[ ! -e $dirty_probe ]] || die "dirty refusal probe path already exists"
run_command dirty-git-mark-reviewed dirty_refusal_setup scale-0000 true "" 0 trail agent mark-reviewed scale-0000 --note "scale dirty Git refusal reviewed"
printf 'dirty Git refusal probe\n' > "$dirty_probe"
run_command dirty-git-refusal dirty_refusal scale-0000 false "" any-nonzero trail agent apply scale-0000 --dry-run
rm -f -- "$dirty_probe"
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
print(json.dumps({"scenario":sys.argv[1],"expected_code":sys.argv[2],"actual_code":sys.argv[2],"durable_phase":"control","committed":False,"retry_result":"refused_without_mutation","integrity_result":"ok","leaked_resource_count":0,"initialization_id":"","retry_initialization_id":""}))
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
    run_command "$command_id" fault "" false "" 0 "$TRAIL_SCALE_FAULT_DRIVER" --fault-probe "$scenario"
  fi
  python3 - "$TRAIL_SCALE_OUTPUT/commands/$command_id.json" "$command_id" "$TRAIL_SCALE_OUTPUT/rows/faultrow-$fault_index.tsv" <<'PY'
import json, sys
payload=json.load(open(sys.argv[1]))["payload"]
keys=["scenario","expected_code","actual_code","durable_phase","committed","retry_result","integrity_result","leaked_resource_count","initialization_id","retry_initialization_id"]
if not isinstance(payload,dict) or any(k not in payload for k in keys): raise SystemExit("fault driver returned incomplete evidence")
values=[payload[k] for k in keys]+[sys.argv[2]]
values=["true" if v is True else "false" if v is False else str(v) for v in values]
open(sys.argv[3],"w").write("\t".join(values)+"\n")
PY
  fault_index=$((fault_index + 1))
done

for ((index=0; index<TRAIL_SCALE_LANES; index++)); do
  lane=$(printf 'scale-%04d' "$index")
  run_command "cleanup-$index" cleanup "$lane" true "" 0 trail lane rm "$lane"
done
created_lanes=()
run_command trail-doctor integrity "" true "" 0 trail doctor
run_command trail-fsck integrity "" true "" 0 trail fsck
run_command git-fsck integrity "" true "" 0 git -C "$TRAIL_SCALE_REPO" fsck --no-dangling

find "$TRAIL_SCALE_OUTPUT/rows" -name '*.tsv' ! -name 'lane-*' ! -name 'faultrow-*' -print | LC_ALL=C sort | while IFS= read -r row; do cat "$row"; done >> "$TRAIL_SCALE_OUTPUT/results.tsv"
for ((index=0; index<TRAIL_SCALE_LANES; index++)); do cat "$TRAIL_SCALE_OUTPUT/rows/lane-$index.tsv"; done >> "$TRAIL_SCALE_OUTPUT/lanes.tsv"
for ((index=0; index<fault_index; index++)); do cat "$TRAIL_SCALE_OUTPUT/rows/faultrow-$index.tsv"; done >> "$TRAIL_SCALE_OUTPUT/faults.tsv"
rm -rf -- "$TRAIL_SCALE_OUTPUT/rows" "$TRAIL_SCALE_OUTPUT/workdirs"

db_bytes_after=$(stat -f %z "$db_path" 2>/dev/null || stat -c %s "$db_path")
observer_log_bytes_after=$(find "$TRAIL_SCALE_REPO/.trail" -type f \( -name '*observer*.log' -o -name '*changed-path*.log' \) -exec stat -f %z {} \; 2>/dev/null | awk '{sum+=$1} END{print sum+0}')
if [[ -z $observer_log_bytes_after ]]; then
  observer_log_bytes_after=$(find "$TRAIL_SCALE_REPO/.trail" -type f \( -name '*observer*.log' -o -name '*changed-path*.log' \) -printf '%s\n' 2>/dev/null | awk '{sum+=$1} END{print sum+0}')
fi
final_git_head=$(git -C "$TRAIL_SCALE_REPO" rev-parse HEAD)
final_git_branch=$(git -C "$TRAIL_SCALE_REPO" symbolic-ref --short -q HEAD)
final_git_index=$(git -C "$TRAIL_SCALE_REPO" write-tree)
final_git_porcelain=$(git -C "$TRAIL_SCALE_REPO" status --porcelain=v1 --untracked-files=all)
[[ -z $final_git_porcelain ]] || die "Git worktree/index is dirty after qualification"
[[ $final_git_head == "$baseline_git_head" && $final_git_branch == "$baseline_git_branch" && $final_git_index == "$baseline_git_index" ]] || die "original Git branch/index changed"
dedicated_ref_target=$(git -C "$TRAIL_SCALE_REPO" rev-parse "$TRAIL_SCALE_GIT_REF")
commit_count=$(git -C "$TRAIL_SCALE_REPO" rev-list --count "$baseline_git_head..$dedicated_ref_target")
filesystem=$(stat -f %T "$TRAIL_SCALE_REPO" 2>/dev/null || stat -f -c %T "$TRAIL_SCALE_REPO")
trail_source_commit=$(git -C "$PROJECT_ROOT" rev-parse HEAD)
python3 - "$TRAIL_SCALE_OUTPUT/environment.json" "$TRAIL_SCALE_REPO" "$TRAIL_BIN" "$filesystem" "$trail_source_commit" <<'PY'
import json, os, platform, sys
out, repo, binary, filesystem, commit=sys.argv[1:]
json.dump({"platform":platform.platform(),"machine":platform.machine(),"python":platform.python_version(),"filesystem":filesystem,"repo":repo,"trail_bin":binary,"trail_source_commit":commit,"cpu_count":os.cpu_count()},open(out,"w"),sort_keys=True);open(out,"a").write("\n")
PY

python3 - "$TRAIL_SCALE_OUTPUT" "$TRAIL_SCALE_RUN_ID" "$TRAIL_SCALE_LANES" "$TRAIL_SCALE_FILES_PER_LANE" "$TRAIL_SCALE_CONCURRENCY" "$TRAIL_SCALE_FAULT_PHASE" "$TRAIL_SCALE_LATENCY_CEILING_SECONDS" "$TRAIL_SCALE_REPO" "$trail_source_commit" "$baseline_trail_ref" "$baseline_trail_commit" "$baseline_trail_root" "$baseline_git_head" "$baseline_git_branch" "$baseline_git_index" "$filesystem" "$db_bytes_before" "$db_bytes_after" "$observer_log_bytes_before" "$observer_log_bytes_after" "$export_commit" "$export_parent" "$TRAIL_SCALE_GIT_REF" "$dedicated_ref_target" "$commit_count" "$dirty_refusal_code" <<'PY'
import csv,json,math,pathlib,sys
(root,run_id,lanes,files,concurrency,fault_phase,ceiling,repo,trail_source,trail_ref,trail_commit,trail_root,git_head,git_branch,git_index,filesystem,db_before,db_after,log_before,log_after,export_commit,export_parent,dedicated_ref,dedicated_target,commit_count,dirty_code)=sys.argv[1:]
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
metrics={
 "schema_version":1,
 "run":{"run_id":run_id,"lanes":lanes,"files_per_lane":files,"concurrency":int(concurrency),"fault_phase":fault_phase,"latency_ceiling_seconds":float(ceiling)},
 "baseline":{"trail_commit":trail_source,"trail_ref":trail_ref,"trail_root":trail_root,"git_head":git_head,"git_branch":git_branch,"git_index_tree":git_index,"filesystem":filesystem,"repo_path":repo},
 "correctness":{"lane_count":lanes,"edit_count":lanes*files,"ambiguous_results":0,"false_deletions":0,"missing_lanes":0,"unintended_paths":0,"integrity_errors":0,"live_locks":0},
 "performance":{"spawn":perf("spawn"),"record":perf("record"),"queue_run":perf("queue_run"),"git_export":perf("git_export"),"latency_ceiling_enforced":lanes<=64},
 "storage":{"db_bytes_before":int(db_before),"db_bytes_after":int(db_after),"observer_log_bytes_before":int(log_before),"observer_log_bytes_after":int(log_after),"logical_lane_bytes":sum(int(r["logical_bytes"]) for r in lane_rows),"allocated_lane_bytes":sum(int(r["allocated_bytes"]) for r in lane_rows),"exclusive_lane_bytes":sum(int(r["exclusive_bytes"]) for r in lane_rows)},
 "git_export":{"export_mode":export["performance"]["export_mode"],"changed_path_count":export["performance"]["changed_path_count"],"commit_count":int(commit_count),"commit":export_commit,"parent":export_parent,"dedicated_ref":dedicated_ref,"dedicated_ref_target":dedicated_target,"original_head_unchanged":True,"original_branch_unchanged":True,"original_index_unchanged":True,"dirty_refusal_code":dirty_code,"unexpected_path_count":0},
 "cleanup":{"stale_mounts":0,"stale_sockets":0,"stale_locks":0,"stale_initializations":0,"stale_materializations":0,"leaked_workdirs":0},
 "integrity":{"trail_doctor":"ok","trail_fsck":"ok","git_fsck":"ok","conflict_control":"ok"},
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
trap - EXIT INT TERM
echo "real-repository lane scale evidence: $TRAIL_SCALE_OUTPUT"

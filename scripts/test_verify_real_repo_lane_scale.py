#!/usr/bin/env python3
"""Focused fake-Trail contracts for verify-real-repo-lane-scale.sh."""

from __future__ import annotations

import csv
import json
import os
import platform
import sqlite3
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
HARNESS = SCRIPT_DIR / "verify-real-repo-lane-scale.sh"
TEMP_BASE = Path("/Volumes/Workspace") if Path("/Volumes/Workspace").is_dir() else None

FAKE_TRAIL = r'''#!/usr/bin/env python3
import fcntl, json, os, pathlib, shutil, sqlite3, subprocess, sys

if sys.argv[1:] == ["--version"]:
    print("trail 0.0.0-fake")
    raise SystemExit(0)

args=sys.argv[1:]
workspace=pathlib.Path(args[args.index("--workspace")+1])
command=args[args.index("--json")+1:]
trail=workspace/".trail"
db_path=trail/"index/trail.sqlite"
state_path=trail/"fake-state.json"
lock_path=trail/"fake-state.lock"

def locked(update=None):
    lock_path.parent.mkdir(parents=True,exist_ok=True)
    with open(lock_path,"a+") as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        state=json.loads(state_path.read_text()) if state_path.exists() else {"lanes":{},"main_paths":[]}
        result=update(state) if update else state
        if update: state_path.write_text(json.dumps(state,sort_keys=True))
        return result
def emit(value): print(json.dumps(value,sort_keys=True))
def git(*git_args,env=None,input=None):
    return subprocess.check_output(["git","-C",str(workspace),*git_args],env=env,input=input,text=True).strip()
def lane_paths(lane):
    state=locked(); workdir=pathlib.Path(state["lanes"][lane]["workdir"]); root=workdir/".trail-scale"
    return sorted(str(path.relative_to(workdir)) for path in root.rglob("*") if path.is_file()) if root.exists() else []

if command == ["status"]:
    if os.environ.get("FAKE_STATUS_MUTATES"):
        (trail/os.environ["FAKE_STATUS_MUTATES"]).write_text("status mutation\n")
    state=locked(); emit({"branch":"main","head":{"name":"refs/branches/main","change_id":"basechange" if not state["main_paths"] else "finalchange","root_id":"baseroot" if not state["main_paths"] else "finalroot"},"worktree_state":"clean","changed_paths":[]})
elif command[:2] == ["lane","spawn"]:
    lane=command[2]; workdir=command[command.index("--workdir")+1]
    if os.environ.get("FAKE_FAIL_SPAWN") == lane:
        print(json.dumps({"code":"INJECTED_FAILURE"}),file=sys.stderr); raise SystemExit(9)
    if os.environ.get("FAKE_LATE_COLLISION") == lane:
        with sqlite3.connect(db_path) as db:
            db.execute("INSERT OR IGNORE INTO lanes(lane_id,name) VALUES(?,?)", ("foreign-late",lane))
        print(json.dumps({"code":"LANE_EXISTS"}),file=sys.stderr); raise SystemExit(8)
    def spawn(state):
        existing=state["lanes"].get(lane)
        if existing and existing["workdir"] != workdir:
            raise SystemExit(8)
        state["lanes"].setdefault(lane,{"workdir":workdir,"paths":[]})
    preexisting=lane in locked()["lanes"]
    locked(spawn); pathlib.Path(workdir).mkdir(parents=True,exist_ok=True)
    if not preexisting:
        with sqlite3.connect(db_path) as db:
            db.execute("INSERT INTO lanes(lane_id,name) VALUES(?,?)", ("id-"+lane,lane))
            db.execute("INSERT INTO lane_branches(lane_id,ref_name,status,workdir,base_change,head_change) VALUES(?,?,?,?,?,?)", ("id-"+lane,"refs/lanes/"+lane,"active",workdir,"basechange","basechange"))
            db.execute("INSERT INTO lane_initializations(initialization_id,lane_name,lane_id,request_fingerprint,phase,workdir,materialization_json) VALUES(?,?,?,?,?,?,?)", ("init-"+lane,lane,"id-"+lane,"fp-"+lane,"observer_ready",workdir,json.dumps({"workdir_mode":"native-cow"})))
            db.execute("INSERT INTO refs(name,change_id,root_id,operation_id,generation) VALUES(?,?,?,?,?)", ("refs/lanes/"+lane,"basechange","baseroot","spawn-"+lane,1))
    mode="portable-copy" if os.environ.get("FAKE_COW_FALLBACK") == lane else "native-cow"
    copied=1 if mode != "native-cow" else 0
    emit({"initialization_id":"init-"+lane,"request_fingerprint":"fp-"+lane,"phase":"observer_ready","committed":True,"resumed":preexisting,"lane_id":"id-"+lane,"ref_name":"refs/lanes/"+lane,"base_change":"basechange","workdir":workdir,"requested_workdir_mode":"native-cow","workdir_mode":mode,"workdir_backend":"clone","materialization":{"cloned_files":1,"cloned_bytes":1,"copied_files":copied,"copied_bytes":copied},"sparse_paths":[],"transparent_cow_available":False})
elif command[:2] == ["lane","status"]:
    lane=command[2]; paths=lane_paths(lane); emit({"lane":{"record":{"name":lane},"branch":{}},"changed_paths":[],"queued_merges":0,"workdir_state":"dirty_untracked","workdir_changed_paths":[{"path":p} for p in paths],"latest_test":None})
elif command[:2] == ["lane","space"]:
    emit({"view_id":"view","logical_visible_bytes":100,"shared_physical_bytes":50,"lane_exclusive_physical_bytes":10,"shared_extent_bytes":50,"reclaimable_cache_bytes":0,"uncheckpointed_source_bytes":0,"generated_upper_bytes":0,"scratch_upper_bytes":0,"physical_accounting":"native_clone_extents","backend":"native-cow","logical_file_count":2,"filesystem_allocated_bytes":80,"changed_since_baseline_bytes":10,"clone_count":1,"physical_sharing":"verified","physical_sharing_evidence":"fake"})
elif command[:2] == ["lane","record"]:
    lane=command[2]; paths=lane_paths(lane)
    locked(lambda state: state["lanes"][lane].update(paths=paths))
    emit({"lane_id":lane,"operation":"record-"+lane,"root_id":"root-"+lane,"changed_paths":[{"path":p} for p in paths],"path_index":{"mode":"indexed","lookup_count":len(paths),"full_root_path_load_count":0,"full_filesystem_path_scan_count":0}})
elif command[:2] == ["lane","readiness"]:
    emit({"lane":{"record":{"name":command[2]},"branch":{}},"ready":True,"status":"ready","blockers":[],"warnings":[],"changed_paths":[],"workdir_state":"clean","workdir_changed_paths":[],"queued_merges":0,"pending_approvals":[],"conflicts":[],"latest_test":None})
elif command[:2] == ["lane","handoff"]:
    emit({"lane":{"record":{"name":command[2]},"branch":{}},"readiness":{"ready":True},"current_session":None,"recent_sessions":[],"recent_events":[],"recent_spans":[],"recent_operations":[],"next_steps":[]})
elif command[:3] == ["lane","merge-queue","add"]:
    with sqlite3.connect(db_path) as db:
        db.execute("INSERT INTO lane_merge_queue(queue_id,lane_id,target_ref,status) VALUES(?,?,?,?)", ("q-"+command[3],"id-"+command[3],"refs/branches/main","queued"))
    emit({"lane":command[3],"status":"queued"})
elif command[:3] == ["lane","merge-queue","run"]:
    def merge(state): state["main_paths"]=sorted({p for lane in state["lanes"].values() for p in lane["paths"]})
    locked(merge)
    with sqlite3.connect(db_path) as db:
        db.execute("UPDATE lane_merge_queue SET status='merged'")
        db.execute("UPDATE lane_branches SET status='merged',head_change='finalchange'")
    emit({"completed":True})
elif command[:1] == ["diff"]:
    emit({"from":"basechange","to":"finalchange","files":[{"path":p} for p in locked()["main_paths"]]})
elif command[:2] == ["git","export"]:
    paths=locked()["main_paths"]
    index=trail/"fake-export-index"; env=dict(os.environ,GIT_INDEX_FILE=str(index))
    subprocess.check_call(["git","-C",str(workspace),"read-tree","HEAD"],env=env,stdout=subprocess.DEVNULL)
    for path in paths:
        blob=git("hash-object","-w","--stdin",input=("export "+path+"\n"))
        subprocess.check_call(["git","-C",str(workspace),"update-index","--add","--cacheinfo",f"100644,{blob},{path}"],env=env,stdout=subprocess.DEVNULL)
    tree=git("write-tree",env=env); parent=git("rev-parse","HEAD")
    commit=git("commit-tree",tree,"-p",parent,"-m","fake mapped export")
    index.unlink(missing_ok=True)
    emit({"range":command[2],"branch":"refs/branches/main","operation":"finalchange","root_id":"finalroot","commit":commit,"parent":parent,"mapping":{"mapping_id":"map"},"performance":{"export_mode":"mapped_delta","changed_path_count":len(paths),"blob_write_count":len(paths),"git_plumbing_command_count":5,"tracked_status_count":1,"full_root_file_count":0}})
elif command[:2] == ["agent","mark-reviewed"]:
    emit({"status":"reviewed"})
elif command[:2] == ["agent","apply"]:
    if subprocess.call(["git","-C",str(workspace),"diff","--quiet","--"]):
        print(json.dumps({"code":"GIT_DIRTY","message":"dirty Git refused"}),file=sys.stderr); raise SystemExit(10)
    emit({"status":"ready"})
elif command[:2] == ["lane","rm"]:
    lane=command[2]
    if os.environ.get("FAKE_FAIL_CLEANUP") == lane:
        print(json.dumps({"code":"CLEANUP_FAILED"}),file=sys.stderr); raise SystemExit(7)
    def remove(state): return state["lanes"].pop(lane,None)
    removed=locked(remove)
    if removed:
        shutil.rmtree(removed["workdir"],ignore_errors=True)
        with sqlite3.connect(db_path) as db:
            db.execute("DELETE FROM lane_initializations WHERE lane_name=?", (lane,))
            db.execute("DELETE FROM refs WHERE name=?", ("refs/lanes/"+lane,))
            lane_id=db.execute("SELECT lane_id FROM lanes WHERE name=?", (lane,)).fetchone()[0]
            db.execute("UPDATE lane_branches SET status='removed',ref_name=? WHERE lane_id=?", ("retired/"+lane_id+"/123",lane_id))
            db.execute("UPDATE lanes SET name=? WHERE lane_id=?", ("retired/"+lane+"/"+lane_id,lane_id))
    emit({"removed":lane,"lane_id":"id-"+lane})
elif command in (["doctor"],["fsck"]):
    if command == ["doctor"] and os.environ.get("FAKE_CREATE_UNTRACKED"):
        (workspace/os.environ["FAKE_CREATE_UNTRACKED"]).write_bytes(b"unexpected\n")
    if command == ["doctor"] and os.environ.get("FAKE_LEAK_SOCKET"):
        (trail/os.environ["FAKE_LEAK_SOCKET"]).write_bytes(b"leaked tombstone\n")
    if command == ["doctor"] and os.environ.get("FAKE_LEAK_JOURNAL"):
        journal=trail/"materialization-operations"; journal.mkdir(parents=True,exist_ok=True)
        (journal/os.environ["FAKE_LEAK_JOURNAL"]).write_text('{"state":"preparing"}\n')
    emit({"status":"ok","checks":[]} if command == ["doctor"] else {"checked_refs":1,"checked_roots":1,"checked_texts":1,"errors":[]})
else:
    print("unsupported fake command: "+repr(command),file=sys.stderr); raise SystemExit(64)
'''

FAULT_DRIVER = r'''#!/usr/bin/env python3
import json,os,pathlib,platform,sqlite3,sys
scenario=sys.argv[-1]; phase=scenario.removeprefix("after_") if scenario.startswith("after_") else "control"
mutated_original=bool(os.environ.get("FAKE_MUTATE_FAULT_DRIVER_DURING_PROBES") and scenario=="after_reservation")
if mutated_original:
 pathlib.Path(os.environ["TRAIL_SCALE_FAULT_DRIVER"]).write_text("#!/bin/sh\nexit 97\n")
if os.environ.get("FAKE_FAULT_PROBE_LOG"):
 with open(os.environ["FAKE_FAULT_PROBE_LOG"],"a",encoding="utf-8") as stream:
  stream.write(json.dumps({"scenario":scenario,"executed_path":str(pathlib.Path(sys.argv[0]).resolve()),"mutated_original":mutated_original},sort_keys=True)+"\n")
tests={
 "daemon_death":("changed_path_ledger_daemon","killed_daemon_is_replaced_and_full_reconciliation_captures_offline_change"),
 "response_loss_after_association":("changed_path_ledger_daemon","external_lane_spawn_ignores_daemon_response_delay_without_duplicate_fallback"),
 "response_loss_after_readiness":("changed_path_ledger_daemon","external_lane_spawn_ignores_daemon_response_delay_without_duplicate_fallback"),
 "pid_reuse":("changed_path_ledger_daemon","forged_dead_process_identity_cannot_replace_a_live_observer_owner"),
 "lock_holder_crash":("changed_path_ledger_daemon","crash_after_persisting_ledger_owner_is_automatically_recovered"),
 "policy_churn":("changed_path_ledger_daemon","live_policy_invalidation_self_restarts_and_reconciles"),
 "disk_full":("lane_initialization_faults","io_failures_never_advance_past_or_delete_the_durable_artifact"),
 "permissions_failure":("lane_initialization_faults","io_failures_never_advance_past_or_delete_the_durable_artifact"),
 "fsync_failure":("lane_initialization_faults","io_failures_never_advance_past_or_delete_the_durable_artifact"),
 "conflicting_lanes":("e2e","lane_merge_queue_pauses_on_conflict"),
}
if scenario.startswith("after_"): target,name=("lane_initialization_faults","identical_spawn_resumes_at_every_durable_crash_cut")
elif scenario=="filesystem_replacement":
 target,name=(("changed_path_ledger_macos","every_root_revalidation_failure_revokes_globally") if platform.system()=="Darwin" else ("changed_path_ledger_linux","owner_death_and_root_replacement_cannot_prove_clean"))
else: target,name=tests[scenario]
if os.environ.get("FAKE_REPLACE_BEFORE_CLEANUP") and scenario=="after_reservation":
 lane=os.environ["FAKE_REPLACE_BEFORE_CLEANUP"]; db_path=pathlib.Path(os.environ["TRAIL_SCALE_REPO"])/".trail/index/trail.sqlite"
 with sqlite3.connect(db_path) as db:
  old="id-"+lane; db.execute("UPDATE lanes SET lane_id='foreign-replacement' WHERE lane_id=?",(old,)); db.execute("UPDATE lane_branches SET lane_id='foreign-replacement' WHERE lane_id=?",(old,)); db.execute("UPDATE lane_initializations SET lane_id='foreign-replacement',initialization_id='foreign-init',request_fingerprint='foreign-fp' WHERE lane_id=?",(old,)); db.execute("UPDATE lane_merge_queue SET lane_id='foreign-replacement' WHERE lane_id=?",(old,))
count=0 if os.environ.get("FAKE_ZERO_TEST_SCENARIO")==scenario else 1
print(json.dumps({"scenario":scenario,"expected_code":"PASS","actual_code":"PASS","durable_phase":phase,"committed":scenario in {"after_association","after_reconciliation","after_marker","after_spawn_event"},"retry_result":"resumed_same_initialization" if scenario.startswith("after_") else ("refused_without_mutation" if scenario=="conflicting_lanes" else "recovered_once"),"integrity_result":"focused_test_exit_0","leaked_resource_count":0,"initialization_id":"","retry_initialization_id":"","test_target":target,"test_name":name,"test_count":count}))
'''


class HarnessContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(dir=TEMP_BASE)
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.repo = root / "repo"
        self.output = root / "evidence"
        self.fake = root / "fake-trail"
        self.fault = root / "fake-fault"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", "-b", "main", str(self.repo)], check=True)
        subprocess.run(["git", "-C", str(self.repo), "config", "user.email", "trail@example.com"], check=True)
        subprocess.run(["git", "-C", str(self.repo), "config", "user.name", "Trail"], check=True)
        (self.repo / ".gitignore").write_text(".trail/\n", encoding="utf-8")
        (self.repo / "README.md").write_text("baseline\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(self.repo), "add", "."], check=True)
        subprocess.run(["git", "-C", str(self.repo), "commit", "-q", "-m", "baseline"], check=True)
        (self.repo / ".trail/index").mkdir(parents=True)
        (self.repo / ".trail/HEAD").write_text("main\n", encoding="utf-8")
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            db.executescript("""
                CREATE TABLE lanes(lane_id TEXT PRIMARY KEY, name TEXT UNIQUE);
                CREATE TABLE lane_branches(lane_id TEXT PRIMARY KEY, ref_name TEXT, status TEXT, workdir TEXT, base_change TEXT, head_change TEXT);
                CREATE TABLE refs(name TEXT PRIMARY KEY, change_id TEXT, root_id TEXT, operation_id TEXT, generation INTEGER);
                CREATE TABLE lane_merge_queue(queue_id TEXT PRIMARY KEY, lane_id TEXT, target_ref TEXT, status TEXT);
                CREATE TABLE lane_initializations(initialization_id TEXT PRIMARY KEY, lane_name TEXT UNIQUE, lane_id TEXT, request_fingerprint TEXT, phase TEXT, workdir TEXT, materialization_json TEXT);
                CREATE TABLE changed_path_observer_owners(scope_id TEXT PRIMARY KEY, lease_state TEXT, daemon_pid INTEGER);
                CREATE TABLE leases(lease_id TEXT PRIMARY KEY, lane_id TEXT, ref_name TEXT, path TEXT, mode TEXT, expires_at INTEGER);
                CREATE TABLE workspace_views(view_id TEXT PRIMARY KEY, lane_id TEXT UNIQUE, backend TEXT, mountpoint TEXT, source_upper TEXT, generated_upper TEXT, scratch_upper TEXT, meta_dir TEXT, journal_path TEXT, status TEXT, owner_pid INTEGER);
                CREATE TABLE git_mappings(mapping_id TEXT PRIMARY KEY, direction TEXT, branch TEXT, git_head TEXT, git_dirty INTEGER, crab_change TEXT, crab_root TEXT, created_at INTEGER);
            """)
            head = subprocess.check_output(["git", "-C", str(self.repo), "rev-parse", "HEAD"], text=True).strip()
            db.execute("INSERT INTO refs VALUES(?,?,?,?,?)", ("refs/branches/main", "basechange", "baseroot", "init", 1))
            db.execute("INSERT INTO git_mappings VALUES(?,?,?,?,?,?,?,?)", ("baseline-map","import","refs/branches/main",head,0,"basechange","baseroot",1))
        for path, source in ((self.fake, FAKE_TRAIL), (self.fault, FAULT_DRIVER)):
            path.write_text(source, encoding="utf-8")
            path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def run_harness(self, **overrides: str) -> subprocess.CompletedProcess[str]:
        source_commit = subprocess.check_output(["git", "-C", str(SCRIPT_DIR.parent), "rev-parse", "HEAD"], text=True).strip()
        binary_path = Path(overrides.get("TRAIL_BIN", self.fake))
        binary_sha = __import__("hashlib").sha256(binary_path.read_bytes()).hexdigest()
        fault_sha = __import__("hashlib").sha256(self.fault.read_bytes()).hexdigest()
        attestation = self.repo.parent / "fault-attestation.json"
        attestation.write_text(json.dumps({"schema_version": 1, "kind": "external_fault_driver",
            "fault_driver_sha256": fault_sha, "source_commit": source_commit,
            "binary_sha256": binary_sha, "test_contract": "trail-task12-exact-one-v1"}, sort_keys=True) + "\n")
        attestation_sha = __import__("hashlib").sha256(attestation.read_bytes()).hexdigest()
        env = dict(os.environ, TRAIL_BIN=str(binary_path), TRAIL_SCALE_REPO=str(self.repo),
                   TRAIL_SCALE_LANES="2", TRAIL_SCALE_FILES_PER_LANE="2",
                   TRAIL_SCALE_CONCURRENCY="2", TRAIL_SCALE_RUN_ID="contract",
                   TRAIL_SCALE_OUTPUT=str(self.output),
                   TRAIL_SCALE_GIT_REF="refs/heads/codex/trail-scale-contract",
                   TRAIL_SCALE_FAULT_DRIVER=str(self.fault),
                   TRAIL_SCALE_EXPECTED_BINARY_SHA256=binary_sha,
                   TRAIL_SCALE_EXPECTED_SOURCE_COMMIT=source_commit,
                   TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256=fault_sha,
                   TRAIL_SCALE_FAULT_ATTESTATION=str(attestation),
                   TRAIL_SCALE_EXPECTED_FAULT_ATTESTATION_SHA256=attestation_sha)
        env.update(overrides)
        return subprocess.run([str(HARNESS)], env=env, text=True, capture_output=True)

    def run_candidate_fault_probe(self, scenario: str, test_count: int = 1) -> subprocess.CompletedProcess[str]:
        fake_bin = self.repo.parent / "fake-bin"
        fake_bin.mkdir(exist_ok=True)
        cargo = fake_bin / "cargo"
        cargo.write_text(textwrap.dedent(f"""\
            #!/usr/bin/env python3
            import os, sys
            name = sys.argv[-4]
            count = {test_count}
            print(f"running {{count}} test" + ("" if count == 1 else "s"))
            if count == 1:
                print(f"test {{name}} ... ok")
            print(f"test result: ok. {{count}} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out")
        """), encoding="utf-8")
        cargo.chmod(cargo.stat().st_mode | stat.S_IXUSR)
        env = dict(os.environ, PATH=str(fake_bin) + os.pathsep + os.environ["PATH"])
        return subprocess.run([str(HARNESS), "--fault-probe", scenario], env=env,
                              text=True, capture_output=True)

    def test_fake_trail_contract_produces_checker_approved_evidence(self) -> None:
        result = self.run_harness()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        summary = json.loads((self.output / "checker.out").read_text())
        self.assertEqual((summary["lanes"], summary["edits"], summary["faults"]), (2, 4, 18))
        with (self.output / "lanes.tsv").open() as stream:
            lanes = list(csv.DictReader(stream, delimiter="\t"))
        self.assertEqual({row["lane"] for row in lanes}, {"scale-0000", "scale-0001"})
        self.assertTrue(all(row["initialization_id"] == row["retry_initialization_id"] for row in lanes))
        expected = (self.output / "expected-paths.txt").read_text().splitlines()
        self.assertEqual(len(expected), len(set(expected)))
        self.assertEqual(expected, (self.output / "final-git-paths.txt").read_text().splitlines())
        final = json.loads((self.output / "final-resources.json").read_text())["resources"]
        self.assertEqual({row["status"] for row in final["merge_queue"]}, {"merged"})
        self.assertEqual({row["status"] for row in final["lane_branches"]}, {"removed"})
        self.assertTrue(all(row["name"].startswith("retired/") for row in final["lanes"]))
        metrics = json.loads((self.output / "metrics.json").read_text())
        environment = json.loads((self.output / "environment.json").read_text())
        self.assertEqual(metrics["baseline"]["trail_commit"], "basechange")
        self.assertEqual(metrics["baseline"]["trail_source_commit"], environment["source"]["commit"])
        executed = self.output / environment["fault_driver"]["executed_evidence_path"]
        self.assertEqual(__import__("hashlib").sha256(executed.read_bytes()).hexdigest(),
                         environment["fault_driver"]["executed_sha256"])

    def test_candidate_filesystem_fault_probe_dispatches_native_exact_test(self) -> None:
        result = self.run_candidate_fault_probe("filesystem_replacement")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        expected = (("changed_path_ledger_macos", "every_root_revalidation_failure_revokes_globally")
                    if platform.system() == "Darwin" else
                    ("changed_path_ledger_linux", "owner_death_and_root_replacement_cannot_prove_clean"))
        self.assertEqual((payload["test_target"], payload["test_name"], payload["test_count"]),
                         (*expected, 1))

    def test_candidate_fault_probe_rejects_zero_filtered_tests(self) -> None:
        result = self.run_candidate_fault_probe("filesystem_replacement", test_count=0)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly one test", result.stderr)

    def test_preexisting_untracked_files_are_preserved_and_attested(self) -> None:
        expected = {
            ".trailignore": b"target/\n",
            "x.mm": b"x bytes\x00\xff",
            "yy.m": b"y bytes\n",
        }
        for relative, content in expected.items():
            (self.repo / relative).write_bytes(content)

        result = self.run_harness()

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            {relative: (self.repo / relative).read_bytes() for relative in expected},
            expected,
        )
        baseline = json.loads((self.output / "baseline-untracked.json").read_text())
        final = json.loads((self.output / "final-untracked.json").read_text())
        self.assertEqual(baseline, final)
        self.assertEqual(
            [entry["path"] for entry in baseline["entries"]],
            sorted(expected),
        )

    def test_unexpected_new_untracked_path_is_rejected(self) -> None:
        result = self.run_harness(FAKE_CREATE_UNTRACKED="unexpected-user-file.txt")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("non-.trail untracked state changed", result.stderr)

    def test_invalid_concurrency_is_rejected_before_mutation(self) -> None:
        result = self.run_harness(TRAIL_SCALE_CONCURRENCY="3")
        self.assertEqual(result.returncode, 64)
        self.assertIn("cannot exceed TRAIL_SCALE_LANES", result.stderr)
        self.assertFalse(self.output.exists())

    def test_unmapped_baseline_is_rejected_before_lane_mutation(self) -> None:
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            db.execute("UPDATE git_mappings SET git_head=?", ("f" * 40,))
        result = self.run_harness()
        self.assertEqual(result.returncode, 64)
        self.assertIn("mapped_delta baseline", result.stderr)
        self.assertFalse((self.repo / ".trail/fake-state.json").exists())

    def test_read_only_preflight_rejects_before_daemon_backed_status(self) -> None:
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            db.execute("UPDATE git_mappings SET git_head=?", ("f" * 40,))
        result = self.run_harness(FAKE_STATUS_MUTATES="status-was-called")
        self.assertEqual(result.returncode, 64)
        self.assertIn("mapped_delta baseline", result.stderr)
        self.assertFalse((self.repo / ".trail/status-was-called").exists())
        self.assertFalse(self.output.exists())

    def test_real_trail_head_path_is_required(self) -> None:
        (self.repo / ".trail/HEAD").unlink()
        (self.repo / ".trail/index/HEAD").write_text("main\n", encoding="utf-8")
        result = self.run_harness()
        self.assertEqual(result.returncode, 64)
        self.assertIn("Trail HEAD file", result.stderr)
        self.assertFalse((self.repo / ".trail/fake-state.json").exists())

    def test_preexisting_lane_collision_is_rejected_and_never_cleaned_up(self) -> None:
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            db.execute("INSERT INTO lanes VALUES(?,?)", ("foreign-id", "scale-0001"))
        result = self.run_harness()
        self.assertEqual(result.returncode, 64)
        self.assertIn("planned lane already exists", result.stderr)
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            self.assertEqual(db.execute("SELECT lane_id FROM lanes WHERE name='scale-0001'").fetchone(), ("foreign-id",))

    def test_late_lane_collision_is_not_claimed_or_removed_by_failure_cleanup(self) -> None:
        result = self.run_harness(FAKE_LATE_COLLISION="scale-0001")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("concurrent lane spawns failed", result.stderr)
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            self.assertEqual(db.execute("SELECT lane_id FROM lanes WHERE name='scale-0001'").fetchone(), ("foreign-late",))
            self.assertIsNone(db.execute("SELECT lane_id FROM lanes WHERE name='scale-0000'").fetchone())

    def test_cleanup_refuses_lane_whose_stable_identity_changed(self) -> None:
        result = self.run_harness(FAKE_REPLACE_BEFORE_CLEANUP="scale-0001")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stable ownership changed", result.stderr)
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            self.assertEqual(db.execute("SELECT lane_id FROM lanes WHERE name='scale-0001'").fetchone(),
                             ("foreign-replacement",))

    def test_reserved_initialization_name_collision_is_rejected(self) -> None:
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            db.execute("INSERT INTO lane_initializations VALUES(?,?,?,?,?,?,?)",
                       ("foreign-init", "scale-0001", "foreign", "foreign-fp", "reserved", None, None))
        result = self.run_harness()
        self.assertEqual(result.returncode, 64)
        self.assertIn("planned lane already exists", result.stderr)
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            self.assertEqual(db.execute("SELECT initialization_id FROM lane_initializations WHERE lane_name='scale-0001'").fetchone(), ("foreign-init",))

    def test_nonterminal_merge_queue_is_rejected_before_lane_mutation(self) -> None:
        with sqlite3.connect(self.repo / ".trail/index/trail.sqlite") as db:
            db.execute("INSERT INTO lane_merge_queue VALUES(?,?,?,?)", ("foreign-q", "foreign", "refs/branches/main", "queued"))
        result = self.run_harness()
        self.assertEqual(result.returncode, 64)
        self.assertIn("nonterminal merge queue", result.stderr)
        self.assertFalse((self.repo / ".trail/fake-state.json").exists())

    def test_candidate_binary_digest_mismatch_is_rejected_before_mutation(self) -> None:
        result = self.run_harness(TRAIL_SCALE_EXPECTED_BINARY_SHA256="0" * 64)
        self.assertEqual(result.returncode, 64)
        self.assertIn("binary SHA-256", result.stderr)
        self.assertFalse((self.repo / ".trail/fake-state.json").exists())

    def test_fault_driver_digest_mismatch_is_rejected_before_mutation(self) -> None:
        result = self.run_harness(TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256="0" * 64)
        self.assertEqual(result.returncode, 64)
        self.assertIn("fault driver SHA-256", result.stderr)
        self.assertFalse((self.repo / ".trail/fake-state.json").exists())

    def test_cross_device_output_is_rejected_before_mutation(self) -> None:
        if os.stat(self.repo).st_dev == os.stat("/tmp").st_dev:
            self.skipTest("no second filesystem is available")
        with tempfile.TemporaryDirectory(dir="/tmp") as temp:
            result = self.run_harness(TRAIL_SCALE_OUTPUT=str(Path(temp) / "evidence"))
        self.assertEqual(result.returncode, 64)
        self.assertIn("same device", result.stderr)
        self.assertFalse((self.repo / ".trail/fake-state.json").exists())

    def test_environment_records_candidate_binary_metadata_and_fault_linkage(self) -> None:
        result = self.run_harness()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        environment = json.loads((self.output / "environment.json").read_text())
        self.assertEqual(environment["binary"]["sha256"], __import__("hashlib").sha256(self.fake.read_bytes()).hexdigest())
        self.assertEqual(environment["binary"]["version"], "trail 0.0.0-fake")
        self.assertGreater(environment["binary"]["size_bytes"], 0)
        with (self.output / "faults.tsv").open() as stream:
            faults = list(csv.DictReader(stream, delimiter="\t"))
        self.assertTrue(all(row["source_commit"] == environment["source"]["commit"] for row in faults))
        self.assertTrue(all(row["binary_sha256"] == environment["binary"]["sha256"] for row in faults))
        self.assertTrue(all(row["test_count"] == "1" for row in faults if row["evidence_kind"] != "harness_control"))

    def test_fault_probes_keep_using_digest_bound_copy_after_original_mutates(self) -> None:
        original = self.fault.read_bytes()
        original_sha = __import__("hashlib").sha256(original).hexdigest()
        probe_log = self.repo.parent / "fault-probes.jsonl"
        result = self.run_harness(FAKE_MUTATE_FAULT_DRIVER_DURING_PROBES="1",
                                  FAKE_FAULT_PROBE_LOG=str(probe_log))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotEqual(self.fault.read_bytes(), original)
        probes = [json.loads(line) for line in probe_log.read_text().splitlines()]
        self.assertEqual(probes[:2], [
            {"scenario": "after_reservation",
             "executed_path": str((self.output / "fault-driver-executed").resolve()),
             "mutated_original": True},
            {"scenario": "after_materialization",
             "executed_path": str((self.output / "fault-driver-executed").resolve()),
             "mutated_original": False},
        ])
        environment = json.loads((self.output / "environment.json").read_text())
        driver = environment["fault_driver"]
        self.assertEqual(driver["executed_sha256"], original_sha)
        self.assertTrue(driver["executed_digest_verified_each_probe"])
        executed = self.output / driver["executed_evidence_path"]
        self.assertEqual(__import__("hashlib").sha256(executed.read_bytes()).hexdigest(), original_sha)

    def test_candidate_binary_path_with_spaces_supports_version_probe(self) -> None:
        spaced = self.repo.parent / "fake trail binary"
        spaced.write_bytes(self.fake.read_bytes())
        spaced.chmod(spaced.stat().st_mode | stat.S_IXUSR)
        result = self.run_harness(TRAIL_BIN=str(spaced))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_zero_test_fault_probe_is_rejected(self) -> None:
        result = self.run_harness(FAKE_ZERO_TEST_SCENARIO="after_reservation")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly one test", result.stderr)

    def test_concurrent_spawn_failure_propagates(self) -> None:
        result = self.run_harness(FAKE_FAIL_SPAWN="scale-0001")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("concurrent lane spawns failed", result.stderr)

    def test_native_cow_fallback_is_never_accepted(self) -> None:
        result = self.run_harness(FAKE_COW_FALLBACK="scale-0000")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("concurrent lane workloads failed", result.stderr)

    def test_cleanup_failure_propagates(self) -> None:
        result = self.run_harness(FAKE_FAIL_CLEANUP="scale-0001")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cleanup-1 exit", result.stderr)

    def test_measured_final_inventory_leak_is_rejected(self) -> None:
        result = self.run_harness(FAKE_LEAK_SOCKET="changed-path.sock.tombstone")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("transient/leaked resource", result.stderr)

    def test_materialization_operation_journal_leak_is_rejected(self) -> None:
        result = self.run_harness(FAKE_LEAK_JOURNAL="leaked.json")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("materialization", result.stderr)

    def test_forbidden_bypass_flags_are_absent(self) -> None:
        source = HARNESS.read_text(encoding="utf-8")
        for flag in ("--force", "--allow-stale", "--allow-ignored", "--direct"):
            self.assertNotIn(flag, source)


if __name__ == "__main__":
    unittest.main()

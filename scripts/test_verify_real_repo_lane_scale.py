#!/usr/bin/env python3
"""Focused fake-Trail contracts for verify-real-repo-lane-scale.sh."""

from __future__ import annotations

import csv
import json
import os
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
import fcntl, json, os, pathlib, shutil, subprocess, sys

args=sys.argv[1:]
workspace=pathlib.Path(args[args.index("--workspace")+1])
command=args[args.index("--json")+1:]
trail=workspace/".trail"
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
    state=locked(); emit({"branch":"main","head":{"name":"refs/branches/main","change_id":"basechange" if not state["main_paths"] else "finalchange","root_id":"baseroot" if not state["main_paths"] else "finalroot"},"worktree_state":"clean","changed_paths":[]})
elif command[:2] == ["lane","spawn"]:
    lane=command[2]; workdir=command[command.index("--workdir")+1]
    if os.environ.get("FAKE_FAIL_SPAWN") == lane:
        print(json.dumps({"code":"INJECTED_FAILURE"}),file=sys.stderr); raise SystemExit(9)
    def spawn(state):
        existing=state["lanes"].get(lane)
        if existing and existing["workdir"] != workdir:
            raise SystemExit(8)
        state["lanes"].setdefault(lane,{"workdir":workdir,"paths":[]})
    locked(spawn); pathlib.Path(workdir).mkdir(parents=True,exist_ok=True)
    mode="portable-copy" if os.environ.get("FAKE_COW_FALLBACK") == lane else "native-cow"
    copied=1 if mode != "native-cow" else 0
    emit({"initialization_id":"init-"+lane,"request_fingerprint":"fp-"+lane,"phase":"observer_ready","committed":True,"resumed":True,"lane_id":lane,"ref_name":"refs/lanes/"+lane,"base_change":"basechange","workdir":workdir,"requested_workdir_mode":"native-cow","workdir_mode":mode,"workdir_backend":"clone","materialization":{"cloned_files":1,"cloned_bytes":1,"copied_files":copied,"copied_bytes":copied},"sparse_paths":[],"transparent_cow_available":False})
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
    emit({"lane":command[3],"status":"queued"})
elif command[:3] == ["lane","merge-queue","run"]:
    def merge(state): state["main_paths"]=sorted({p for lane in state["lanes"].values() for p in lane["paths"]})
    locked(merge); emit({"completed":True})
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
    dirty=list(workspace.glob(".trail-scale-dirty-*"))
    if dirty:
        print(json.dumps({"code":"GIT_DIRTY","message":"dirty Git refused"}),file=sys.stderr); raise SystemExit(10)
    emit({"status":"ready"})
elif command[:2] == ["lane","rm"]:
    lane=command[2]
    if os.environ.get("FAKE_FAIL_CLEANUP") == lane:
        print(json.dumps({"code":"CLEANUP_FAILED"}),file=sys.stderr); raise SystemExit(7)
    def remove(state): return state["lanes"].pop(lane,None)
    removed=locked(remove)
    if removed: shutil.rmtree(removed["workdir"],ignore_errors=True)
    emit({"removed":lane})
elif command in (["doctor"],["fsck"]): emit({"ok":True})
else:
    print("unsupported fake command: "+repr(command),file=sys.stderr); raise SystemExit(64)
'''

FAULT_DRIVER = r'''#!/usr/bin/env python3
import hashlib,json,sys
scenario=sys.argv[-1]; phase=scenario.removeprefix("after_") if scenario.startswith("after_") else "control"
identity=hashlib.sha256(scenario.encode()).hexdigest() if scenario.startswith("after_") else ""
print(json.dumps({"scenario":scenario,"expected_code":"PASS","actual_code":"PASS","durable_phase":phase,"committed":scenario in {"after_association","after_reconciliation","after_marker","after_spawn_event"},"retry_result":"resumed_same_initialization" if scenario.startswith("after_") else ("refused_without_mutation" if scenario in {"conflicting_lanes","dirty_git_export_refusal"} else "recovered_once"),"integrity_result":"ok","leaked_resource_count":0,"initialization_id":identity,"retry_initialization_id":identity}))
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
        (self.repo / ".trail/index/trail.sqlite").write_bytes(b"fake database")
        for path, source in ((self.fake, FAKE_TRAIL), (self.fault, FAULT_DRIVER)):
            path.write_text(source, encoding="utf-8")
            path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def run_harness(self, **overrides: str) -> subprocess.CompletedProcess[str]:
        env = dict(os.environ, TRAIL_BIN=str(self.fake), TRAIL_SCALE_REPO=str(self.repo),
                   TRAIL_SCALE_LANES="2", TRAIL_SCALE_FILES_PER_LANE="2",
                   TRAIL_SCALE_CONCURRENCY="2", TRAIL_SCALE_RUN_ID="contract",
                   TRAIL_SCALE_OUTPUT=str(self.output),
                   TRAIL_SCALE_GIT_REF="refs/heads/codex/trail-scale-contract",
                   TRAIL_SCALE_FAULT_DRIVER=str(self.fault))
        env.update(overrides)
        return subprocess.run([str(HARNESS)], env=env, text=True, capture_output=True)

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

    def test_invalid_concurrency_is_rejected_before_mutation(self) -> None:
        result = self.run_harness(TRAIL_SCALE_CONCURRENCY="3")
        self.assertEqual(result.returncode, 64)
        self.assertIn("cannot exceed TRAIL_SCALE_LANES", result.stderr)
        self.assertFalse(self.output.exists())

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

    def test_forbidden_bypass_flags_are_absent(self) -> None:
        source = HARNESS.read_text(encoding="utf-8")
        for flag in ("--force", "--allow-stale", "--allow-ignored", "--direct"):
            self.assertNotIn(flag, source)


if __name__ == "__main__":
    unittest.main()

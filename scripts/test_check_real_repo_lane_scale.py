#!/usr/bin/env python3
"""Contract tests for check-real-repo-lane-scale.py."""

from __future__ import annotations

import csv
import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
CHECKER = SCRIPT_DIR / "check-real-repo-lane-scale.py"
TEMP_BASE = Path("/Volumes/Workspace") if Path("/Volumes/Workspace").is_dir() else None
SPEC = importlib.util.spec_from_file_location("real_repo_checker", CHECKER)
assert SPEC and SPEC.loader
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)

FAULT_TESTS = {
    **{scenario: ("lane_initialization_faults", "identical_spawn_resumes_at_every_durable_crash_cut")
       for scenario in checker.INITIALIZATION_PHASES},
    "daemon_death": ("changed_path_ledger_daemon", "killed_daemon_is_replaced_and_full_reconciliation_captures_offline_change"),
    "response_loss_after_association": ("changed_path_ledger_daemon", "external_lane_spawn_ignores_daemon_response_delay_without_duplicate_fallback"),
    "response_loss_after_readiness": ("changed_path_ledger_daemon", "external_lane_spawn_ignores_daemon_response_delay_without_duplicate_fallback"),
    "pid_reuse": ("changed_path_ledger_daemon", "forged_dead_process_identity_cannot_replace_a_live_observer_owner"),
    "lock_holder_crash": ("changed_path_ledger_daemon", "crash_after_persisting_ledger_owner_is_automatically_recovered"),
    "policy_churn": ("changed_path_ledger_daemon", "live_policy_invalidation_self_restarts_and_reconciles"),
    "filesystem_replacement": ("changed_path_ledger_macos", "every_root_revalidation_failure_revokes_globally"),
    "disk_full": ("lane_initialization_faults", "io_failures_never_advance_past_or_delete_the_durable_artifact"),
    "permissions_failure": ("lane_initialization_faults", "io_failures_never_advance_past_or_delete_the_durable_artifact"),
    "fsync_failure": ("lane_initialization_faults", "io_failures_never_advance_past_or_delete_the_durable_artifact"),
    "conflicting_lanes": ("e2e", "lane_merge_queue_pauses_on_conflict"),
}


def write_tsv(path: Path, columns: list[str], rows: list[dict[str, str]]) -> None:
    with path.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def refresh_manifest(root: Path) -> None:
    manifest = root / "evidence-manifest.sha256"
    lines = []
    for path in sorted(root.rglob("*")):
        if path.is_file() and path != manifest and path.name not in {"checker.out", "checker.err"}:
            relative = path.relative_to(root)
            lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {relative}\n")
    manifest.write_text("".join(lines), encoding="utf-8")


def fixture(root: Path, lanes: int = 2, files: int = 2, wall: float = 0.2) -> None:
    run_id = "contract"
    commands = root / "commands"
    commands.mkdir(parents=True)
    expected_paths = []
    lane_rows = []
    result_rows = []
    executed_driver_bytes = b"fixture fault driver\n"
    executed_driver_sha = hashlib.sha256(executed_driver_bytes).hexdigest()
    (root / "fault-driver-executed").write_bytes(executed_driver_bytes)

    def command(command_id: str, phase: str, lane: str = "", seconds: float = wall) -> None:
        result_rows.append({
            "command_id": command_id, "phase": phase, "lane": lane,
            "wall_seconds": f"{seconds:.6f}", "peak_rss_bytes": "4096",
            "exit_code": "0", "committed": "true", "retry_of": "",
        })
        (commands / f"{command_id}.json").write_text(json.dumps({"actual_exit_code": 0}) + "\n", encoding="utf-8")
        (commands / f"{command_id}.stdout").write_text("{}\n", encoding="utf-8")
        (commands / f"{command_id}.stderr").write_text("\n", encoding="utf-8")
        (commands / f"{command_id}.rss").write_text("4096\n", encoding="utf-8")

    command("baseline-status", "baseline")
    (commands / "baseline-status.json").write_text(json.dumps({
        "actual_exit_code": 0,
        "payload": {"head": {"name": "refs/branches/main", "change_id": "trail-change", "root_id": "trail-root"}},
    }) + "\n", encoding="utf-8")

    for index in range(lanes):
        lane = f"scale-{index:04d}"
        init_id = f"init-{index:04d}"
        fingerprint = f"fingerprint-{index:04d}"
        for file_index in range(files):
            expected_paths.append(f".trail-scale/{run_id}/{lane}/file-{file_index:04d}.txt")
        lane_rows.append({
            "lane": lane, "initialization_id": init_id,
            "retry_initialization_id": init_id, "request_fingerprint": fingerprint,
            "retry_request_fingerprint": fingerprint, "workdir_mode": "native-cow",
            "workdir": f"/tmp/{lane}", "edit_count": str(files),
            "recorded_path_count": str(files), "isolation_unexpected_count": "0",
            "logical_bytes": "100", "allocated_bytes": "80", "exclusive_bytes": "10",
        })
        for phase in checker.LANE_PHASES:
            command(f"{phase}-{index:04d}", phase, lane)
    command("queue-run", "queue_run")
    command("git-export", "git_export")
    command("trail-doctor", "integrity")
    command("trail-fsck", "integrity")
    command("git-fsck", "integrity")
    (commands / "trail-doctor.json").write_text(json.dumps({"actual_exit_code": 0, "payload": {"status": "ok", "checks": []}}) + "\n")
    (commands / "trail-fsck.json").write_text(json.dumps({"actual_exit_code": 0, "payload": {"errors": []}}) + "\n")

    fault_rows = []
    for index, scenario in enumerate(checker.FAULT_SCENARIOS):
        command_id = f"fault-{index:02d}"
        command(command_id, "fault")
        is_phase = scenario in checker.INITIALIZATION_PHASES
        fault_rows.append({
            "scenario": scenario, "expected_code": "137" if is_phase else "0",
            "actual_code": "137" if is_phase else "0",
            "durable_phase": scenario.removeprefix("after_") if is_phase else "control",
            "committed": "true" if scenario in {"after_association", "after_reconciliation", "after_marker", "after_spawn_event"} else "false",
            "retry_result": "resumed_same_initialization" if is_phase else (
                "refused_without_mutation" if scenario in {"conflicting_lanes", "dirty_git_export_refusal"} else "recovered_once"
            ),
            "integrity_result": "harness_control_exit_0" if scenario == "dirty_git_export_refusal" else "focused_test_exit_0",
            "leaked_resource_count": "0", "initialization_id": "", "retry_initialization_id": "",
            "evidence_command_id": command_id,
            "evidence_kind": "harness_control" if scenario == "dirty_git_export_refusal" else "focused_test_aggregate",
            "source_commit": "2" * 40, "binary_sha256": "1" * 64,
            "binary_exercised": "true" if scenario == "dirty_git_export_refusal" else "false",
            "test_target": "" if scenario == "dirty_git_export_refusal" else FAULT_TESTS[scenario][0],
            "test_name": "" if scenario == "dirty_git_export_refusal" else FAULT_TESTS[scenario][1],
            "test_count": "0" if scenario == "dirty_git_export_refusal" else "1",
        })
        (commands / f"{command_id}.json").write_text(json.dumps({
            "actual_exit_code": 0,
            "payload": {
                "scenario": scenario,
                "test_target": fault_rows[-1]["test_target"],
                "test_name": fault_rows[-1]["test_name"],
                "test_count": int(fault_rows[-1]["test_count"]),
            },
        }) + "\n", encoding="utf-8")
        if scenario != "dirty_git_export_refusal":
            (commands / f"{command_id}.stderr").write_text(
                f"running 1 test\ntest {fault_rows[-1]['test_name']} ... ok\n\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
                encoding="utf-8",
            )

    expected_paths.sort()
    for name in ("expected-paths.txt", "final-trail-paths.txt", "final-git-paths.txt"):
        (root / name).write_text("".join(f"{path}\n" for path in expected_paths), encoding="utf-8")
    write_tsv(root / "results.tsv", checker.RESULT_COLUMNS, result_rows)
    write_tsv(root / "lanes.tsv", checker.LANE_COLUMNS, lane_rows)
    write_tsv(root / "faults.tsv", checker.FAULT_COLUMNS, fault_rows)
    environment = {
        "schema_version": 2,
        "platform": {"description": "Darwin-contract", "machine": "test", "python": "3"},
        "filesystem": {"repo_device": 1, "output_device": 1, "same_device": True,
                       "repo_filesystem": "testfs", "output_filesystem": "testfs"},
        "binary": {"path": "/tmp/trail", "sha256": "1" * 64,
                   "size_bytes": 123, "version": "trail 1.0.0"},
        "source": {"repo": "/tmp/source", "commit": "2" * 40,
                   "tree_clean": True, "submodules_clean": True,
                   "status_porcelain": [], "submodule_status": []},
        "fault_driver": {"path": "/tmp/fault-driver", "sha256": executed_driver_sha,
                         "expected_sha256": executed_driver_sha, "exact_expected": True,
                         "is_candidate_harness": True, "qualification_kind": "candidate_harness",
                         "attestation_path": "", "attestation_sha256": "",
                         "executed_evidence_path": "fault-driver-executed",
                         "executed_sha256": executed_driver_sha,
                         "executed_digest_verified_each_probe": True},
        "candidate_relationship": {"kind": "locally_bound_unproven_build",
                                   "expected_binary_sha256": "1" * 64,
                                   "expected_source_commit": "2" * 40},
    }
    (root / "environment.json").write_text(json.dumps(environment, sort_keys=True) + "\n", encoding="utf-8")
    empty_resources = {
        "schema_version": 1,
        "resources": {key: [] for key in (
            "lanes", "lane_branches", "lane_refs", "merge_queue", "initializations", "workspace_views",
            "leases", "observer_owners", "lock_paths", "socket_paths",
            "mount_paths", "workdir_paths", "materialization_journals",
        )},
    }
    active_resources = json.loads(json.dumps(empty_resources))
    active_resources["resources"]["lanes"] = [
        {"lane_id": f"id-scale-{index:04d}", "name": f"scale-{index:04d}"}
        for index in range(lanes)
    ]
    active_resources["resources"]["lane_branches"] = [
        {"lane_id": f"id-scale-{index:04d}", "ref_name": f"refs/lanes/scale-{index:04d}",
         "status": "active", "workdir": f"/tmp/scale-{index:04d}",
         "base_change": "basechange", "head_change": f"change-{index}"}
        for index in range(lanes)
    ]
    active_resources["resources"]["initializations"] = [
        {"initialization_id": f"init-{index:04d}", "lane_id": f"id-scale-{index:04d}",
         "lane_name": f"scale-{index:04d}", "phase": "observer_ready",
         "request_fingerprint": f"fingerprint-{index:04d}", "workdir": f"/tmp/scale-{index:04d}",
         "materialization_json": json.dumps({"workdir_mode": "native-cow"}, sort_keys=True)}
        for index in range(lanes)
    ]
    active_resources["resources"]["lane_refs"] = [
        {"name": f"refs/lanes/scale-{index:04d}", "change_id": f"change-{index}",
         "root_id": f"root-{index}", "operation_id": f"operation-{index}", "generation": 1}
        for index in range(lanes)
    ]
    active_resources["resources"]["lane_refs"].sort(key=lambda row: json.dumps(row, sort_keys=True, separators=(",", ":")))
    active_resources["resources"]["merge_queue"] = [
        {"queue_id": f"queue-{index}", "lane_id": f"id-scale-{index:04d}",
         "target_ref": "refs/branches/main", "status": "queued"}
        for index in range(lanes)
    ]
    active_resources["resources"]["merge_queue"].sort(key=lambda row: json.dumps(row, sort_keys=True, separators=(",", ":")))
    active_resources["resources"]["workdir_paths"] = [f"/tmp/scale-{index:04d}" for index in range(lanes)]
    final_resources = json.loads(json.dumps(empty_resources))
    final_resources["resources"]["lanes"] = [
        {"lane_id": f"id-scale-{index:04d}",
         "name": f"retired/scale-{index:04d}/id-scale-{index:04d}"}
        for index in range(lanes)
    ]
    final_resources["resources"]["lane_branches"] = [
        {"lane_id": f"id-scale-{index:04d}", "ref_name": f"retired/id-scale-{index:04d}/123",
         "status": "removed", "workdir": f"/tmp/scale-{index:04d}",
         "base_change": "basechange", "head_change": f"change-{index}"}
        for index in range(lanes)
    ]
    final_resources["resources"]["merge_queue"] = [
        {"queue_id": f"queue-{index}", "lane_id": f"id-scale-{index:04d}",
         "target_ref": "refs/branches/main", "status": "merged"}
        for index in range(lanes)
    ]
    for payload in (active_resources, final_resources):
        for key in payload["resources"]:
            payload["resources"][key].sort(key=lambda row: json.dumps(row, sort_keys=True, separators=(",", ":")))
    for name, payload in (("baseline-resources.json", empty_resources),
                          ("runtime-resources.json", empty_resources),
                          ("active-resources.json", active_resources),
                          ("final-resources.json", final_resources)):
        (root / name).write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")

    baseline_path_state = {
        "schema_version": 1, "tree": "b" * 40,
        "entries": [{"path": "README.md", "mode": "100644", "type": "blob", "object": "d" * 40}],
    }
    final_path_state = {
        "schema_version": 1, "tree": "e" * 40,
        "entries": sorted(baseline_path_state["entries"] + [
            {"path": path, "mode": "100644", "type": "blob", "object": f"{index + 1:040x}"}
            for index, path in enumerate(expected_paths)
        ], key=lambda row: row["path"]),
    }
    path_changes = {
        "schema_version": 1, "baseline_tree": "b" * 40, "final_tree": "e" * 40,
        "changes": [{"status": "A", "path": path} for path in expected_paths],
    }
    for name, payload in (("baseline-path-state.json", baseline_path_state),
                          ("final-path-state.json", final_path_state),
                          ("path-changes.json", path_changes)):
        (root / name).write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")
    untracked = {
        "schema_version": checker.UNTRACKED_SCHEMA_VERSION,
        "algorithm": "sha256",
        "entries": [
            {"path": ".trailignore", "type": "regular", "digest": "d" * 64},
        ],
    }
    for name in ("baseline-untracked.json", "final-untracked.json"):
        (root / name).write_text(json.dumps(untracked, sort_keys=True) + "\n", encoding="utf-8")

    def perf(count: int) -> dict[str, object]:
        return {"count": count, "p50_seconds": wall, "p95_seconds": wall,
                "p99_seconds": wall, "peak_rss_bytes": 4096}

    metrics = {
        "schema_version": checker.SCHEMA_VERSION,
        "run": {"run_id": run_id, "lanes": lanes, "files_per_lane": files,
                "concurrency": lanes, "fault_phase": "all", "latency_ceiling_seconds": 1.0},
        "baseline": {"trail_commit": "trail-change", "trail_source_commit": "2" * 40,
                     "trail_ref": "refs/branches/main",
                     "trail_root": "trail-root", "git_head": "a" * 40,
                     "git_branch": "main", "git_index_tree": "b" * 40,
                     "filesystem": "testfs", "repo_path": "/tmp/repo"},
        "correctness": {"lane_count": lanes, "edit_count": lanes * files,
                        "ambiguous_results": 0, "false_deletions": 0,
                        "missing_lanes": 0, "unintended_paths": 0,
                        "integrity_errors": 0, "live_locks": 0},
        "performance": {"spawn": perf(lanes), "record": perf(lanes),
                        "queue_run": perf(1), "git_export": perf(1),
                        "latency_ceiling_enforced": lanes <= 64},
        "storage": {"db_bytes_before": 1000, "db_bytes_after": 2000,
                    "observer_log_bytes_before": 0, "observer_log_bytes_after": 100,
                    "logical_lane_bytes": lanes * 100, "allocated_lane_bytes": lanes * 80,
                    "exclusive_lane_bytes": lanes * 10},
        "git_export": {"export_mode": "mapped_delta", "changed_path_count": lanes * files,
                       "commit_count": 1, "commit": "c" * 40, "parent": "a" * 40,
                       "dedicated_ref": "refs/heads/codex/trail-scale-contract",
                       "dedicated_ref_target": "c" * 40, "original_head_unchanged": True,
                       "original_branch_unchanged": True, "original_index_unchanged": True,
                       "dirty_refusal_code": "GIT_MAPPING_REQUIRED", "unexpected_path_count": 0},
        "cleanup": {"stale_mounts": 0, "stale_sockets": 0, "stale_locks": 0,
                    "stale_initializations": 0, "stale_materializations": 0,
                    "leaked_workdirs": 0, "stale_queue_rows": 0, "stale_lane_rows": 0,
                    "stale_lane_refs": 0},
        "audit_history": {"retired_lane_rows": lanes, "removed_lane_branch_rows": lanes,
                          "terminal_queue_rows": lanes},
        "integrity": {"trail_doctor": True, "trail_fsck": True,
                      "git_fsck": True, "conflict_control": True},
        "git_state_preservation": {"tracked_worktree_clean": True, "index_clean": True,
                                   "preexisting_untracked_count": 1,
                                   "final_untracked_count": 1,
                                   "preserved_untracked_count": 1,
                                   "added_untracked_count": 0,
                                   "removed_untracked_count": 0,
                                   "modified_untracked_count": 0},
        "evidence": {"result_rows": len(result_rows), "command_count": len(result_rows),
                     "fault_rows": len(fault_rows), "manifest_entries": 0},
    }
    (root / "metrics.json").write_text(json.dumps(metrics, sort_keys=True) + "\n", encoding="utf-8")
    # All files except the manifest itself are covered.
    metrics["evidence"]["manifest_entries"] = len([path for path in root.rglob("*") if path.is_file()])
    (root / "metrics.json").write_text(json.dumps(metrics, sort_keys=True) + "\n", encoding="utf-8")
    refresh_manifest(root)


class CheckerContractTests(unittest.TestCase):
    def artifact(self, lanes: int = 2, files: int = 2, wall: float = 0.2) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temp = tempfile.TemporaryDirectory(dir=TEMP_BASE)
        root = Path(temp.name)
        fixture(root, lanes, files, wall)
        return temp, root

    def test_accepts_complete_closed_schema(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        self.assertEqual(checker.check(root)["status"], "PASS")

    def test_missing_evidence_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        (root / "commands/spawn-0000.json").unlink()
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "missing command evidence"):
            checker.check(root)

    def test_unknown_metric_field_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        metrics = json.loads((root / "metrics.json").read_text())
        metrics["correctness"]["probably_ok"] = True
        (root / "metrics.json").write_text(json.dumps(metrics) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "keys mismatch"):
            checker.check(root)

    def test_changed_preexisting_untracked_digest_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        final = json.loads((root / "final-untracked.json").read_text())
        final["entries"][0]["digest"] = "e" * 64
        (root / "final-untracked.json").write_text(json.dumps(final) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "untracked state was not preserved"):
            checker.check(root)

    def test_unexpected_git_path_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        with (root / "final-git-paths.txt").open("a") as stream:
            stream.write("outside.txt\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "final Git path manifest"):
            checker.check(root)

    def test_cleanup_leak_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        metrics = json.loads((root / "metrics.json").read_text())
        metrics["cleanup"]["stale_locks"] = 1
        (root / "metrics.json").write_text(json.dumps(metrics) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "resource inventory"):
            checker.check(root)

    def test_raw_final_inventory_difference_is_rejected_even_when_metrics_claim_zero(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        final = json.loads((root / "final-resources.json").read_text())
        final["resources"]["socket_paths"] = ["/tmp/leaked.sock"]
        (root / "final-resources.json").write_text(json.dumps(final) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "resource inventory"):
            checker.check(root)

    def test_real_lane_retirement_and_terminal_queue_history_are_accepted(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        summary = checker.check(root)
        self.assertEqual(summary["status"], "PASS")

    def test_nonterminal_run_queue_history_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        final = json.loads((root / "final-resources.json").read_text())
        final["resources"]["merge_queue"][0]["status"] = "queued"
        (root / "final-resources.json").write_text(json.dumps(final, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "terminal queue"):
            checker.check(root)

    def test_materialization_journal_leak_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        final = json.loads((root / "final-resources.json").read_text())
        final["resources"]["materialization_journals"] = [
            {"path": "materialize-leak.json", "kind": "regular", "size_bytes": 12,
             "sha256": "a" * 64},
        ]
        (root / "final-resources.json").write_text(json.dumps(final, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "materialization"):
            checker.check(root)

    def test_resource_row_with_unknown_field_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        active = json.loads((root / "active-resources.json").read_text())
        active["resources"]["lanes"][0]["untrusted"] = "value"
        active["resources"]["lanes"].sort(key=lambda row: json.dumps(row, sort_keys=True, separators=(",", ":")))
        (root / "active-resources.json").write_text(json.dumps(active, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "keys mismatch"):
            checker.check(root)

    def test_zero_test_fault_probe_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        rows = checker.read_tsv(root / "faults.tsv", checker.FAULT_COLUMNS)
        rows[0]["test_count"] = "0"
        write_tsv(root / "faults.tsv", checker.FAULT_COLUMNS, rows)
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "exactly one"):
            checker.check(root)

    def test_candidate_fault_raw_output_must_prove_exactly_one_named_test(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        (root / "commands/fault-00.stderr").write_text(
            "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out\n"
        )
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "raw Cargo output"):
            checker.check(root)

    def test_wrong_platform_fault_test_name_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        rows = checker.read_tsv(root / "faults.tsv", checker.FAULT_COLUMNS)
        target = next(row for row in rows if row["scenario"] == "filesystem_replacement")
        target["test_target"] = "changed_path_ledger_linux"
        target["test_name"] = "wrong_test"
        write_tsv(root / "faults.tsv", checker.FAULT_COLUMNS, rows)
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "exact focused test"):
            checker.check(root)

    def test_unqualified_fault_driver_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        environment = json.loads((root / "environment.json").read_text())
        environment["fault_driver"]["exact_expected"] = False
        (root / "environment.json").write_text(json.dumps(environment, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "fault driver"):
            checker.check(root)

    def test_external_fault_driver_requires_embedded_exact_attestation(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        environment = json.loads((root / "environment.json").read_text())
        environment["fault_driver"].update({
            "is_candidate_harness": False,
            "qualification_kind": "external_attestation",
            "attestation_path": "/tmp/external-attestation.json",
        })
        attestation = {
            "schema_version": 1, "kind": "external_fault_driver",
            "fault_driver_sha256": environment["fault_driver"]["sha256"],
            "source_commit": environment["source"]["commit"],
            "binary_sha256": environment["binary"]["sha256"],
            "test_contract": "trail-task12-exact-one-v1",
        }
        attestation_path = root / "fault-attestation.json"
        attestation_path.write_text(json.dumps(attestation, sort_keys=True) + "\n")
        environment["fault_driver"]["attestation_sha256"] = hashlib.sha256(attestation_path.read_bytes()).hexdigest()
        (root / "environment.json").write_text(json.dumps(environment, sort_keys=True) + "\n")
        rows = checker.read_tsv(root / "faults.tsv", checker.FAULT_COLUMNS)
        for row in rows:
            if row["scenario"] != "dirty_git_export_refusal":
                row["evidence_kind"] = "externally_attested_focused_test"
        write_tsv(root / "faults.tsv", checker.FAULT_COLUMNS, rows)
        metrics = json.loads((root / "metrics.json").read_text())
        metrics["evidence"]["manifest_entries"] += 1
        (root / "metrics.json").write_text(json.dumps(metrics, sort_keys=True) + "\n")
        refresh_manifest(root)
        self.assertEqual(checker.check(root)["status"], "PASS")

        attestation_path.unlink()
        metrics["evidence"]["manifest_entries"] -= 1
        (root / "metrics.json").write_text(json.dumps(metrics, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "attestation evidence is missing"):
            checker.check(root)

    def test_dirty_candidate_source_for_cargo_faults_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        environment = json.loads((root / "environment.json").read_text())
        environment["source"]["tree_clean"] = False
        environment["source"]["status_porcelain"] = [" M trail/src/lib.rs"]
        (root / "environment.json").write_text(json.dumps(environment, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "clean source"):
            checker.check(root)

    def test_false_deletion_from_exact_path_state_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        final = json.loads((root / "final-path-state.json").read_text())
        final["entries"] = [row for row in final["entries"] if row["path"] != "README.md"]
        (root / "final-path-state.json").write_text(json.dumps(final, sort_keys=True) + "\n")
        changes = json.loads((root / "path-changes.json").read_text())
        changes["changes"].append({"status": "D", "path": "README.md"})
        changes["changes"].sort(key=lambda row: (row["path"], row["status"]))
        (root / "path-changes.json").write_text(json.dumps(changes, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "false.deletion"):
            checker.check(root)

    def test_active_inventory_missing_run_lane_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        active = json.loads((root / "active-resources.json").read_text())
        active["resources"]["lanes"].pop()
        (root / "active-resources.json").write_text(json.dumps(active) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "active inventory"):
            checker.check(root)

    def test_fault_binary_linkage_mismatch_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        rows = checker.read_tsv(root / "faults.tsv", checker.FAULT_COLUMNS)
        rows[0]["binary_sha256"] = "f" * 64
        write_tsv(root / "faults.tsv", checker.FAULT_COLUMNS, rows)
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "binary"):
            checker.check(root)

    def test_binary_metadata_is_required(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        environment = json.loads((root / "environment.json").read_text())
        del environment["binary"]["size_bytes"]
        (root / "environment.json").write_text(json.dumps(environment) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "binary"):
            checker.check(root)

    def test_baseline_trail_identity_must_match_raw_status(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        metrics = json.loads((root / "metrics.json").read_text())
        metrics["baseline"]["trail_commit"] = "candidate-source-confusion"
        (root / "metrics.json").write_text(json.dumps(metrics, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "baseline Trail identity"):
            checker.check(root)

    def test_baseline_source_identity_must_match_environment(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        metrics = json.loads((root / "metrics.json").read_text())
        metrics["baseline"]["trail_source_commit"] = "4" * 40
        (root / "metrics.json").write_text(json.dumps(metrics, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "candidate source"):
            checker.check(root)

    def test_baseline_git_tree_must_match_exact_path_state(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        metrics = json.loads((root / "metrics.json").read_text())
        metrics["baseline"]["git_index_tree"] = "9" * 40
        (root / "metrics.json").write_text(json.dumps(metrics, sort_keys=True) + "\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "baseline Git tree"):
            checker.check(root)

    def test_executed_fault_driver_bytes_must_match_claimed_digest(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        (root / "fault-driver-executed").write_bytes(b"mutated executed driver\n")
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "executed fault driver"):
            checker.check(root)

    def test_native_cow_fallback_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        rows = checker.read_tsv(root / "lanes.tsv", checker.LANE_COLUMNS)
        rows[0]["workdir_mode"] = "portable-copy"
        write_tsv(root / "lanes.tsv", checker.LANE_COLUMNS, rows)
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "not native-cow"):
            checker.check(root)

    def test_unstable_retry_identity_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        rows = checker.read_tsv(root / "lanes.tsv", checker.LANE_COLUMNS)
        rows[0]["retry_initialization_id"] = "ambiguous"
        write_tsv(root / "lanes.tsv", checker.LANE_COLUMNS, rows)
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "identity is missing or unstable"):
            checker.check(root)

    def test_incomplete_fault_matrix_is_rejected(self) -> None:
        temp, root = self.artifact()
        self.addCleanup(temp.cleanup)
        rows = checker.read_tsv(root / "faults.tsv", checker.FAULT_COLUMNS)[:-1]
        write_tsv(root / "faults.tsv", checker.FAULT_COLUMNS, rows)
        refresh_manifest(root)
        with self.assertRaisesRegex(checker.EvidenceError, "fault matrix"):
            checker.check(root)

    def test_64_lane_latency_ceiling_is_blocking(self) -> None:
        temp, root = self.artifact(lanes=64, files=1, wall=2.0)
        self.addCleanup(temp.cleanup)
        with self.assertRaisesRegex(checker.EvidenceError, "64-lane ceiling"):
            checker.check(root)

    def test_128_lane_records_latency_without_64_lane_ceiling(self) -> None:
        temp, root = self.artifact(lanes=128, files=1, wall=2.0)
        self.addCleanup(temp.cleanup)
        self.assertFalse(checker.check(root)["latency_ceiling_enforced"])


if __name__ == "__main__":
    unittest.main()

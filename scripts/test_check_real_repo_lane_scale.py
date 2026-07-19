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

    def command(command_id: str, phase: str, lane: str = "", seconds: float = wall) -> None:
        result_rows.append({
            "command_id": command_id, "phase": phase, "lane": lane,
            "wall_seconds": f"{seconds:.6f}", "peak_rss_bytes": "4096",
            "exit_code": "0", "committed": "true", "retry_of": "",
        })
        (commands / f"{command_id}.json").write_text("{}\n", encoding="utf-8")
        (commands / f"{command_id}.stdout").write_text("{}\n", encoding="utf-8")
        (commands / f"{command_id}.stderr").write_text("\n", encoding="utf-8")
        (commands / f"{command_id}.rss").write_text("4096\n", encoding="utf-8")

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

    fault_rows = []
    for index, scenario in enumerate(checker.FAULT_SCENARIOS):
        command_id = f"fault-{index:02d}"
        command(command_id, "fault")
        is_phase = scenario in checker.INITIALIZATION_PHASES
        identity = f"fault-init-{index:02d}" if is_phase else ""
        fault_rows.append({
            "scenario": scenario, "expected_code": "137" if is_phase else "0",
            "actual_code": "137" if is_phase else "0",
            "durable_phase": scenario.removeprefix("after_") if is_phase else "control",
            "committed": "true" if scenario in {"after_association", "after_reconciliation", "after_marker", "after_spawn_event"} else "false",
            "retry_result": "resumed_same_initialization" if is_phase else (
                "refused_without_mutation" if scenario in {"conflicting_lanes", "dirty_git_export_refusal"} else "recovered_once"
            ),
            "integrity_result": "ok", "leaked_resource_count": "0",
            "initialization_id": identity, "retry_initialization_id": identity,
            "evidence_command_id": command_id,
        })

    expected_paths.sort()
    for name in ("expected-paths.txt", "final-trail-paths.txt", "final-git-paths.txt"):
        (root / name).write_text("".join(f"{path}\n" for path in expected_paths), encoding="utf-8")
    write_tsv(root / "results.tsv", checker.RESULT_COLUMNS, result_rows)
    write_tsv(root / "lanes.tsv", checker.LANE_COLUMNS, lane_rows)
    write_tsv(root / "faults.tsv", checker.FAULT_COLUMNS, fault_rows)
    (root / "environment.json").write_text('{"platform":"contract"}\n', encoding="utf-8")
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
        "baseline": {"trail_commit": "trail-commit", "trail_ref": "trail-ref",
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
                    "leaked_workdirs": 0},
        "integrity": {"trail_doctor": "ok", "trail_fsck": "ok",
                      "git_fsck": "ok", "conflict_control": "ok"},
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
        with self.assertRaisesRegex(checker.EvidenceError, "leaked resources"):
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

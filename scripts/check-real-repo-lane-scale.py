#!/usr/bin/env python3
"""Fail-closed structural checker for real-repository Trail lane scale evidence."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import re
import sys
from pathlib import Path
from typing import Any, Iterable


SCHEMA_VERSION = 1
RESULT_COLUMNS = [
    "command_id", "phase", "lane", "wall_seconds", "peak_rss_bytes",
    "exit_code", "committed", "retry_of",
]
LANE_COLUMNS = [
    "lane", "initialization_id", "retry_initialization_id", "request_fingerprint",
    "retry_request_fingerprint", "workdir_mode", "workdir", "edit_count",
    "recorded_path_count", "isolation_unexpected_count", "logical_bytes",
    "allocated_bytes", "exclusive_bytes",
]
FAULT_COLUMNS = [
    "scenario", "expected_code", "actual_code", "durable_phase", "committed",
    "retry_result", "integrity_result", "leaked_resource_count",
    "initialization_id", "retry_initialization_id", "evidence_command_id",
]
INITIALIZATION_PHASES = [
    "after_reservation", "after_materialization", "after_association",
    "after_reconciliation", "after_marker", "after_spawn_event",
]
FAULT_SCENARIOS = INITIALIZATION_PHASES + [
    "daemon_death", "response_loss_after_association",
    "response_loss_after_readiness", "pid_reuse", "lock_holder_crash",
    "policy_churn", "filesystem_replacement", "disk_full",
    "permissions_failure", "fsync_failure", "conflicting_lanes",
    "dirty_git_export_refusal",
]
LANE_PHASES = ("spawn", "spawn_retry", "status", "space", "record", "readiness", "handoff")
ROOT_FILES = {
    "environment.json", "expected-paths.txt", "faults.tsv", "final-git-paths.txt",
    "final-trail-paths.txt", "lanes.tsv", "metrics.json", "results.tsv",
    "evidence-manifest.sha256",
}
DEDICATED_REF = re.compile(r"^refs/heads/codex/[A-Za-z0-9._/-]+$")


class EvidenceError(ValueError):
    pass


def fail(message: str) -> None:
    raise EvidenceError(message)


def read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read valid JSON {path.name}: {error}")
    if not isinstance(value, dict):
        fail(f"{path.name} must contain one JSON object")
    return value


def read_tsv(path: Path, columns: list[str]) -> list[dict[str, str]]:
    try:
        with path.open(encoding="utf-8", newline="") as stream:
            reader = csv.DictReader(stream, delimiter="\t")
            if reader.fieldnames != columns:
                fail(f"{path.name} columns must be exactly {columns}, got {reader.fieldnames}")
            rows = list(reader)
    except OSError as error:
        fail(f"cannot read {path.name}: {error}")
    for index, row in enumerate(rows, 2):
        if None in row or any(value is None for value in row.values()):
            fail(f"{path.name}:{index} is malformed or has extra columns")
    return rows


def exact_keys(value: dict[str, Any], keys: Iterable[str], label: str) -> None:
    expected = set(keys)
    actual = set(value)
    if actual != expected:
        fail(f"{label} keys mismatch: missing={sorted(expected-actual)} unknown={sorted(actual-expected)}")


def integer(value: Any, label: str, minimum: int = 0) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        fail(f"{label} must be an integer >= {minimum}")
    return value


def number(value: Any, label: str, minimum: float = 0.0) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        fail(f"{label} must be numeric")
    result = float(value)
    if not math.isfinite(result) or result < minimum:
        fail(f"{label} must be finite and >= {minimum}")
    return result


def boolean(value: Any, label: str) -> bool:
    if not isinstance(value, bool):
        fail(f"{label} must be boolean")
    return value


def parse_int(value: str, label: str, minimum: int = 0) -> int:
    if not re.fullmatch(r"[0-9]+", value):
        fail(f"{label} must be a decimal integer")
    result = int(value)
    if result < minimum:
        fail(f"{label} must be >= {minimum}")
    return result


def parse_float(value: str, label: str) -> float:
    try:
        result = float(value)
    except ValueError:
        fail(f"{label} must be numeric")
    if not math.isfinite(result) or result < 0:
        fail(f"{label} must be finite and non-negative")
    return result


def read_paths(path: Path) -> list[str]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(f"cannot read {path.name}: {error}")
    if any(not line or line.startswith("/") or "\t" in line or "\0" in line for line in lines):
        fail(f"{path.name} contains an invalid path")
    if lines != sorted(set(lines)):
        fail(f"{path.name} must be sorted and duplicate-free")
    return lines


def percentile(values: list[float], quantile: float) -> float:
    ordered = sorted(values)
    index = max(0, math.ceil(len(ordered) * quantile) - 1)
    return ordered[index]


def check_manifest(root: Path) -> set[str]:
    manifest_path = root / "evidence-manifest.sha256"
    entries: dict[str, str] = {}
    try:
        lines = manifest_path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(f"cannot read evidence manifest: {error}")
    for index, line in enumerate(lines, 1):
        match = re.fullmatch(r"([0-9a-f]{64})  ([^\0]+)", line)
        if not match:
            fail(f"evidence-manifest.sha256:{index} is malformed")
        digest, relative = match.groups()
        candidate = Path(relative)
        if candidate.is_absolute() or ".." in candidate.parts or relative == "evidence-manifest.sha256":
            fail(f"evidence manifest contains unsafe path {relative!r}")
        if relative in entries:
            fail(f"evidence manifest duplicates {relative}")
        target = root / candidate
        if not target.is_file() or target.is_symlink():
            fail(f"evidence manifest target is missing or not a regular file: {relative}")
        actual = hashlib.sha256(target.read_bytes()).hexdigest()
        if actual != digest:
            fail(f"evidence digest mismatch for {relative}")
        entries[relative] = digest
    actual_files = {
        str(path.relative_to(root)) for path in root.rglob("*")
        if path.is_file() and path.name not in {"checker.out", "checker.err", "task-11-report.md"}
    }
    expected_files = set(entries) | {"evidence-manifest.sha256"}
    if actual_files != expected_files:
        fail(f"evidence file set mismatch: missing={sorted(expected_files-actual_files)} unknown={sorted(actual_files-expected_files)}")
    if not ROOT_FILES.issubset(actual_files):
        fail(f"missing root evidence files: {sorted(ROOT_FILES-actual_files)}")
    return set(entries)


def check(root: Path) -> dict[str, Any]:
    if not root.is_dir() or root.is_symlink():
        fail("artifact directory must be a real directory")
    manifest_entries = check_manifest(root)
    metrics = read_json(root / "metrics.json")
    exact_keys(metrics, [
        "schema_version", "run", "baseline", "correctness", "performance", "storage",
        "git_export", "cleanup", "integrity", "evidence",
    ], "metrics")
    if metrics["schema_version"] != SCHEMA_VERSION:
        fail(f"metrics schema_version must be {SCHEMA_VERSION}")

    run = metrics["run"]
    if not isinstance(run, dict): fail("run must be an object")
    exact_keys(run, ["run_id", "lanes", "files_per_lane", "concurrency", "fault_phase", "latency_ceiling_seconds"], "run")
    lanes_expected = integer(run["lanes"], "run.lanes", 1)
    files_per_lane = integer(run["files_per_lane"], "run.files_per_lane", 1)
    concurrency = integer(run["concurrency"], "run.concurrency", 1)
    if concurrency > lanes_expected: fail("run.concurrency cannot exceed run.lanes")
    ceiling = number(run["latency_ceiling_seconds"], "run.latency_ceiling_seconds", 0.001)
    if run["fault_phase"] not in ["all", *FAULT_SCENARIOS]: fail("run.fault_phase is unsupported")
    if not isinstance(run["run_id"], str) or not re.fullmatch(r"[A-Za-z0-9._-]+", run["run_id"]): fail("run.run_id is invalid")

    baseline = metrics["baseline"]
    if not isinstance(baseline, dict): fail("baseline must be an object")
    exact_keys(baseline, ["trail_commit", "trail_ref", "trail_root", "git_head", "git_branch", "git_index_tree", "filesystem", "repo_path"], "baseline")
    for key, value in baseline.items():
        if not isinstance(value, str) or not value: fail(f"baseline.{key} must be a non-empty string")

    results = read_tsv(root / "results.tsv", RESULT_COLUMNS)
    command_ids: set[str] = set()
    phase_times: dict[str, list[float]] = {phase: [] for phase in ("spawn", "record", "queue_run", "git_export")}
    for index, row in enumerate(results, 2):
        command_id = row["command_id"]
        if not re.fullmatch(r"[A-Za-z0-9._-]+", command_id) or command_id in command_ids:
            fail(f"results.tsv:{index} has invalid or duplicate command_id")
        command_ids.add(command_id)
        wall = parse_float(row["wall_seconds"], f"results.tsv:{index} wall_seconds")
        parse_int(row["peak_rss_bytes"], f"results.tsv:{index} peak_rss_bytes", 1)
        exit_code = parse_int(row["exit_code"], f"results.tsv:{index} exit_code")
        if exit_code != 0: fail(f"command {command_id} failed with exit {exit_code}")
        if row["committed"] not in {"true", "false"}: fail(f"results.tsv:{index} committed must be true/false")
        if row["phase"] in phase_times: phase_times[row["phase"]].append(wall)
        for suffix in ("json", "stdout", "stderr", "rss"):
            relative = f"commands/{command_id}.{suffix}"
            if relative not in manifest_entries: fail(f"missing command evidence {relative}")

    lane_rows = read_tsv(root / "lanes.tsv", LANE_COLUMNS)
    if len(lane_rows) != lanes_expected: fail(f"expected {lanes_expected} lane rows, got {len(lane_rows)}")
    lane_names = [row["lane"] for row in lane_rows]
    if len(set(lane_names)) != lanes_expected or any(not re.fullmatch(r"scale-[0-9]{4}", name) for name in lane_names):
        fail("lane names must be unique deterministic scale-NNNN names")
    initialization_ids: set[str] = set()
    fingerprints: set[str] = set()
    for row in lane_rows:
        lane = row["lane"]
        if not row["initialization_id"] or row["initialization_id"] != row["retry_initialization_id"]:
            fail(f"{lane}: initialization identity is missing or unstable across retry")
        if row["initialization_id"] in initialization_ids: fail(f"{lane}: initialization identity is not unique")
        initialization_ids.add(row["initialization_id"])
        if not row["request_fingerprint"] or row["request_fingerprint"] != row["retry_request_fingerprint"]:
            fail(f"{lane}: request fingerprint is missing or unstable")
        if row["request_fingerprint"] in fingerprints: fail(f"{lane}: request fingerprint is not unique")
        fingerprints.add(row["request_fingerprint"])
        if row["workdir_mode"] != "native-cow": fail(f"{lane}: resolved workdir mode is not native-cow")
        if not Path(row["workdir"]).is_absolute(): fail(f"{lane}: workdir is not absolute")
        if parse_int(row["edit_count"], f"{lane} edit_count") != files_per_lane: fail(f"{lane}: wrong edit count")
        if parse_int(row["recorded_path_count"], f"{lane} recorded_path_count") != files_per_lane: fail(f"{lane}: wrong recorded path count")
        if parse_int(row["isolation_unexpected_count"], f"{lane} isolation") != 0: fail(f"{lane}: isolation violation")
        parse_int(row["logical_bytes"], f"{lane} logical_bytes", 1)
        parse_int(row["allocated_bytes"], f"{lane} allocated_bytes", 1)
        parse_int(row["exclusive_bytes"], f"{lane} exclusive_bytes")
        for phase in LANE_PHASES:
            matches = [r for r in results if r["lane"] == lane and r["phase"] == phase]
            if len(matches) != 1: fail(f"{lane}: expected exactly one {phase} command, got {len(matches)}")

    expected_paths = read_paths(root / "expected-paths.txt")
    final_trail_paths = read_paths(root / "final-trail-paths.txt")
    final_git_paths = read_paths(root / "final-git-paths.txt")
    expected_count = lanes_expected * files_per_lane
    if len(expected_paths) != expected_count: fail(f"expected-paths count must be exactly {expected_count}")
    if expected_paths != final_trail_paths: fail("final Trail path manifest differs from expected paths")
    if expected_paths != final_git_paths: fail("final Git path manifest differs from expected paths")
    for lane in lane_names:
        prefix = f".trail-scale/{run['run_id']}/{lane}/"
        lane_paths = [path for path in expected_paths if path.startswith(prefix)]
        if len(lane_paths) != files_per_lane: fail(f"{lane}: expected-path allocation is not exact/disjoint")

    correctness = metrics["correctness"]
    if not isinstance(correctness, dict): fail("correctness must be an object")
    exact_keys(correctness, ["lane_count", "edit_count", "ambiguous_results", "false_deletions", "missing_lanes", "unintended_paths", "integrity_errors", "live_locks"], "correctness")
    if integer(correctness["lane_count"], "correctness.lane_count") != lanes_expected: fail("correctness lane count mismatch")
    if integer(correctness["edit_count"], "correctness.edit_count") != expected_count: fail("correctness edit count mismatch")
    for key in ["ambiguous_results", "false_deletions", "missing_lanes", "unintended_paths", "integrity_errors", "live_locks"]:
        if integer(correctness[key], f"correctness.{key}") != 0: fail(f"correctness.{key} must be zero")

    performance = metrics["performance"]
    if not isinstance(performance, dict): fail("performance must be an object")
    exact_keys(performance, ["spawn", "record", "queue_run", "git_export", "latency_ceiling_enforced"], "performance")
    enforce_latency = lanes_expected <= 64
    if boolean(performance["latency_ceiling_enforced"], "performance.latency_ceiling_enforced") != enforce_latency:
        fail("latency ceiling enforcement must be enabled through 64 lanes and disabled above 64")
    for phase in ("spawn", "record", "queue_run", "git_export"):
        values = phase_times[phase]
        expected_samples = lanes_expected if phase in {"spawn", "record"} else 1
        if len(values) != expected_samples: fail(f"{phase}: expected {expected_samples} timing samples, got {len(values)}")
        report = performance[phase]
        if not isinstance(report, dict): fail(f"performance.{phase} must be an object")
        exact_keys(report, ["count", "p50_seconds", "p95_seconds", "p99_seconds", "peak_rss_bytes"], f"performance.{phase}")
        if integer(report["count"], f"performance.{phase}.count") != expected_samples: fail(f"performance.{phase}.count mismatch")
        for key, quantile in (("p50_seconds", .50), ("p95_seconds", .95), ("p99_seconds", .99)):
            observed = number(report[key], f"performance.{phase}.{key}")
            computed = percentile(values, quantile)
            if abs(observed - computed) > 0.001: fail(f"performance.{phase}.{key} does not match results.tsv")
        integer(report["peak_rss_bytes"], f"performance.{phase}.peak_rss_bytes", 1)
        if enforce_latency and number(report["p99_seconds"], f"performance.{phase}.p99_seconds") > ceiling:
            fail(f"performance.{phase}.p99_seconds exceeds {ceiling}s 64-lane ceiling")

    storage = metrics["storage"]
    if not isinstance(storage, dict): fail("storage must be an object")
    exact_keys(storage, ["db_bytes_before", "db_bytes_after", "observer_log_bytes_before", "observer_log_bytes_after", "logical_lane_bytes", "allocated_lane_bytes", "exclusive_lane_bytes"], "storage")
    for key in storage:
        integer(storage[key], f"storage.{key}")
    if storage["db_bytes_before"] <= 0 or storage["db_bytes_after"] <= 0: fail("database byte evidence must be non-zero")
    if storage["logical_lane_bytes"] != sum(int(row["logical_bytes"]) for row in lane_rows): fail("logical lane bytes do not match lanes.tsv")
    if storage["allocated_lane_bytes"] != sum(int(row["allocated_bytes"]) for row in lane_rows): fail("allocated lane bytes do not match lanes.tsv")
    if storage["exclusive_lane_bytes"] != sum(int(row["exclusive_bytes"]) for row in lane_rows): fail("exclusive lane bytes do not match lanes.tsv")

    git_export = metrics["git_export"]
    if not isinstance(git_export, dict): fail("git_export must be an object")
    exact_keys(git_export, ["export_mode", "changed_path_count", "commit_count", "commit", "parent", "dedicated_ref", "dedicated_ref_target", "original_head_unchanged", "original_branch_unchanged", "original_index_unchanged", "dirty_refusal_code", "unexpected_path_count"], "git_export")
    if git_export["export_mode"] != "mapped_delta": fail("Git export mode must be mapped_delta")
    if integer(git_export["changed_path_count"], "git_export.changed_path_count") != expected_count: fail("Git changed-path count mismatch")
    if integer(git_export["commit_count"], "git_export.commit_count") != 1: fail("Git export must create exactly one commit")
    for key in ["commit", "parent", "dedicated_ref_target"]:
        if not isinstance(git_export[key], str) or not re.fullmatch(r"[0-9a-f]{40,64}", git_export[key]): fail(f"git_export.{key} is not an object id")
    if git_export["commit"] != git_export["dedicated_ref_target"]: fail("dedicated ref does not target export commit")
    if not isinstance(git_export["dedicated_ref"], str) or not DEDICATED_REF.fullmatch(git_export["dedicated_ref"]): fail("Git export ref is not dedicated refs/heads/codex/... ref")
    for key in ["original_head_unchanged", "original_branch_unchanged", "original_index_unchanged"]:
        if not boolean(git_export[key], f"git_export.{key}"): fail(f"git_export.{key} must be true")
    if git_export["dirty_refusal_code"] not in {"GIT_MAPPING_REQUIRED", "GIT_DIRTY", "GIT_ERROR"}: fail("dirty Git refusal code is missing or unstable")
    if integer(git_export["unexpected_path_count"], "git_export.unexpected_path_count") != 0: fail("Git export contains unexpected paths")

    fault_rows = read_tsv(root / "faults.tsv", FAULT_COLUMNS)
    expected_faults = FAULT_SCENARIOS if run["fault_phase"] == "all" else [run["fault_phase"]]
    if [row["scenario"] for row in fault_rows] != expected_faults: fail("fault matrix is missing, duplicated, unordered, or contains unknown scenarios")
    for row in fault_rows:
        scenario = row["scenario"]
        if not row["expected_code"] or row["actual_code"] != row["expected_code"]: fail(f"{scenario}: actual fault code differs from expected")
        if row["committed"] not in {"true", "false"}: fail(f"{scenario}: committed must be true/false")
        if row["integrity_result"] != "ok": fail(f"{scenario}: integrity result is not ok")
        if parse_int(row["leaked_resource_count"], f"{scenario} leaked_resource_count") != 0: fail(f"{scenario}: leaked resources remain")
        if row["evidence_command_id"] not in command_ids: fail(f"{scenario}: evidence command is missing")
        if scenario in INITIALIZATION_PHASES:
            if row["durable_phase"] != scenario.removeprefix("after_"): fail(f"{scenario}: durable phase is wrong")
            if not row["initialization_id"] or row["initialization_id"] != row["retry_initialization_id"]: fail(f"{scenario}: retry changed initialization identity")
            if row["retry_result"] not in {"resumed_same_initialization", "repaired_once"}: fail(f"{scenario}: retry result is not a committed repair/resume")
        elif row["retry_result"] not in {"resumed_same_initialization", "repaired_once", "refused_without_mutation", "recovered_once"}:
            fail(f"{scenario}: unsupported retry result")

    cleanup = metrics["cleanup"]
    if not isinstance(cleanup, dict): fail("cleanup must be an object")
    exact_keys(cleanup, ["stale_mounts", "stale_sockets", "stale_locks", "stale_initializations", "stale_materializations", "leaked_workdirs"], "cleanup")
    if any(integer(value, f"cleanup.{key}") != 0 for key, value in cleanup.items()): fail("cleanup evidence reports leaked resources")
    integrity = metrics["integrity"]
    if not isinstance(integrity, dict): fail("integrity must be an object")
    exact_keys(integrity, ["trail_doctor", "trail_fsck", "git_fsck", "conflict_control"], "integrity")
    if any(value != "ok" for value in integrity.values()): fail("doctor/fsck/conflict integrity control failed")

    evidence = metrics["evidence"]
    if not isinstance(evidence, dict): fail("evidence must be an object")
    exact_keys(evidence, ["result_rows", "command_count", "fault_rows", "manifest_entries"], "evidence")
    if integer(evidence["result_rows"], "evidence.result_rows") != len(results): fail("evidence result row count mismatch")
    if integer(evidence["command_count"], "evidence.command_count") != len(command_ids): fail("evidence command count mismatch")
    if integer(evidence["fault_rows"], "evidence.fault_rows") != len(fault_rows): fail("evidence fault row count mismatch")
    if integer(evidence["manifest_entries"], "evidence.manifest_entries") != len(manifest_entries): fail("evidence manifest entry count mismatch")

    return {"status": "PASS", "lanes": lanes_expected, "edits": expected_count, "commands": len(results), "faults": len(fault_rows), "latency_ceiling_enforced": enforce_latency}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_dir", type=Path)
    args = parser.parse_args(argv)
    try:
        summary = check(args.artifact_dir.resolve())
    except EvidenceError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

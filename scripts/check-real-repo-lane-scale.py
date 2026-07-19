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


SCHEMA_VERSION = 4
UNTRACKED_SCHEMA_VERSION = 1
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
    "evidence_kind", "source_commit", "binary_sha256", "binary_exercised",
    "test_target", "test_name", "test_count",
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
    "active-resources.json", "baseline-resources.json", "baseline-untracked.json",
    "baseline-path-state.json", "environment.json", "expected-paths.txt", "faults.tsv",
    "final-path-state.json", "final-resources.json", "path-changes.json", "runtime-resources.json",
    "final-git-paths.txt", "final-trail-paths.txt", "final-untracked.json", "lanes.tsv",
    "metrics.json", "results.tsv", "evidence-manifest.sha256",
}
RESOURCE_KEYS = [
    "lanes", "lane_branches", "lane_refs", "merge_queue", "initializations", "workspace_views", "leases",
    "observer_owners", "lock_paths", "socket_paths", "mount_paths", "workdir_paths",
    "materialization_journals",
]
DEDICATED_REF = re.compile(r"^refs/heads/codex/[A-Za-z0-9._/-]+$")
FAULT_TESTS = {
    **{scenario: ("lane_initialization_faults", "identical_spawn_resumes_at_every_durable_crash_cut")
       for scenario in INITIALIZATION_PHASES},
    "daemon_death": ("changed_path_ledger_daemon", "killed_daemon_is_replaced_and_full_reconciliation_captures_offline_change"),
    "response_loss_after_association": ("changed_path_ledger_daemon", "external_lane_spawn_ignores_daemon_response_delay_without_duplicate_fallback"),
    "response_loss_after_readiness": ("changed_path_ledger_daemon", "external_lane_spawn_ignores_daemon_response_delay_without_duplicate_fallback"),
    "pid_reuse": ("changed_path_ledger_daemon", "forged_dead_process_identity_cannot_replace_a_live_observer_owner"),
    "lock_holder_crash": ("changed_path_ledger_daemon", "crash_after_persisting_ledger_owner_is_automatically_recovered"),
    "policy_churn": ("changed_path_ledger_daemon", "live_policy_invalidation_self_restarts_and_reconciles"),
    "disk_full": ("lane_initialization_faults", "io_failures_never_advance_past_or_delete_the_durable_artifact"),
    "permissions_failure": ("lane_initialization_faults", "io_failures_never_advance_past_or_delete_the_durable_artifact"),
    "fsync_failure": ("lane_initialization_faults", "io_failures_never_advance_past_or_delete_the_durable_artifact"),
    "conflicting_lanes": ("e2e", "lane_merge_queue_pauses_on_conflict"),
}

RESOURCE_ROW_KEYS = {
    "lanes": {"lane_id", "name"},
    "lane_branches": {"lane_id", "ref_name", "status", "workdir", "base_change", "head_change"},
    "lane_refs": {"name", "change_id", "root_id", "operation_id", "generation"},
    "merge_queue": {"queue_id", "lane_id", "target_ref", "status"},
    "initializations": {"initialization_id", "lane_name", "lane_id", "request_fingerprint", "phase", "workdir", "materialization_json"},
    "workspace_views": {"view_id", "lane_id", "backend", "mountpoint", "source_upper", "generated_upper", "scratch_upper", "meta_dir", "journal_path", "status", "owner_pid"},
    "leases": {"lease_id", "lane_id", "ref_name", "path", "mode", "expires_at"},
    "observer_owners": {"scope_id", "lease_state", "daemon_pid"},
    "materialization_journals": {"path", "kind", "size_bytes", "sha256"},
}


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


def read_untracked_snapshot(path: Path) -> list[dict[str, str]]:
    snapshot = read_json(path)
    exact_keys(snapshot, ["schema_version", "algorithm", "entries"], path.name)
    if snapshot["schema_version"] != UNTRACKED_SCHEMA_VERSION:
        fail(f"{path.name} schema_version must be {UNTRACKED_SCHEMA_VERSION}")
    if snapshot["algorithm"] != "sha256":
        fail(f"{path.name} algorithm must be sha256")
    entries = snapshot["entries"]
    if not isinstance(entries, list):
        fail(f"{path.name} entries must be an array")
    paths: list[str] = []
    allowed_types = {"regular", "symlink", "fifo", "socket", "block_device", "char_device", "other"}
    for index, entry in enumerate(entries):
        label = f"{path.name}.entries[{index}]"
        if not isinstance(entry, dict):
            fail(f"{label} must be an object")
        exact_keys(entry, ["path", "type", "digest"], label)
        relative = entry["path"]
        if (not isinstance(relative, str) or not relative or relative.startswith("/")
                or "\0" in relative or relative == ".trail" or relative.startswith(".trail/")):
            fail(f"{label}.path is invalid or Trail-internal")
        if Path(relative).is_absolute() or ".." in Path(relative).parts:
            fail(f"{label}.path is unsafe")
        if entry["type"] not in allowed_types:
            fail(f"{label}.type is unsupported")
        if not isinstance(entry["digest"], str) or not re.fullmatch(r"[0-9a-f]{64}", entry["digest"]):
            fail(f"{label}.digest is not sha256")
        paths.append(relative)
    if paths != sorted(set(paths)):
        fail(f"{path.name} entries must be path-sorted and duplicate-free")
    return entries


def read_resource_inventory(path: Path) -> dict[str, list[Any]]:
    inventory = read_json(path)
    exact_keys(inventory, ["schema_version", "resources"], path.name)
    if inventory["schema_version"] != 1:
        fail(f"{path.name} schema_version must be 1")
    resources = inventory["resources"]
    if not isinstance(resources, dict):
        fail(f"{path.name}.resources must be an object")
    exact_keys(resources, RESOURCE_KEYS, f"{path.name}.resources")
    for key, rows in resources.items():
        if not isinstance(rows, list):
            fail(f"{path.name}.resources.{key} must be an array")
        encoded = [json.dumps(row, sort_keys=True, separators=(",", ":")) for row in rows]
        if encoded != sorted(set(encoded)):
            fail(f"{path.name}.resources.{key} must be sorted and duplicate-free")
    for key, keys in RESOURCE_ROW_KEYS.items():
        for index, row in enumerate(resources[key]):
            label = f"{path.name}.resources.{key}[{index}]"
            if not isinstance(row, dict):
                fail(f"{label} must be an object")
            exact_keys(row, keys, label)
            for field, value in row.items():
                if field in {"generation", "expires_at", "owner_pid", "daemon_pid", "size_bytes"}:
                    if value is not None:
                        integer(value, f"{label}.{field}")
                elif value is not None and (not isinstance(value, str) or not value):
                    fail(f"{label}.{field} must be a non-empty string or null")
            if key == "materialization_journals":
                if row["kind"] not in {"regular", "directory", "symlink", "other"}:
                    fail(f"{label}.kind is unsupported")
                if row["kind"] == "regular":
                    if not isinstance(row["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", row["sha256"]):
                        fail(f"{label}.sha256 is invalid")
                elif row["sha256"] is not None:
                    fail(f"{label}.sha256 must be null for non-regular paths")
    for key in ["lock_paths", "socket_paths", "mount_paths", "workdir_paths"]:
        if any(not isinstance(row, str) or not row for row in resources[key]):
            fail(f"{path.name}.resources.{key} rows must be non-empty paths")
    return resources


def read_path_state(path: Path) -> tuple[str, list[dict[str, str]]]:
    state = read_json(path)
    exact_keys(state, ["schema_version", "tree", "entries"], path.name)
    if state["schema_version"] != 1:
        fail(f"{path.name} schema_version must be 1")
    if not isinstance(state["tree"], str) or not re.fullmatch(r"[0-9a-f]{40,64}", state["tree"]):
        fail(f"{path.name}.tree is invalid")
    entries = state["entries"]
    if not isinstance(entries, list):
        fail(f"{path.name}.entries must be an array")
    paths: list[str] = []
    for index, row in enumerate(entries):
        label = f"{path.name}.entries[{index}]"
        if not isinstance(row, dict):
            fail(f"{label} must be an object")
        exact_keys(row, ["path", "mode", "type", "object"], label)
        if (not isinstance(row["path"], str) or not row["path"] or row["path"].startswith("/")
                or "\0" in row["path"] or ".." in Path(row["path"]).parts):
            fail(f"{label}.path is unsafe")
        if not isinstance(row["mode"], str) or not re.fullmatch(r"[0-7]{6}", row["mode"]):
            fail(f"{label}.mode is invalid")
        if row["type"] not in {"blob", "tree", "commit"}:
            fail(f"{label}.type is unsupported")
        if not isinstance(row["object"], str) or not re.fullmatch(r"[0-9a-f]{40,64}", row["object"]):
            fail(f"{label}.object is invalid")
        paths.append(row["path"])
    if paths != sorted(set(paths)):
        fail(f"{path.name} entries must be path-sorted and duplicate-free")
    return state["tree"], entries


def read_path_changes(path: Path) -> tuple[str, str, list[dict[str, str]]]:
    value = read_json(path)
    exact_keys(value, ["schema_version", "baseline_tree", "final_tree", "changes"], path.name)
    if value["schema_version"] != 1:
        fail(f"{path.name} schema_version must be 1")
    for key in ["baseline_tree", "final_tree"]:
        if not isinstance(value[key], str) or not re.fullmatch(r"[0-9a-f]{40,64}", value[key]):
            fail(f"{path.name}.{key} is invalid")
    changes = value["changes"]
    if not isinstance(changes, list):
        fail(f"{path.name}.changes must be an array")
    for index, row in enumerate(changes):
        label = f"{path.name}.changes[{index}]"
        if not isinstance(row, dict):
            fail(f"{label} must be an object")
        exact_keys(row, ["status", "path"], label)
        if row["status"] not in {"A", "M", "D", "T"}:
            fail(f"{label}.status is unsupported")
        if not isinstance(row["path"], str) or not row["path"]:
            fail(f"{label}.path is invalid")
    if changes != sorted(changes, key=lambda row: (row["path"], row["status"])):
        fail(f"{path.name}.changes must be sorted")
    return value["baseline_tree"], value["final_tree"], changes


def resource_added_count(before: dict[str, list[Any]], after: dict[str, list[Any]], key: str) -> int:
    encode = lambda rows: {json.dumps(row, sort_keys=True, separators=(",", ":")) for row in rows}
    return len(encode(after[key]) - encode(before[key]))


def check_environment(path: Path) -> dict[str, Any]:
    environment = read_json(path)
    exact_keys(environment, ["schema_version", "platform", "filesystem", "binary", "source", "fault_driver", "candidate_relationship"], "environment")
    if environment["schema_version"] != 2: fail("environment schema_version must be 2")
    platform_data = environment["platform"]
    exact_keys(platform_data, ["description", "machine", "python"], "environment.platform")
    if any(not isinstance(value, str) or not value for value in platform_data.values()): fail("environment.platform values must be non-empty strings")
    filesystem = environment["filesystem"]
    exact_keys(filesystem, ["repo_device", "output_device", "same_device", "repo_filesystem", "output_filesystem"], "environment.filesystem")
    integer(filesystem["repo_device"], "environment.filesystem.repo_device")
    integer(filesystem["output_device"], "environment.filesystem.output_device")
    if not boolean(filesystem["same_device"], "environment.filesystem.same_device"): fail("native-cow evidence requires the same device")
    if filesystem["repo_device"] != filesystem["output_device"]: fail("filesystem device identities differ")
    for key in ["repo_filesystem", "output_filesystem"]:
        if not isinstance(filesystem[key], str) or not filesystem[key]: fail(f"environment.filesystem.{key} is missing")
    binary = environment["binary"]
    exact_keys(binary, ["path", "sha256", "size_bytes", "version"], "environment.binary")
    if not isinstance(binary["path"], str) or not Path(binary["path"]).is_absolute(): fail("environment.binary.path must be absolute")
    if not isinstance(binary["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", binary["sha256"]): fail("environment.binary.sha256 is invalid")
    integer(binary["size_bytes"], "environment.binary.size_bytes", 1)
    if not isinstance(binary["version"], str) or not binary["version"] or "\n" in binary["version"]: fail("environment.binary.version is invalid")
    source = environment["source"]
    exact_keys(source, ["repo", "commit", "tree_clean", "submodules_clean", "status_porcelain", "submodule_status"], "environment.source")
    if not isinstance(source["repo"], str) or not Path(source["repo"]).is_absolute(): fail("environment.source.repo must be absolute")
    if not isinstance(source["commit"], str) or not re.fullmatch(r"[0-9a-f]{40,64}", source["commit"]): fail("environment.source.commit is invalid")
    boolean(source["tree_clean"], "environment.source.tree_clean"); boolean(source["submodules_clean"], "environment.source.submodules_clean")
    if not all(isinstance(source[key], list) and all(isinstance(row, str) for row in source[key]) for key in ["status_porcelain", "submodule_status"]): fail("environment source cleanliness disclosure is malformed")
    derived_tree_clean = not any(not row.startswith("??") for row in source["status_porcelain"])
    derived_submodules_clean = not any(row[:1] in {"+", "-", "U"} for row in source["submodule_status"])
    if source["tree_clean"] != derived_tree_clean or source["submodules_clean"] != derived_submodules_clean:
        fail("environment source cleanliness booleans disagree with raw disclosure")
    fault_driver = environment["fault_driver"]
    exact_keys(fault_driver, ["path", "sha256", "expected_sha256", "exact_expected",
                              "is_candidate_harness", "qualification_kind",
                              "attestation_path", "attestation_sha256"], "environment.fault_driver")
    if not isinstance(fault_driver["path"], str) or not Path(fault_driver["path"]).is_absolute(): fail("environment.fault_driver.path must be absolute")
    if not isinstance(fault_driver["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", fault_driver["sha256"]): fail("environment.fault_driver.sha256 is invalid")
    if fault_driver["expected_sha256"] != fault_driver["sha256"]:
        fail("environment fault driver does not match the exact expected digest")
    if not boolean(fault_driver["exact_expected"], "environment.fault_driver.exact_expected"):
        fail("environment fault driver is not exact")
    candidate_driver = boolean(fault_driver["is_candidate_harness"], "environment.fault_driver.is_candidate_harness")
    if fault_driver["qualification_kind"] == "candidate_harness":
        if not candidate_driver or fault_driver["attestation_path"] or fault_driver["attestation_sha256"]:
            fail("candidate harness fault driver linkage is malformed")
        if not source["tree_clean"] or not source["submodules_clean"]:
            fail("Cargo fault probes require a clean source tree and clean submodules")
    elif fault_driver["qualification_kind"] == "external_attestation":
        if candidate_driver:
            fail("external fault attestation cannot claim the candidate harness")
        if (not isinstance(fault_driver["attestation_path"], str)
                or not Path(fault_driver["attestation_path"]).is_absolute()
                or not isinstance(fault_driver["attestation_sha256"], str)
                or not re.fullmatch(r"[0-9a-f]{64}", fault_driver["attestation_sha256"])):
            fail("external fault driver attestation is malformed")
        attestation_file = path.parent / "fault-attestation.json"
        try:
            attestation_digest = hashlib.sha256(attestation_file.read_bytes()).hexdigest()
        except OSError as error:
            fail(f"external fault attestation evidence is missing: {error}")
        if attestation_digest != fault_driver["attestation_sha256"]:
            fail("external fault attestation evidence digest mismatch")
        attestation = read_json(attestation_file)
        exact_keys(attestation, ["schema_version", "kind", "fault_driver_sha256",
                                 "source_commit", "binary_sha256", "test_contract"],
                   "fault-attestation")
        expected_attestation = {
            "schema_version": 1,
            "kind": "external_fault_driver",
            "fault_driver_sha256": fault_driver["sha256"],
            "source_commit": source["commit"],
            "binary_sha256": binary["sha256"],
            "test_contract": "trail-task12-exact-one-v1",
        }
        if attestation != expected_attestation:
            fail("external fault attestation does not bind the exact candidate/driver contract")
    else:
        fail("environment fault driver qualification is unsupported")
    relationship = environment["candidate_relationship"]
    exact_keys(relationship, ["kind", "expected_binary_sha256", "expected_source_commit"], "environment.candidate_relationship")
    if relationship["kind"] not in {"locally_bound_unproven_build", "verified_reproducible_build"}: fail("candidate binary/source relationship is unsupported")
    if relationship["expected_binary_sha256"] != binary["sha256"] or relationship["expected_source_commit"] != source["commit"]: fail("candidate relationship does not bind exact binary and source")
    return environment


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
    environment = check_environment(root / "environment.json")
    baseline_resources = read_resource_inventory(root / "baseline-resources.json")
    runtime_resources = read_resource_inventory(root / "runtime-resources.json")
    active_resources = read_resource_inventory(root / "active-resources.json")
    final_resources = read_resource_inventory(root / "final-resources.json")
    runtime_mutable = {"observer_owners", "socket_paths", "lock_paths", "leases"}
    unexpected_runtime = [key for key in RESOURCE_KEYS
                          if key not in runtime_mutable and runtime_resources[key] != baseline_resources[key]]
    if unexpected_runtime:
        fail(f"daemon-backed status changed unexpected runtime resources: {unexpected_runtime}")
    metrics = read_json(root / "metrics.json")
    exact_keys(metrics, [
        "schema_version", "run", "baseline", "correctness", "performance", "storage",
        "git_export", "cleanup", "audit_history", "integrity", "git_state_preservation", "evidence",
    ], "metrics")
    if metrics["schema_version"] != SCHEMA_VERSION:
        fail(f"metrics schema_version must be {SCHEMA_VERSION}")

    baseline_untracked = read_untracked_snapshot(root / "baseline-untracked.json")
    final_untracked = read_untracked_snapshot(root / "final-untracked.json")
    baseline_by_path = {entry["path"]: entry for entry in baseline_untracked}
    final_by_path = {entry["path"]: entry for entry in final_untracked}
    added_untracked = sorted(final_by_path.keys() - baseline_by_path.keys())
    removed_untracked = sorted(baseline_by_path.keys() - final_by_path.keys())
    modified_untracked = sorted(
        path for path in baseline_by_path.keys() & final_by_path.keys()
        if baseline_by_path[path] != final_by_path[path]
    )
    preserved_untracked = sum(
        baseline_by_path[path] == final_by_path[path]
        for path in baseline_by_path.keys() & final_by_path.keys()
    )
    if added_untracked or removed_untracked or modified_untracked:
        fail(
            "untracked state was not preserved: "
            f"added={added_untracked} removed={removed_untracked} modified={modified_untracked}"
        )

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

    active_lanes = {row.get("name"): row for row in active_resources["lanes"]}
    if set(lane_names) - set(active_lanes):
        fail(f"active inventory is missing run lanes: {sorted(set(lane_names)-set(active_lanes))}")
    active_initializations = {row.get("lane_name"): row for row in active_resources["initializations"]}
    active_branches = {row.get("lane_id"): row for row in active_resources["lane_branches"]}
    active_lane_refs = {row.get("name") for row in active_resources["lane_refs"]}
    active_queue_by_lane = {}
    for queue_row in active_resources["merge_queue"]:
        active_queue_by_lane.setdefault(queue_row.get("lane_id"), []).append(queue_row)
    for row in lane_rows:
        initialization = active_initializations.get(row["lane"])
        if not initialization:
            fail(f"active inventory is missing initialization for {row['lane']}")
        expected = {
            "initialization_id": row["initialization_id"],
            "request_fingerprint": row["request_fingerprint"],
            "workdir": row["workdir"],
        }
        if any(initialization.get(key) != value for key, value in expected.items()):
            fail(f"active inventory initialization does not match lanes.tsv for {row['lane']}")
        if initialization.get("phase") != "observer_ready":
            fail(f"active inventory initialization is not observer_ready for {row['lane']}")
        materialization = initialization.get("materialization_json")
        if not isinstance(materialization, str) or not materialization:
            fail(f"active inventory is missing durable materialization evidence for {row['lane']}")
        try:
            if not isinstance(json.loads(materialization), dict):
                fail(f"active inventory materialization evidence is malformed for {row['lane']}")
        except json.JSONDecodeError:
            fail(f"active inventory materialization evidence is malformed for {row['lane']}")
        if row["workdir"] not in active_resources["workdir_paths"]:
            fail(f"active inventory is missing workdir for {row['lane']}")
        if f"refs/lanes/{row['lane']}" not in active_lane_refs:
            fail(f"active inventory is missing lane ref for {row['lane']}")
        lane_id = active_lanes[row["lane"]].get("lane_id")
        branch = active_branches.get(lane_id)
        if (not branch or branch.get("ref_name") != f"refs/lanes/{row['lane']}"
                or branch.get("workdir") != row["workdir"] or branch.get("status") == "removed"):
            fail(f"active inventory lane branch does not match {row['lane']}")
        queue_rows = active_queue_by_lane.get(lane_id, [])
        if len(queue_rows) != 1 or queue_rows[0].get("status") != "queued":
            fail(f"active inventory does not contain exactly one queued run-owned row for {row['lane']}")

    run_lane_ids = {active_lanes[name]["lane_id"] for name in lane_names}
    final_lanes_by_id = {row.get("lane_id"): row for row in final_resources["lanes"]}
    final_branches_by_id = {row.get("lane_id"): row for row in final_resources["lane_branches"]}
    final_queue_by_lane: dict[Any, list[dict[str, Any]]] = {}
    for queue_row in final_resources["merge_queue"]:
        final_queue_by_lane.setdefault(queue_row.get("lane_id"), []).append(queue_row)
    for row in lane_rows:
        lane_id = active_lanes[row["lane"]]["lane_id"]
        retired = final_lanes_by_id.get(lane_id)
        if not retired or retired.get("name") != f"retired/{row['lane']}/{lane_id}":
            fail(f"final inventory lacks the exact retired lane audit row for {row['lane']}")
        branch = final_branches_by_id.get(lane_id)
        if (not branch or branch.get("status") != "removed"
                or not str(branch.get("ref_name", "")).startswith(f"retired/{lane_id}/")
                or branch.get("workdir") != row["workdir"]):
            fail(f"final inventory lacks the exact removed lane branch audit row for {row['lane']}")
        queue_rows = final_queue_by_lane.get(lane_id, [])
        if len(queue_rows) != 1 or queue_rows[0].get("status") != "merged":
            fail(f"final inventory lacks exactly one terminal queue audit row for {row['lane']}")

    def without_run(rows: list[dict[str, Any]], key: str = "lane_id") -> list[dict[str, Any]]:
        return [row for row in rows if row.get(key) not in run_lane_ids]

    for key in ["lanes", "lane_branches", "merge_queue"]:
        if without_run(final_resources[key]) != runtime_resources[key]:
            fail(f"final resource inventory changed pre-runtime {key} rows")
    if [row for row in final_resources["initializations"] if row.get("lane_id") in run_lane_ids]:
        fail("final resource inventory retains run-owned initializations")
    if [row for row in final_resources["workspace_views"] if row.get("lane_id") in run_lane_ids]:
        fail("final resource inventory retains run-owned materializations")
    if [row for row in final_resources["leases"] if row.get("lane_id") in run_lane_ids]:
        fail("final resource inventory retains run-owned leases")
    if any(row.get("name") in {f"refs/lanes/{lane}" for lane in lane_names}
           for row in final_resources["lane_refs"]):
        fail("final resource inventory retains run-owned lane refs")
    stable_keys = ["lane_refs", "initializations", "workspace_views", "leases",
                   "observer_owners", "lock_paths", "socket_paths", "mount_paths",
                   "workdir_paths", "materialization_journals"]
    differences = [key for key in stable_keys if final_resources[key] != runtime_resources[key]]
    if differences:
        fail(f"final resource inventory contains transient/leaked resource differences: {differences}")

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

    baseline_tree, baseline_entries = read_path_state(root / "baseline-path-state.json")
    final_tree, final_entries = read_path_state(root / "final-path-state.json")
    change_base, change_final, path_changes = read_path_changes(root / "path-changes.json")
    if (change_base, change_final) != (baseline_tree, final_tree):
        fail("path change evidence is not bound to the exact baseline/final trees")
    baseline_by_path = {row["path"]: row for row in baseline_entries}
    final_by_path = {row["path"]: row for row in final_entries}
    deleted_paths = sorted(baseline_by_path.keys() - final_by_path.keys())
    modified_baseline = sorted(path for path in baseline_by_path.keys() & final_by_path.keys()
                               if baseline_by_path[path] != final_by_path[path])
    change_deleted = sorted(row["path"] for row in path_changes if row["status"] == "D")
    if deleted_paths != change_deleted:
        fail("false deletion evidence disagrees with exact path state")
    added_changes = sorted(row["path"] for row in path_changes if row["status"] == "A")
    unexpected_change_rows = [row for row in path_changes
                              if row["status"] != "A" or row["path"] not in set(expected_paths)]

    correctness = metrics["correctness"]
    if not isinstance(correctness, dict): fail("correctness must be an object")
    exact_keys(correctness, ["lane_count", "edit_count", "ambiguous_results", "false_deletions", "missing_lanes", "unintended_paths", "integrity_errors", "live_locks"], "correctness")
    if integer(correctness["lane_count"], "correctness.lane_count") != lanes_expected: fail("correctness lane count mismatch")
    if integer(correctness["edit_count"], "correctness.edit_count") != expected_count: fail("correctness edit count mismatch")
    derived_correctness = {
        "ambiguous_results": sum(row["initialization_id"] != row["retry_initialization_id"] for row in lane_rows),
        "false_deletions": len(deleted_paths),
        "missing_lanes": len(set(lane_names) - set(active_lanes)),
        "unintended_paths": (len(set(final_trail_paths) ^ set(expected_paths))
                             + len(set(final_git_paths) ^ set(expected_paths))
                             + len(unexpected_change_rows) + len(modified_baseline)
                             + len(set(added_changes) ^ set(expected_paths))),
        "integrity_errors": 0,
        "live_locks": resource_added_count(runtime_resources, final_resources, "lock_paths") + resource_added_count(runtime_resources, final_resources, "leases"),
    }
    for key, expected in derived_correctness.items():
        if integer(correctness[key], f"correctness.{key}") != expected: fail(f"correctness.{key} does not match raw evidence")
        if expected != 0: fail(f"correctness.{key} must be zero")

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
        if parse_int(row["leaked_resource_count"], f"{scenario} leaked_resource_count") != 0: fail(f"{scenario}: leaked resources remain")
        if row["evidence_command_id"] not in command_ids: fail(f"{scenario}: evidence command is missing")
        if row["source_commit"] != environment["source"]["commit"]: fail(f"{scenario}: fault evidence source commit differs from candidate source")
        if row["binary_sha256"] != environment["binary"]["sha256"]: fail(f"{scenario}: fault evidence binary differs from candidate binary")
        if scenario == "dirty_git_export_refusal":
            if row["evidence_kind"] != "harness_control" or row["binary_exercised"] != "true" or row["integrity_result"] != "harness_control_exit_0":
                fail(f"{scenario}: binary-backed harness control linkage is malformed")
            if row["test_target"] or row["test_name"] or parse_int(row["test_count"], f"{scenario} test_count") != 0:
                fail(f"{scenario}: harness control must not claim a Cargo test")
        else:
            expected_kind = ("focused_test_aggregate"
                             if environment["fault_driver"]["qualification_kind"] == "candidate_harness"
                             else "externally_attested_focused_test")
            if row["evidence_kind"] != expected_kind or row["binary_exercised"] != "false" or row["integrity_result"] != "focused_test_exit_0":
                fail(f"{scenario}: focused-test evidence linkage is malformed")
            if row["initialization_id"] or row["retry_initialization_id"]:
                fail(f"{scenario}: focused test did not establish per-scenario initialization identity")
            if scenario == "filesystem_replacement":
                platform_description = environment["platform"]["description"]
                if "Darwin" in platform_description or "macOS" in platform_description:
                    expected_test = ("changed_path_ledger_macos", "every_root_revalidation_failure_revokes_globally")
                elif "Linux" in platform_description:
                    expected_test = ("changed_path_ledger_linux", "owner_death_and_root_replacement_cannot_prove_clean")
                else:
                    fail("filesystem replacement fault evidence has an unsupported platform")
            else:
                expected_test = FAULT_TESTS[scenario]
            if (row["test_target"], row["test_name"]) != expected_test:
                fail(f"{scenario}: fault evidence does not name the exact focused test")
            if parse_int(row["test_count"], f"{scenario} test_count") != 1:
                fail(f"{scenario}: focused fault probe must execute exactly one test")
            if environment["fault_driver"]["qualification_kind"] == "candidate_harness":
                try:
                    cargo_output = (root / f"commands/{row['evidence_command_id']}.stderr").read_text(
                        encoding="utf-8", errors="replace")
                except OSError as error:
                    fail(f"{scenario}: cannot read raw Cargo output: {error}")
                summaries = re.findall(
                    r"test result: ok\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;",
                    cargo_output,
                )
                named = len(re.findall(
                    r"^test " + re.escape(row["test_name"]) + r" \.\.\. ok$",
                    cargo_output,
                    re.MULTILINE,
                ))
                if summaries != [("1", "0", "0")] or named != 1:
                    fail(f"{scenario}: raw Cargo output does not prove exactly one named test")
        command_wrapper = read_json(root / f"commands/{row['evidence_command_id']}.json")
        if command_wrapper.get("actual_exit_code") != 0:
            fail(f"{scenario}: raw evidence command did not exit zero")
        command_payload = command_wrapper.get("payload")
        if (not isinstance(command_payload, dict)
                or command_payload.get("scenario") != scenario
                or command_payload.get("test_target", "") != row["test_target"]
                or command_payload.get("test_name", "") != row["test_name"]
                or command_payload.get("test_count", 0) != int(row["test_count"])):
            fail(f"{scenario}: raw fault evidence does not match the exact test identity/count")
        if scenario in INITIALIZATION_PHASES:
            if row["durable_phase"] != scenario.removeprefix("after_"): fail(f"{scenario}: durable phase is wrong")
            if row["retry_result"] not in {"resumed_same_initialization", "repaired_once"}: fail(f"{scenario}: retry result is not a committed repair/resume")
        elif row["retry_result"] not in {"resumed_same_initialization", "repaired_once", "refused_without_mutation", "recovered_once"}:
            fail(f"{scenario}: unsupported retry result")

    cleanup = metrics["cleanup"]
    if not isinstance(cleanup, dict): fail("cleanup must be an object")
    exact_keys(cleanup, ["stale_mounts", "stale_sockets", "stale_locks", "stale_initializations", "stale_materializations", "leaked_workdirs", "stale_queue_rows", "stale_lane_rows", "stale_lane_refs"], "cleanup")
    derived_cleanup = {
        "stale_mounts": resource_added_count(runtime_resources, final_resources, "mount_paths"),
        "stale_sockets": resource_added_count(runtime_resources, final_resources, "socket_paths"),
        "stale_locks": resource_added_count(runtime_resources, final_resources, "lock_paths") + resource_added_count(runtime_resources, final_resources, "leases"),
        "stale_initializations": len([row for row in final_resources["initializations"] if row.get("lane_id") in run_lane_ids]),
        "stale_materializations": (len([row for row in final_resources["workspace_views"] if row.get("lane_id") in run_lane_ids])
                                   + resource_added_count(runtime_resources, final_resources, "materialization_journals")),
        "leaked_workdirs": resource_added_count(runtime_resources, final_resources, "workdir_paths"),
        "stale_queue_rows": len([row for row in final_resources["merge_queue"]
                                  if row.get("lane_id") in run_lane_ids and row.get("status") not in {"merged", "failed", "cancelled"}]),
        "stale_lane_rows": len([row for row in final_resources["lanes"]
                                 if row.get("lane_id") in run_lane_ids and not str(row.get("name", "")).startswith("retired/")]),
        "stale_lane_refs": len([row for row in final_resources["lane_refs"]
                                 if row.get("name") in {f"refs/lanes/{lane}" for lane in lane_names}]),
    }
    for key, expected in derived_cleanup.items():
        if integer(cleanup[key], f"cleanup.{key}") != expected: fail(f"cleanup.{key} does not match resource inventory")
        if expected != 0: fail("cleanup evidence reports leaked resources")
    audit_history = metrics["audit_history"]
    if not isinstance(audit_history, dict):
        fail("audit_history must be an object")
    exact_keys(audit_history, ["retired_lane_rows", "removed_lane_branch_rows", "terminal_queue_rows"], "audit_history")
    derived_audit = {
        "retired_lane_rows": len([row for row in final_resources["lanes"] if row.get("lane_id") in run_lane_ids]),
        "removed_lane_branch_rows": len([row for row in final_resources["lane_branches"] if row.get("lane_id") in run_lane_ids]),
        "terminal_queue_rows": len([row for row in final_resources["merge_queue"] if row.get("lane_id") in run_lane_ids]),
    }
    for key, expected in derived_audit.items():
        if integer(audit_history[key], f"audit_history.{key}") != expected or expected != lanes_expected:
            fail(f"audit_history.{key} does not match retained run audit history")
    integrity = metrics["integrity"]
    if not isinstance(integrity, dict): fail("integrity must be an object")
    exact_keys(integrity, ["trail_doctor", "trail_fsck", "git_fsck", "conflict_control"], "integrity")
    integrity_commands = {"trail_doctor": "trail-doctor", "trail_fsck": "trail-fsck", "git_fsck": "git-fsck"}
    for key, command_id in integrity_commands.items():
        wrapper = read_json(root / f"commands/{command_id}.json")
        measured = wrapper.get("actual_exit_code") == 0 and command_id in command_ids
        payload = wrapper.get("payload")
        if key == "trail_doctor": measured = measured and isinstance(payload, dict) and payload.get("status") == "ok"
        if key == "trail_fsck": measured = measured and isinstance(payload, dict) and payload.get("errors") == []
        if boolean(integrity[key], f"integrity.{key}") != measured or not measured:
            fail("doctor/fsck integrity command failed")
    conflict_measured = any(row["scenario"] == "conflicting_lanes" and row["actual_code"] == row["expected_code"] for row in fault_rows)
    if boolean(integrity["conflict_control"], "integrity.conflict_control") != conflict_measured or not conflict_measured:
        fail("conflict integrity control failed")

    git_state = metrics["git_state_preservation"]
    if not isinstance(git_state, dict): fail("git_state_preservation must be an object")
    exact_keys(git_state, [
        "tracked_worktree_clean", "index_clean", "preexisting_untracked_count",
        "final_untracked_count", "preserved_untracked_count", "added_untracked_count",
        "removed_untracked_count", "modified_untracked_count",
    ], "git_state_preservation")
    for key in ["tracked_worktree_clean", "index_clean"]:
        if not boolean(git_state[key], f"git_state_preservation.{key}"):
            fail(f"git_state_preservation.{key} must be true")
    expected_git_state_counts = {
        "preexisting_untracked_count": len(baseline_untracked),
        "final_untracked_count": len(final_untracked),
        "preserved_untracked_count": preserved_untracked,
        "added_untracked_count": len(added_untracked),
        "removed_untracked_count": len(removed_untracked),
        "modified_untracked_count": len(modified_untracked),
    }
    for key, expected in expected_git_state_counts.items():
        if integer(git_state[key], f"git_state_preservation.{key}") != expected:
            fail(f"git_state_preservation.{key} does not match untracked snapshots")

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

#!/usr/bin/env python3
"""Gate changed-path-ledger scale artifacts without mixing oracle work into fast paths."""

import argparse
import csv
import json
import math
import pathlib
import sys


DEFAULT_OPERATIONS = (
    "workspace_status",
    "workspace_diff",
    "workspace_record",
    "materialized_lane_record",
    "structured_patch",
)
ORACLE_OPERATIONS = (
    "workspace_status",
    "workspace_diff",
    "workspace_record",
    "materialized_lane_record",
    "structured_patch",
    "cow_checkpoint",
)
SCOPED_OPERATIONS = {
    "workspace_status": "status",
    "workspace_diff": "diff",
    "workspace_record": "record",
    "materialized_lane_record": "materialized_lane_record",
    "structured_patch": "structured_patch",
    "cow_checkpoint": "cow_checkpoint",
}
ZERO_SCOPE_FIELDS = (
    "full_filesystem_walk_count",
    "bounded_filesystem_walk_count",
    "full_root_range_count",
    "selected_worktree_index_sqlite_full_scan_count",
    "policy_dependency_full_discovery",
    "reconciliation_run_count",
    "manifest_bytes",
    "upper_work_count",
    "external_adapter_global_work",
    "git_global_work_count",
    "git_index_refresh_count",
    "git_trace2_region_count",
    "git_trace2_bytes",
    "git_fsmonitor_qualification_count",
    "git_untracked_cache_qualification_count",
    "git_index_read_count",
    "git_index_bytes",
    "git_shared_index_read_count",
    "git_shared_index_bytes",
    "daemon_cumulative_rewrite_count",
    "daemon_cumulative_rewrite_bytes",
)
OPERATION_METRIC_FIELDS = (
    "generation",
    "operation",
    "outcome",
    "input_path_count",
    "canonical_path_count",
    "expanded_path_count",
    "final_path_count",
    "full_filesystem_walk_count",
    "bounded_filesystem_walk_count",
    "filesystem_entry_count",
    "filesystem_stat_count",
    "filesystem_read_count",
    "filesystem_read_bytes",
    "filesystem_hash_count",
    "filesystem_hash_bytes",
    "full_root_range_count",
    "bounded_root_range_count",
    "root_range_row_count",
    "root_point_key_count",
    "prolly_read_call_count",
    "prolly_read_key_count",
    "prolly_read_value_count",
    "prolly_read_value_bytes",
    "prolly_write_call_count",
    "prolly_write_key_count",
    "prolly_write_value_bytes",
    "prolly_tree_batch_call_count",
    "prolly_tree_batch_mutation_count",
    "selected_worktree_index_sqlite_accounting_complete",
    "selected_worktree_index_sqlite_accounting_disposition",
    "selected_worktree_index_sqlite_envelope_count",
    "selected_worktree_index_sqlite_not_applicable_count",
    "selected_worktree_index_sqlite_full_scan_count",
    "selected_worktree_index_sqlite_row_read_count",
    "selected_worktree_index_sqlite_row_delete_count",
    "selected_worktree_index_sqlite_row_upsert_count",
    "selected_worktree_index_sqlite_statement_count",
    "selected_worktree_index_sqlite_transaction_count",
    "selection_comparison_count",
    "policy_build_count",
    "policy_dependency_full_discovery",
    "policy_dependency_bytes",
    "policy_dependency_file_count",
    "git_subprocess_count",
    "git_global_work_count",
    "git_index_refresh_count",
    "git_trace2_region_count",
    "git_trace2_bytes",
    "git_fsmonitor_qualification_count",
    "git_untracked_cache_qualification_count",
    "external_adapter_global_work",
    "git_index_read_count",
    "git_index_bytes",
    "git_shared_index_read_count",
    "git_shared_index_bytes",
    "git_output_bytes",
    "git_output_record_count",
    "daemon_snapshot_bytes",
    "daemon_snapshot_path_count",
    "daemon_cumulative_rewrite_count",
    "daemon_cumulative_rewrite_bytes",
    "daemon_cumulative_rewrite_count_total",
    "daemon_cumulative_rewrite_bytes_total",
    "authoritative_candidate_count",
    "ledger_row_touch_count",
    "observer_tail_record_fold_count",
    "reconciliation_run_count",
    "manifest_bytes",
    "manifest_key_comparison_count",
    "journal_bytes",
    "upper_work_count",
    "wall_time_ns",
    "rss_start_bytes",
    "rss_end_bytes",
    "rss_lifetime_high_water_bytes",
)
REQUIRED_SCOPE_FIELDS = (*OPERATION_METRIC_FIELDS, "configured_tail_bound")

# Each nonzero work allowance is affine in the exact candidate count. The
# operation multiplier accounts for the different constant number of stages in
# each command while preserving a repository-size-independent O(k) ceiling.
OPERATION_WORK_MULTIPLIER = {
    "workspace_status": 1,
    "workspace_diff": 2,
    "workspace_record": 3,
    "materialized_lane_record": 6,
    "structured_patch": 6,
    "cow_checkpoint": 8,
}
AFFINE_WORK_BOUNDS = {
    "input_path_count": (8, 2),
    "canonical_path_count": (8, 2),
    "filesystem_entry_count": (16, 16),
    "filesystem_stat_count": (8, 8),
    "filesystem_read_count": (8, 8),
    "filesystem_read_bytes": (1 << 20, 1 << 20),
    "filesystem_hash_count": (8, 8),
    "filesystem_hash_bytes": (1 << 20, 1 << 20),
    "bounded_root_range_count": (8, 8),
    "root_range_row_count": (16, 16),
    "root_point_key_count": (8, 8),
    "prolly_read_call_count": (32, 32),
    "prolly_read_key_count": (64, 64),
    "prolly_read_value_count": (64, 64),
    "prolly_read_value_bytes": (1 << 24, 1 << 24),
    "prolly_write_call_count": (32, 32),
    "prolly_write_key_count": (64, 64),
    "prolly_write_value_bytes": (1 << 24, 1 << 24),
    "prolly_tree_batch_call_count": (16, 16),
    "prolly_tree_batch_mutation_count": (64, 64),
    "selection_comparison_count": (64, 64),
    "policy_build_count": (8, 8),
    "policy_dependency_bytes": (1 << 20, 1 << 20),
    "policy_dependency_file_count": (16, 16),
    "git_subprocess_count": (16, 4),
    "git_output_bytes": (1 << 12, 1 << 12),
    "git_output_record_count": (16, 16),
    "daemon_snapshot_bytes": (1 << 20, 1 << 20),
    "daemon_snapshot_path_count": (8, 8),
    "ledger_row_touch_count": (16, 16),
    "manifest_key_comparison_count": (64, 64),
    "journal_bytes": (1 << 24, 1 << 24),
}

SIDE_CAR_BASE_FIELDS = {
    "benchmark",
    "benchmark_operation",
    "repo_files",
    "authoritative_input_k",
    "final_changed_output",
    "configured_tail_bound",
    "metric_source",
    "scope_caps",
}
UPPER_RECOVERY_FIELDS = {
    "upper_recovery_walks",
}
GENERATED_PATH_ACCOUNTING_FIELDS = {
    "generated_path_accounting",
}
PATH_INDEX_FIELDS = {
    "path_index_full_root_path_load_count",
    "path_index_full_filesystem_path_scan_count",
    "path_index_lookup_count",
    "path_index_mode",
}


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("results", type=pathlib.Path)
    parser.add_argument("structural_metrics", type=pathlib.Path)
    parser.add_argument("--oracle", required=True, type=pathlib.Path)
    parser.add_argument("--k", default="0,1,100")
    parser.add_argument("--operations", default=",".join(DEFAULT_OPERATIONS))
    parser.add_argument("--require-cow", action="store_true")
    parser.add_argument("--require-cold-reconcile", action="store_true")
    parser.add_argument("--max-seconds", action="append", default=[])
    parser.add_argument("--max-rss-bytes", action="append", default=[])
    return parser.parse_args(argv)


def parse_integer_list(value: str, label: str) -> tuple[int, ...]:
    try:
        values = tuple(int(item) for item in value.split(",") if item)
    except ValueError as error:
        raise ValueError(f"invalid {label}: {value}") from error
    if not values or any(item < 0 for item in values):
        raise ValueError(f"invalid {label}: {value}")
    return values


def parse_thresholds(specs: list[str], label: str) -> dict[str, float]:
    parsed = {}
    for spec in specs:
        if "=" not in spec:
            raise ValueError(f"invalid {label} threshold {spec!r}; expected operation=value")
        operation, raw = spec.split("=", 1)
        if not operation:
            raise ValueError(f"invalid {label} threshold {spec!r}; operation is empty")
        try:
            value = float(raw)
        except ValueError as error:
            raise ValueError(f"invalid {label} threshold value in {spec!r}") from error
        if value < 0:
            raise ValueError(f"invalid {label} threshold value in {spec!r}")
        parsed[operation] = value
    return parsed


def read_results(path: pathlib.Path) -> dict[str, dict[str, str]]:
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        fieldnames = reader.fieldnames or []
        rows = list(reader)
    required = {"name", "real_seconds", "max_rss_bytes", "exit_code"}
    if not required.issubset(fieldnames):
        raise ValueError(f"{path}: missing required results columns")
    found = {}
    for row in rows:
        name = row.get("name", "")
        if not name:
            continue
        if name in found:
            raise ValueError(f"{path}: duplicate result {name!r}")
        found[name] = row
    return found


def read_jsonl(path: pathlib.Path) -> dict[str, dict[str, object]]:
    found = {}
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"{path}:{number}: invalid JSON: {error}") from error
        if not isinstance(record, dict):
            raise ValueError(f"{path}:{number}: expected a JSON object")
        benchmark = record.get("benchmark")
        if not isinstance(benchmark, str) or not benchmark:
            raise ValueError(f"{path}:{number}: missing benchmark")
        if benchmark in found:
            raise ValueError(f"{path}:{number}: duplicate benchmark {benchmark!r}")
        found[benchmark] = record
    return found


def read_oracle(path: pathlib.Path) -> dict[str, dict[str, str]]:
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        fieldnames = reader.fieldnames or []
        rows = list(reader)
    required = {
        "benchmark",
        "repo_files",
        "authoritative_input_k",
        "oracle_time_ns",
        "measured_paths",
        "oracle_paths",
        "equal",
    }
    if not required.issubset(fieldnames):
        raise ValueError(f"{path}: missing required oracle columns")
    found = {}
    for row in rows:
        benchmark = row.get("benchmark", "")
        if benchmark in found:
            raise ValueError(f"{path}: duplicate oracle benchmark {benchmark!r}")
        found[benchmark] = row
    return found


def require_number(record: dict[str, object], key: str, failures: list[str], name: str) -> float:
    value = record.get(key)
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
    ):
        failures.append(f"{name}: missing or nonnumeric structural field {key}")
        return 0
    return float(value)


def check_scope_caps(record: dict[str, object], failures: list[str], name: str) -> None:
    scopes = record.get("scope_caps")
    if not isinstance(scopes, list) or not scopes:
        failures.append(f"{name}: no persisted changed-path scope caps were reported")
        return
    comparisons = (
        ("candidate_rows", "candidate_row_cap"),
        ("prefix_rows", "prefix_row_cap"),
        ("observer_log_bytes", "observer_log_byte_cap"),
        ("largest_segment_bytes", "segment_byte_cap"),
    )
    expected_fields = {"scope_id"}
    for value_key, cap_key in comparisons:
        expected_fields.update((value_key, cap_key))
    for scope in scopes:
        if not isinstance(scope, dict):
            failures.append(f"{name}: malformed scope cap record")
            continue
        missing = sorted(expected_fields.difference(scope))
        unknown = sorted(set(scope).difference(expected_fields))
        if missing:
            failures.append(f"{name}: scope cap omitted {', '.join(missing)}")
        if unknown:
            failures.append(f"{name}: scope cap has unknown fields {', '.join(unknown)}")
        scope_id = scope.get("scope_id", "unknown")
        for value_key, cap_key in comparisons:
            value = scope.get(value_key)
            cap = scope.get(cap_key)
            if not isinstance(value, (int, float)) or not isinstance(cap, (int, float)):
                failures.append(f"{name}: scope {scope_id} omitted {value_key}/{cap_key}")
            elif value > cap:
                failures.append(f"{name}: scope {scope_id} {value_key}={value} exceeds {cap_key}={cap}")


def check_scoped(record: dict[str, object], operation: str, k: int, failures: list[str], name: str) -> None:
    if record.get("metric_source") != "operation_scope":
        failures.append(f"{name}: expected operation_scope metrics")
        return
    missing = [key for key in REQUIRED_SCOPE_FIELDS if key not in record]
    if missing:
        failures.append(f"{name}: missing structural fields {', '.join(missing)}")
        return
    allowed_fields = set(OPERATION_METRIC_FIELDS) | SIDE_CAR_BASE_FIELDS
    if operation in {"materialized_lane_record", "cow_checkpoint"}:
        allowed_fields.update(UPPER_RECOVERY_FIELDS)
    if operation == "cow_checkpoint":
        allowed_fields.update(GENERATED_PATH_ACCOUNTING_FIELDS)
    if operation in {"materialized_lane_record", "structured_patch"}:
        allowed_fields.update(PATH_INDEX_FIELDS)
    unknown = sorted(set(record).difference(allowed_fields))
    if unknown:
        failures.append(f"{name}: unknown structural fields {', '.join(unknown)}")
    if record.get("operation") != SCOPED_OPERATIONS[operation]:
        failures.append(f"{name}: operation report is {record.get('operation')!r}")
    if str(record.get("outcome")).lower() != "success":
        failures.append(f"{name}: operation outcome is {record.get('outcome')!r}")
    nonnumeric_fields = {
        "operation",
        "outcome",
        "selected_worktree_index_sqlite_accounting_complete",
        "selected_worktree_index_sqlite_accounting_disposition",
    }
    for field in OPERATION_METRIC_FIELDS:
        if field not in nonnumeric_fields:
            require_number(record, field, failures, name)
    sqlite_activity_fields = (
        "selected_worktree_index_sqlite_envelope_count",
        "selected_worktree_index_sqlite_not_applicable_count",
        "selected_worktree_index_sqlite_full_scan_count",
        "selected_worktree_index_sqlite_row_read_count",
        "selected_worktree_index_sqlite_row_delete_count",
        "selected_worktree_index_sqlite_row_upsert_count",
        "selected_worktree_index_sqlite_statement_count",
        "selected_worktree_index_sqlite_transaction_count",
    )
    sqlite_activity = {
        field: require_number(record, field, failures, name)
        for field in sqlite_activity_fields
    }
    sqlite_complete = record.get("selected_worktree_index_sqlite_accounting_complete")
    sqlite_disposition = record.get("selected_worktree_index_sqlite_accounting_disposition")
    workspace_operation = operation in {
        "workspace_status",
        "workspace_diff",
        "workspace_record",
    }
    if workspace_operation:
        if sqlite_disposition != "not_applicable" or sqlite_complete is not False:
            failures.append(
                f"{name}: workspace selected worktree-index SQLite disposition must be independently proven not_applicable"
            )
        if sqlite_activity["selected_worktree_index_sqlite_not_applicable_count"] != 1:
            failures.append(
                f"{name}: workspace selected worktree-index SQLite N/A proof count must equal one"
            )
        unexpected = {
            field: value
            for field, value in sqlite_activity.items()
            if field != "selected_worktree_index_sqlite_not_applicable_count" and value != 0
        }
        if unexpected:
            failures.append(
                f"{name}: selected worktree-index SQLite work occurred on a not_applicable operation"
            )
    else:
        if sqlite_disposition != "complete" or sqlite_complete is not True:
            failures.append(
                f"{name}: selected worktree-index SQLite accounting must be complete"
            )
        envelopes = sqlite_activity["selected_worktree_index_sqlite_envelope_count"]
        if envelopes <= 0:
            failures.append(
                f"{name}: selected worktree-index SQLite completeness requires an accounting envelope"
            )
        if sqlite_activity["selected_worktree_index_sqlite_not_applicable_count"] != 0:
            failures.append(
                f"{name}: selected worktree-index SQLite accounting mixes complete and not_applicable claims"
            )
        sqlite_bounds = {
            "selected_worktree_index_sqlite_envelope_count": k + 1,
            "selected_worktree_index_sqlite_row_read_count": 8 * (k + 1),
            "selected_worktree_index_sqlite_row_delete_count": 8 * (k + 1),
            "selected_worktree_index_sqlite_row_upsert_count": 8 * (k + 1),
            "selected_worktree_index_sqlite_statement_count": 32 * (k + 1),
            "selected_worktree_index_sqlite_transaction_count": k + 1,
        }
        for field, bound in sqlite_bounds.items():
            if sqlite_activity[field] > bound:
                failures.append(
                    f"{name}: {field}={sqlite_activity[field]:g} exceeds O(k) bound {bound:g}"
                )
    for field in ZERO_SCOPE_FIELDS:
        if require_number(record, field, failures, name) != 0:
            failures.append(f"{name}: {field} must remain zero on a warm trusted run")
    authoritative = require_number(record, "authoritative_candidate_count", failures, name)
    expanded = require_number(record, "expanded_path_count", failures, name)
    if authoritative != k:
        failures.append(
            f"{name}: authoritative candidates {authoritative:g} do not equal exact-file k={k}"
        )
    if expanded > k:
        failures.append(
            f"{name}: expanded paths {expanded:g} exceed exact-file k={k}"
        )
    operation_multiplier = OPERATION_WORK_MULTIPLIER[operation]
    for field, (base, per_candidate) in AFFINE_WORK_BOUNDS.items():
        value = require_number(record, field, failures, name)
        bound = base + per_candidate * k * operation_multiplier
        if value > bound:
            failures.append(
                f"{name}: {field}={value:g} exceeds O(k) bound {bound:g}"
            )
    folded = require_number(record, "observer_tail_record_fold_count", failures, name)
    tail_bound = require_number(record, "configured_tail_bound", failures, name)
    if tail_bound <= 0 or tail_bound > 4096:
        failures.append(f"{name}: configured_tail_bound={tail_bound:g} exceeds audited cap 4096")
    if folded > tail_bound:
        failures.append(f"{name}: observer tail folded {folded:g} records, above {tail_bound:g}")

    total_count = require_number(
        record, "daemon_cumulative_rewrite_count_total", failures, name
    )
    total_bytes = require_number(
        record, "daemon_cumulative_rewrite_bytes_total", failures, name
    )
    if total_count < 0 or total_bytes < 0:
        failures.append(f"{name}: daemon cumulative gauges must be nonnegative")


def check_operation_extras(
    record: dict[str, object], operation: str, k: int, failures: list[str], name: str
) -> None:
    required_zero = []
    if operation in {"materialized_lane_record", "cow_checkpoint"}:
        required_zero.append("upper_recovery_walks")
    if operation in {"materialized_lane_record", "structured_patch"}:
        required_zero.extend(
            (
                "path_index_full_root_path_load_count",
                "path_index_full_filesystem_path_scan_count",
            )
        )
    for field in required_zero:
        if require_number(record, field, failures, name) != 0:
            failures.append(f"{name}: {field} must remain zero on a warm trusted run")
    if operation == "cow_checkpoint" and record.get("generated_path_accounting") != "journal_interval":
        failures.append(
            f"{name}: generated_path_accounting={record.get('generated_path_accounting')!r}, expected 'journal_interval'"
        )
    if operation in {"materialized_lane_record", "structured_patch"}:
        lookups = require_number(record, "path_index_lookup_count", failures, name)
        lookup_bound = 8 * (k + 1)
        if lookups > lookup_bound:
            failures.append(
                f"{name}: path_index_lookup_count={lookups:g} exceeds O(k) bound {lookup_bound:g}"
            )
        mode = record.get("path_index_mode")
        if mode != "indexed" and not (k == 0 and mode == "unknown"):
            failures.append(f"{name}: path_index_mode={mode!r}, expected 'indexed'")


def check_artifacts(args: argparse.Namespace) -> list[str]:
    k_values = parse_integer_list(args.k, "candidate counts")
    operations = tuple(item for item in args.operations.split(",") if item)
    if args.require_cow and "cow_checkpoint" not in operations:
        operations += ("cow_checkpoint",)
    unknown = sorted(set(operations).difference((*DEFAULT_OPERATIONS, "cow_checkpoint")))
    if unknown:
        raise ValueError(f"unknown operations: {', '.join(unknown)}")
    time_limits = parse_thresholds(args.max_seconds, "time")
    rss_limits = parse_thresholds(args.max_rss_bytes, "RSS")
    results = read_results(args.results)
    structural = read_jsonl(args.structural_metrics)
    oracle = read_oracle(args.oracle)
    failures = []

    if args.require_cold_reconcile:
        name = "ledger_cold_reconcile"
        result = results.get(name)
        if result is None:
            failures.append(f"{name}: missing benchmark result")
        else:
            try:
                if int(result["exit_code"]) != 0:
                    failures.append(f"{name}: exit_code={result['exit_code']}")
                seconds = float(result["real_seconds"])
                rss = float(result["max_rss_bytes"])
                if not math.isfinite(seconds) or seconds < 0:
                    failures.append(f"{name}: invalid real_seconds={result['real_seconds']!r}")
                if not math.isfinite(rss) or rss <= 0:
                    failures.append(f"{name}: invalid max_rss_bytes={result['max_rss_bytes']!r}")
                if "cold_reconcile" in time_limits and seconds > time_limits["cold_reconcile"]:
                    failures.append(
                        f"{name}: {seconds:.2f}s > {time_limits['cold_reconcile']:.2f}s"
                    )
                if "cold_reconcile" in rss_limits and rss > rss_limits["cold_reconcile"]:
                    failures.append(
                        f"{name}: RSS {rss:.0f} > {rss_limits['cold_reconcile']:.0f}"
                    )
            except (KeyError, ValueError) as error:
                failures.append(f"{name}: malformed benchmark result: {error}")

    for operation in operations:
        for k in k_values:
            name = f"ledger_{operation}_k{k}"
            result = results.get(name)
            if result is None:
                failures.append(f"{name}: missing benchmark result")
            else:
                try:
                    if int(result["exit_code"]) != 0:
                        failures.append(f"{name}: exit_code={result['exit_code']}")
                    seconds = float(result["real_seconds"])
                    rss = float(result["max_rss_bytes"])
                    if not math.isfinite(seconds) or seconds < 0:
                        failures.append(
                            f"{name}: invalid real_seconds={result['real_seconds']!r}"
                        )
                    if not math.isfinite(rss) or rss <= 0:
                        failures.append(
                            f"{name}: invalid max_rss_bytes={result['max_rss_bytes']!r}"
                        )
                    if operation in time_limits and seconds > time_limits[operation]:
                        failures.append(f"{name}: {seconds:.2f}s > {time_limits[operation]:.2f}s")
                    if operation in rss_limits and rss > rss_limits[operation]:
                        failures.append(f"{name}: RSS {rss:.0f} > {rss_limits[operation]:.0f}")
                except (KeyError, ValueError) as error:
                    failures.append(f"{name}: malformed benchmark result: {error}")

            record = structural.get(name)
            if record is None:
                failures.append(f"{name}: missing structural metrics report")
                continue
            if record.get("benchmark_operation") != operation:
                failures.append(f"{name}: benchmark_operation={record.get('benchmark_operation')!r}")
            if record.get("authoritative_input_k") != k:
                failures.append(f"{name}: authoritative_input_k={record.get('authoritative_input_k')!r}")
            expected_output = k
            if record.get("final_changed_output") != expected_output:
                failures.append(
                    f"{name}: final_changed_output={record.get('final_changed_output')!r}, expected {expected_output}"
                )
            check_scoped(record, operation, k, failures, name)
            check_operation_extras(record, operation, k, failures, name)
            reported_rss = require_number(
                record, "rss_lifetime_high_water_bytes", failures, name
            )
            if reported_rss <= 0:
                failures.append(f"{name}: operation RSS must be positive")
            if operation in rss_limits and reported_rss > rss_limits[operation]:
                failures.append(
                    f"{name}: operation RSS {reported_rss:.0f} > {rss_limits[operation]:.0f}"
                )
            reported_wall_ns = require_number(record, "wall_time_ns", failures, name)
            if reported_wall_ns <= 0:
                failures.append(f"{name}: operation wall_time_ns must be positive")
            reported_seconds = reported_wall_ns / 1_000_000_000
            if operation in time_limits and reported_seconds > time_limits[operation]:
                failures.append(
                    f"{name}: operation wall time {reported_seconds:.2f}s > {time_limits[operation]:.2f}s"
                )
            if record.get("final_path_count") != record.get("final_changed_output"):
                failures.append(
                    f"{name}: final_path_count does not match final_changed_output"
                )
            check_scope_caps(record, failures, name)

            if operation in ORACLE_OPERATIONS:
                oracle_row = oracle.get(name)
                if oracle_row is None:
                    failures.append(f"{name}: missing separate full-scan oracle result")
                else:
                    try:
                        if int(oracle_row["equal"]) != 1:
                            failures.append(f"{name}: oracle equality failed")
                        measured = int(oracle_row["measured_paths"])
                        observed = int(oracle_row["oracle_paths"])
                        if int(oracle_row["authoritative_input_k"]) != k:
                            failures.append(f"{name}: oracle authoritative_input_k mismatch")
                        if int(oracle_row["repo_files"]) != record.get("repo_files"):
                            failures.append(f"{name}: oracle repo_files mismatch")
                        if measured != expected_output or observed != expected_output:
                            failures.append(
                                f"{name}: measured/oracle path counts are {measured}/{observed}, expected {expected_output}"
                            )
                        if int(oracle_row["oracle_time_ns"]) < 0:
                            failures.append(f"{name}: oracle_time_ns is negative")
                    except (KeyError, ValueError) as error:
                        failures.append(f"{name}: malformed oracle result: {error}")
    return failures


def main(argv: list[str] | None = None) -> int:
    try:
        args = parse_args(argv or sys.argv[1:])
        failures = check_artifacts(args)
    except (OSError, ValueError) as error:
        print(f"changed-path ledger threshold input error: {error}", file=sys.stderr)
        return 2
    if failures:
        print("changed-path ledger threshold failures:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print("changed-path ledger thresholds passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

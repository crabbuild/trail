import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("check-changed-path-ledger-thresholds.py")
OPERATIONS = (
    "workspace_status",
    "workspace_diff",
    "workspace_record",
    "materialized_lane_record",
    "structured_patch",
)
SCOPED = {
    "workspace_status": "status",
    "workspace_diff": "diff",
    "workspace_record": "record",
    "materialized_lane_record": "materialized_lane_record",
    "structured_patch": "structured_patch",
    "cow_checkpoint": "cow_checkpoint",
}


def scope_caps():
    return [
        {
            "scope_id": "workspace:test",
            "candidate_rows": 0,
            "candidate_row_cap": 250000,
            "prefix_rows": 0,
            "prefix_row_cap": 16384,
            "observer_log_bytes": 128,
            "observer_log_byte_cap": 268435456,
            "largest_segment_bytes": 128,
            "segment_byte_cap": 16777216,
        }
    ]


def structural_record(operation, k):
    name = f"ledger_{operation}_k{k}"
    final_output = k
    common = {
        "benchmark": name,
        "benchmark_operation": operation,
        "repo_files": 1000,
        "authoritative_input_k": k,
        "final_changed_output": final_output,
        "scope_caps": scope_caps(),
    }
    if operation in SCOPED:
        record = {
            **common,
            "metric_source": "operation_scope",
            "generation": 1,
            "operation": SCOPED[operation],
            "outcome": "success",
            "input_path_count": k,
            "canonical_path_count": k,
            "authoritative_candidate_count": k,
            "final_path_count": k,
            "filesystem_entry_count": 0,
            "filesystem_stat_count": k,
            "filesystem_read_count": k if operation == "workspace_record" else 0,
            "filesystem_read_bytes": 32 * k,
            "filesystem_hash_count": k if operation == "workspace_record" else 0,
            "filesystem_hash_bytes": 32 * k,
            "expanded_path_count": 0,
            "bounded_root_range_count": 0,
            "root_range_row_count": 0,
            "root_point_key_count": k,
            "prolly_read_call_count": k,
            "prolly_read_key_count": k,
            "prolly_read_value_count": k,
            "prolly_read_value_bytes": 32 * k,
            "prolly_write_call_count": k,
            "prolly_write_key_count": k,
            "prolly_write_value_bytes": 32 * k,
            "prolly_tree_batch_call_count": min(k, 1),
            "prolly_tree_batch_mutation_count": k,
            "ledger_row_touch_count": 2 * k,
            "observer_tail_record_fold_count": k,
            "configured_tail_bound": 4096,
            "wall_time_ns": 1000,
            "rss_lifetime_high_water_bytes": 1024,
            "full_filesystem_walk_count": 0,
            "bounded_filesystem_walk_count": 0,
            "full_root_range_count": 0,
            "selected_worktree_index_sqlite_full_scan_count": 0,
            "selected_worktree_index_sqlite_accounting_complete": operation
            not in {"workspace_status", "workspace_diff", "workspace_record"},
            "selected_worktree_index_sqlite_accounting_disposition": (
                "not_applicable"
                if operation in {"workspace_status", "workspace_diff", "workspace_record"}
                else "complete"
            ),
            "selected_worktree_index_sqlite_envelope_count": (
                0
                if operation in {"workspace_status", "workspace_diff", "workspace_record"}
                else 1
            ),
            "selected_worktree_index_sqlite_not_applicable_count": (
                1
                if operation in {"workspace_status", "workspace_diff", "workspace_record"}
                else 0
            ),
            "selected_worktree_index_sqlite_row_read_count": 0,
            "selected_worktree_index_sqlite_row_delete_count": 0,
            "selected_worktree_index_sqlite_row_upsert_count": 0,
            "selected_worktree_index_sqlite_statement_count": 0,
            "selected_worktree_index_sqlite_transaction_count": 0,
            "selection_comparison_count": k,
            "policy_build_count": 1,
            "policy_dependency_full_discovery": 0,
            "policy_dependency_bytes": 0,
            "policy_dependency_file_count": 0,
            "git_subprocess_count": 0,
            "reconciliation_run_count": 0,
            "manifest_bytes": 0,
            "manifest_key_comparison_count": k,
            "journal_bytes": 32 * k,
            "upper_work_count": 0,
            "external_adapter_global_work": 0,
            "git_global_work_count": 0,
            "git_index_refresh_count": 0,
            "git_trace2_region_count": 0,
            "git_trace2_bytes": 0,
            "git_fsmonitor_qualification_count": 0,
            "git_untracked_cache_qualification_count": 0,
            "git_index_read_count": 0,
            "git_index_bytes": 0,
            "git_shared_index_read_count": 0,
            "git_shared_index_bytes": 0,
            "git_output_bytes": 0,
            "git_output_record_count": 0,
            "daemon_snapshot_bytes": 0,
            "daemon_snapshot_path_count": k,
            "daemon_cumulative_rewrite_count": 0,
            "daemon_cumulative_rewrite_bytes": 0,
            "daemon_cumulative_rewrite_count_total": 1,
            "daemon_cumulative_rewrite_bytes_total": 128,
            "rss_start_bytes": 1024,
            "rss_end_bytes": 1024,
        }
        if operation in {"materialized_lane_record", "cow_checkpoint"}:
            record["upper_recovery_walks"] = 0
        if operation == "cow_checkpoint":
            record["generated_path_accounting"] = "journal_interval"
        if operation in {"materialized_lane_record", "structured_patch"}:
            record.update(
                {
                    "path_index_full_root_path_load_count": 0,
                    "path_index_full_filesystem_path_scan_count": 0,
                    "path_index_lookup_count": k,
                    "path_index_mode": "unknown" if k == 0 else "indexed",
                }
            )
        return record
    raise AssertionError(f"missing scoped fixture for {operation}")


class ChangedPathLedgerThresholdTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = pathlib.Path(self.directory.name)
        self.results = self.root / "results.tsv"
        self.structural = self.root / "structural-metrics.jsonl"
        self.oracle = self.root / "oracle-results.tsv"
        self.records = [
            structural_record(operation, k)
            for operation in OPERATIONS
            for k in (0, 1, 100)
        ]
        self.write_fixture()

    def write_fixture(self):
        result_lines = ["name\treal_seconds\tmax_rss_bytes\texit_code"]
        oracle_lines = [
            "benchmark\trepo_files\tauthoritative_input_k\toracle_time_ns\tmeasured_paths\toracle_paths\tequal"
        ]
        for record in self.records:
            name = record["benchmark"]
            result_lines.append(f"{name}\t0.25\t1024\t0")
            if record["benchmark_operation"] in {
                "workspace_status",
                "workspace_diff",
                "workspace_record",
                "materialized_lane_record",
                "structured_patch",
            }:
                k = record["authoritative_input_k"]
                final_output = record["final_changed_output"]
                oracle_lines.append(
                    f"{name}\t1000\t{k}\t5000\t{final_output}\t{final_output}\t1"
                )
        self.results.write_text("\n".join(result_lines) + "\n")
        self.structural.write_text(
            "".join(json.dumps(record) + "\n" for record in self.records)
        )
        self.oracle.write_text("\n".join(oracle_lines) + "\n")

    def run_checker(self, *extra):
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                str(self.results),
                str(self.structural),
                "--oracle",
                str(self.oracle),
                *extra,
            ],
            capture_output=True,
            text=True,
            check=False,
        )

    def record(self, operation="workspace_status", k=1):
        return next(
            record
            for record in self.records
            if record["benchmark_operation"] == operation
            and record["authoritative_input_k"] == k
        )

    def assert_gate_failure(self, expected):
        self.write_fixture()
        result = self.run_checker()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn(expected, result.stderr)

    def test_accepts_complete_warm_artifacts(self):
        result = self.run_checker(
            "--max-seconds",
            "workspace_status=1",
            "--max-rss-bytes",
            "workspace_status=2048",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("thresholds passed", result.stdout)

    def test_rejects_missing_sidecar_report(self):
        self.records.remove(self.record("workspace_diff", 1))
        self.assert_gate_failure("missing structural metrics report")

    def test_rejects_duplicate_sidecar_report(self):
        self.write_fixture()
        with self.structural.open("a") as handle:
            handle.write(json.dumps(self.record("workspace_status", 0)) + "\n")
        result = self.run_checker()
        self.assertEqual(result.returncode, 2)
        self.assertIn("duplicate benchmark", result.stderr)

    def test_rejects_missing_required_structural_field(self):
        del self.record("workspace_status", 1)["filesystem_stat_count"]
        self.assert_gate_failure("missing structural fields")

    def test_rejects_any_warm_global_work(self):
        self.record("workspace_diff", 100)["full_filesystem_walk_count"] = 1
        self.assert_gate_failure("full_filesystem_walk_count must remain zero")

    def test_rejects_global_work_for_lane_patch_and_cow_operations(self):
        cases = (
            ("materialized_lane_record", "external_adapter_global_work"),
            ("structured_patch", "bounded_filesystem_walk_count"),
        )
        for operation, field in cases:
            with self.subTest(operation=operation, field=field):
                self.setUp()
                self.record(operation, 1)[field] = 1
                self.assert_gate_failure(f"{field} must remain zero")

        self.setUp()
        self.records.extend(structural_record("cow_checkpoint", k) for k in (0, 1, 100))
        self.record("cow_checkpoint", 1)["daemon_cumulative_rewrite_count"] = 1
        self.write_fixture()
        result = self.run_checker("--require-cow")
        self.assertEqual(result.returncode, 1)
        self.assertIn("daemon_cumulative_rewrite_count must remain zero", result.stderr)

    def test_rejects_incomplete_sqlite_accounting(self):
        self.record("workspace_status", 0)[
            "selected_worktree_index_sqlite_statement_count"
        ] = 1
        self.assert_gate_failure("SQLite work occurred on a not_applicable operation")

    def test_rejects_false_complete_sqlite_accounting_claim(self):
        self.record("workspace_status", 0)[
            "selected_worktree_index_sqlite_accounting_complete"
        ] = True
        self.assert_gate_failure("disposition must be independently proven not_applicable")

    def test_rejects_untyped_false_zero_sqlite_accounting(self):
        record = self.record("workspace_diff", 0)
        record["selected_worktree_index_sqlite_accounting_disposition"] = "ambiguous"
        record["selected_worktree_index_sqlite_not_applicable_count"] = 0
        self.assert_gate_failure("independently proven not_applicable")

    def test_rejects_missing_or_mixed_selected_index_envelope(self):
        missing = self.record("structured_patch", 0)
        missing["selected_worktree_index_sqlite_accounting_complete"] = False
        missing["selected_worktree_index_sqlite_accounting_disposition"] = "ambiguous"
        missing["selected_worktree_index_sqlite_envelope_count"] = 0
        self.write_fixture()
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("SQLite accounting must be complete", result.stderr)
        self.assertIn("requires an accounting envelope", result.stderr)

        self.setUp()
        mixed = self.record("materialized_lane_record", 1)
        mixed["selected_worktree_index_sqlite_accounting_complete"] = False
        mixed["selected_worktree_index_sqlite_accounting_disposition"] = "ambiguous"
        mixed["selected_worktree_index_sqlite_not_applicable_count"] = 1
        self.assert_gate_failure("mixes complete and not_applicable claims")

    def test_rejects_unknown_metrics_and_unbounded_tail_configuration(self):
        record = self.record("workspace_status", 1)
        record["unreviewed_repository_work_count"] = 0
        record["configured_tail_bound"] = 100000
        self.write_fixture()
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("unknown structural fields", result.stderr)
        self.assertIn("exceeds audited cap 4096", result.stderr)

    def test_rejects_repository_sized_values_for_every_work_counter_class(self):
        cases = (
            "filesystem_entry_count",
            "filesystem_hash_count",
            "filesystem_hash_bytes",
            "root_range_row_count",
            "prolly_read_key_count",
            "prolly_read_value_bytes",
            "prolly_write_key_count",
            "selection_comparison_count",
            "policy_dependency_file_count",
            "policy_dependency_bytes",
            "git_subprocess_count",
            "git_output_bytes",
            "git_output_record_count",
            "daemon_snapshot_path_count",
            "daemon_snapshot_bytes",
            "manifest_key_comparison_count",
            "journal_bytes",
        )
        for field in cases:
            with self.subTest(field=field):
                self.setUp()
                self.record("workspace_record", 1)[field] = 100_000_000
                self.assert_gate_failure(f"{field}=1e+08 exceeds O(k) bound")

    def test_rejects_repository_sized_sqlite_and_path_index_work(self):
        record = self.record("structured_patch", 1)
        record["selected_worktree_index_sqlite_statement_count"] = 100_000
        record["selected_worktree_index_sqlite_row_read_count"] = 100_000
        record["path_index_lookup_count"] = 100_000
        self.write_fixture()
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("selected_worktree_index_sqlite_statement_count", result.stderr)
        self.assertIn("selected_worktree_index_sqlite_row_read_count", result.stderr)
        self.assertIn("path_index_lookup_count", result.stderr)

    def test_rejects_policy_dependency_full_discovery(self):
        self.record("workspace_record", 1)["policy_dependency_full_discovery"] = 1
        self.assert_gate_failure("policy_dependency_full_discovery must remain zero")

    def test_rejects_candidate_and_ledger_locality_overruns(self):
        record = self.record("workspace_record", 1)
        record["filesystem_read_count"] = 99
        record["ledger_row_touch_count"] = 99
        self.write_fixture()
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("filesystem_read_count", result.stderr)
        self.assertIn("ledger_row_touch_count", result.stderr)

    def test_rejects_observer_tail_overrun(self):
        record = self.record("workspace_status", 100)
        record["observer_tail_record_fold_count"] = 4097
        self.assert_gate_failure("observer tail folded")

    def test_rejects_each_persisted_scope_cap(self):
        fields = [
            ("candidate_rows", "candidate_row_cap"),
            ("prefix_rows", "prefix_row_cap"),
            ("observer_log_bytes", "observer_log_byte_cap"),
            ("largest_segment_bytes", "segment_byte_cap"),
        ]
        for value, cap in fields:
            with self.subTest(value=value):
                self.setUp()
                scope = self.record("workspace_status", 0)["scope_caps"][0]
                scope[value] = scope[cap] + 1
                self.assert_gate_failure(value)

    def test_rejects_missing_or_mismatched_oracle(self):
        lines = self.oracle.read_text().splitlines()
        self.oracle.write_text("\n".join(line for line in lines if "workspace_diff_k1" not in line) + "\n")
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing separate full-scan oracle", result.stderr)

        self.write_fixture()
        self.oracle.write_text(self.oracle.read_text().replace(
            "ledger_workspace_status_k1\t1000\t1\t5000\t1\t1\t1",
            "ledger_workspace_status_k1\t1000\t1\t5000\t1\t0\t0",
        ))
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("oracle equality failed", result.stderr)

    def test_rejects_operation_report_recovery_walk(self):
        self.record("materialized_lane_record", 1)["upper_recovery_walks"] = 1
        self.assert_gate_failure("upper_recovery_walks must remain zero")

    def test_cow_requires_typed_journal_interval_accounting(self):
        self.records.extend(structural_record("cow_checkpoint", k) for k in (0, 1, 100))
        self.record("cow_checkpoint", 1)["generated_path_accounting"] = "recursive_inventory"
        self.write_fixture()
        result = self.run_checker("--require-cow")
        self.assertEqual(result.returncode, 1)
        self.assertIn("expected 'journal_interval'", result.stderr)

    def test_rejects_time_rss_and_output_count(self):
        self.record("structured_patch", 100)["final_changed_output"] = 99
        self.write_fixture()
        result = self.run_checker(
            "--max-seconds",
            "workspace_status=0.1",
            "--max-rss-bytes",
            "workspace_diff=512",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("0.25s > 0.10s", result.stderr)
        self.assertIn("RSS 1024", result.stderr)
        self.assertIn("final_changed_output=99", result.stderr)

    def test_require_cow_demands_all_candidate_counts(self):
        result = self.run_checker("--require-cow")
        self.assertEqual(result.returncode, 1)
        self.assertIn("ledger_cow_checkpoint_k0: missing", result.stderr)

    def test_require_cold_reconcile_demands_and_gates_timed_result(self):
        result = self.run_checker("--require-cold-reconcile")
        self.assertEqual(result.returncode, 1)
        self.assertIn("ledger_cold_reconcile: missing benchmark result", result.stderr)

        with self.results.open("a") as handle:
            handle.write("ledger_cold_reconcile\t2.5\t4096\t0\n")
        result = self.run_checker(
            "--require-cold-reconcile",
            "--max-seconds",
            "cold_reconcile=2",
            "--max-rss-bytes",
            "cold_reconcile=2048",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("ledger_cold_reconcile: 2.50s > 2.00s", result.stderr)
        self.assertIn("ledger_cold_reconcile: RSS 4096", result.stderr)

    def test_structured_patch_zero_is_a_real_successful_empty_operation(self):
        record = self.record("structured_patch", 0)
        self.assertEqual(record["authoritative_input_k"], 0)
        self.assertEqual(record["authoritative_candidate_count"], 0)
        self.assertEqual(record["final_changed_output"], 0)
        result = self.run_checker()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_exact_file_prefix_expansion_and_bounded_walk(self):
        record = self.record("workspace_status", 0)
        record["authoritative_candidate_count"] = 1
        record["expanded_path_count"] = 1000
        record["filesystem_stat_count"] = 1000
        record["filesystem_read_count"] = 1000
        record["bounded_filesystem_walk_count"] = 1
        self.write_fixture()
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("bounded_filesystem_walk_count must remain zero", result.stderr)
        self.assertIn("do not equal exact-file k=0", result.stderr)
        self.assertIn("expanded paths 1000 exceed exact-file k=0", result.stderr)

    def test_rejects_zero_or_nonfinite_measurements(self):
        self.write_fixture()
        self.results.write_text(
            self.results.read_text().replace(
                "ledger_materialized_lane_record_k0\t0.25\t1024\t0",
                "ledger_materialized_lane_record_k0\t0\t0\t0",
            )
        )
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("invalid max_rss_bytes='0'", result.stderr)

        self.write_fixture()
        self.results.write_text(
            self.results.read_text().replace(
                "ledger_materialized_lane_record_k0\t0.25\t1024\t0",
                "ledger_materialized_lane_record_k0\tnan\t1024\t0",
            )
        )
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("invalid real_seconds='nan'", result.stderr)


if __name__ == "__main__":
    unittest.main()

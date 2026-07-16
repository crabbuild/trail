import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("cli-scale-bench.sh")
ROOT = SCRIPT.parent.parent


class CliScaleBenchSourceTests(unittest.TestCase):
    def test_changed_path_oracle_excludes_native_macos_storage_noise(self):
        source = SCRIPT.read_text()
        function = source.split("write_changed_path_oracle_manifest() {", 1)[1].split(
            "\nPY\n}", 1
        )[0]
        program = function.split("<<'PY'\n", 1)[1].rsplit("\nPY", 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory) / "root"
            root.mkdir()
            (root / "source.txt").write_text("source")
            (root / "._source.txt").write_text("appledouble")
            (root / ".DS_Store").write_text("finder")
            (root / ".fseventsd").mkdir()
            (root / ".fseventsd" / "state").write_text("native")
            destination = pathlib.Path(directory) / "manifest.json"
            result = subprocess.run(
                [sys.executable, "-c", program, str(root), str(destination)],
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(destination.read_text()), {
                "source.txt": {
                    "kind": "file",
                    "sha256": "41cf6794ba4200b839c53531555f0f3998df4cbb01a4d5cb0b94e3ca5e23947d",
                    "executable": False,
                }
            })

    def test_cow_oracle_derives_from_scanned_baseline_and_reads_only_mutations(self):
        source = SCRIPT.read_text()
        function = source.split("prepare_cow_changed_path_external_oracle() {", 1)[1].split(
            "\nPY\n", 1
        )[0]
        program = function.split("<<'PY'\n", 1)[1]
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory) / "mounted-view"
            root.mkdir()
            mutation = root / "cow-1-000.txt"
            mutation.write_text("COW checkpoint 1:0\n")
            baseline = pathlib.Path(directory) / "baseline.json"
            baseline.write_text(json.dumps({
                "base.txt": {
                    "kind": "file",
                    "sha256": "baseline-digest",
                    "executable": False,
                }
            }))
            current = pathlib.Path(directory) / "current.json"
            changed = pathlib.Path(directory) / "changed.paths"
            result = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    program,
                    str(baseline),
                    str(root),
                    "1",
                    str(current),
                    str(changed),
                ],
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            manifest = json.loads(current.read_text())
            self.assertEqual(manifest["base.txt"]["sha256"], "baseline-digest")
            self.assertEqual(
                manifest["cow-1-000.txt"]["sha256"],
                hashlib.sha256(mutation.read_bytes()).hexdigest(),
            )
            self.assertEqual(changed.read_text(), "cow-1-000.txt\n")

    def test_changed_path_empty_measured_set_is_byte_identical_to_empty_oracle(self):
        source = SCRIPT.read_text()
        function = source.split("json_path_set() {", 1)[1].split("\n}\n", 1)[0]
        program = function.split("<<'PY'\n", 1)[1].rsplit("\nPY", 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            report = pathlib.Path(directory) / "report.json"
            for operation, payload in (
                ("workspace_status", {"changed_paths": []}),
                ("workspace_diff", {"files": []}),
            ):
                with self.subTest(operation=operation):
                    report.write_text(json.dumps(payload))
                    result = subprocess.run(
                        [sys.executable, "-c", program, operation, str(report)],
                        capture_output=True,
                        check=False,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout, b"")

    def test_changed_path_mode_separates_measured_metrics_and_oracle_work(self):
        source = SCRIPT.read_text()
        self.assertIn('MODE="${1:-default}"', source)
        self.assertIn('TRAIL_PERFORMANCE_METRICS_FILE="$work/changed-path-operation-metrics.jsonl"', source)
        self.assertLess(
            source.index('export TRAIL_PERFORMANCE_METRICS_FILE='),
            source.index('ledger_workspace_warm.status.json'),
        )
        self.assertLess(
            source.index('export TRAIL_PERFORMANCE_METRICS_FILE='),
            source.index('--json init --from-git'),
        )
        self.assertIn('handle.seek(start)', source)
        self.assertIn('expected one new matching metrics report', source)
        self.assertIn('>"$work/structural-metrics.jsonl"', source)
        self.assertIn('>"$work/oracle-results.tsv"', source)
        self.assertIn('write_changed_path_oracle_manifest', source)
        self.assertNotIn('TRAIL_PERFORMANCE_METRICS=0 "$BIN"', source)
        self.assertIn('LEDGER_K_VALUES="${TRAIL_CHANGED_PATH_K_VALUES:-0,1,100}"', source)
        self.assertIn('git -C "$repo" commit --quiet', source)
        self.assertIn('--json init --from-git', source)
        self.assertIn('run_timed "$scale" ledger_cold_reconcile', source)
        self.assertIn('for i in range(k):', source)
        self.assertNotIn('edit_count = 1 if k == 0 else k', source)
        self.assertIn('expected exactly one operation metrics report', source)
        self.assertIn('bounded_filesystem_walk_count', source)
        self.assertIn('missing or ambiguous /usr/bin/time real measurement', source)
        self.assertIn('missing, ambiguous, or zero /usr/bin/time RSS measurement', source)
        self.assertNotIn('ledger_structured_patch_k0.report.json', source)
        self.assertNotIn('ledger_structured_patch_k0 7 PATCH_REJECTED', source)
        self.assertIn('selected_worktree_index_sqlite_accounting_complete', source)
        self.assertIn('selected_worktree_index_sqlite_accounting_disposition', source)
        self.assertIn('selected_worktree_index_sqlite_not_applicable_count', source)
        self.assertIn('filesystem_hash_count', source)
        self.assertIn('prolly_read_key_count', source)
        self.assertIn('daemon_snapshot_path_count', source)
        self.assertIn('generated_path_accounting', source)
        self.assertIn('journal_interval', source)
        self.assertIn('policy_dependency_full_discovery', source)
        for operation in [
            "workspace_status",
            "workspace_diff",
            "workspace_record",
            "materialized_lane_record",
            "structured_patch",
            "cow_checkpoint",
        ]:
            self.assertIn(operation, source)
        self.assertIn('scripts/cli-scale-bench.sh changed-path-ledger', (ROOT / ".github/workflows/scale.yml").read_text())
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("Changed-path Ledger 1k Benchmark", ci)
        self.assertIn("check-changed-path-ledger-thresholds.py", ci)

    def test_native_changed_path_workflow_covers_linux_macos_and_large_scales(self):
        source = (ROOT / ".github/workflows/changed-path-ledger-native.yml").read_text()
        self.assertIn("ubuntu-latest", source)
        self.assertIn("macos-latest", source)
        self.assertIn("100000", source)
        self.assertIn("1000000", source)
        self.assertIn("changed_path_ledger_linux", source)
        self.assertIn("changed_path_ledger_macos", source)
        self.assertIn("check-changed-path-ledger-thresholds.py", source)
        self.assertIn("pull_request:", source)
        self.assertIn("branches: [main]", source)
        self.assertIn("findmnt --noheadings --output FSTYPE", source)
        self.assertIn("diskutil info", source)
        self.assertIn('df -P "${{ runner.temp }}"', source)
        self.assertIn('diskutil info "$device"', source)
        self.assertNotIn('diskutil info "${{ runner.temp }}"', source)
        self.assertIn("--require-cow", source)
        self.assertIn("--require-cold-reconcile", source)
        self.assertIn('--max-seconds "cow_checkpoint=$cow_seconds"', source)
        self.assertIn('--max-rss-bytes "cow_checkpoint=$rss"', source)

    def test_exact_sha_native_gates_block_tagging_and_every_dist_release(self):
        native = (ROOT / ".github/workflows/changed-path-ledger-native.yml").read_text()
        automation = (ROOT / ".github/workflows/release-automation.yml").read_text()
        release = (ROOT / ".github/workflows/release.yml").read_text()
        cargo = (ROOT / "Cargo.toml").read_text()
        activation = (
            ROOT / "trail/src/db/change_ledger/activation.rs"
        ).read_text()

        self.assertIn("workflow_call:", native)
        self.assertIn('default: "100000"', native)
        self.assertIn("default: true", native)
        self.assertIn("exact-sha-native-ledger:", automation)
        self.assertIn("needs: [release-please, exact-sha-native-ledger]", automation)
        self.assertIn('plan-jobs = ["./changed-path-ledger-native"]', cargo)
        self.assertIn("custom-changed-path-ledger-native:", release)
        custom_gate = release.split("custom-changed-path-ledger-native:", 1)[1]
        self.assertIn(
            "uses: ./.github/workflows/changed-path-ledger-native.yml",
            custom_gate,
        )
        build = release.split("build-local-artifacts:", 1)[1]
        self.assertIn("- custom-changed-path-ledger-native", build)
        self.assertIn("exact_sha_tag_gate=Release Automation/exact-sha-native-ledger", activation)
        self.assertIn(
            "exact_sha_publish_gate=Release/custom-changed-path-ledger-native",
            activation,
        )

    def test_unmapped_clone_uses_transport_instead_of_local_object_copy(self):
        source = SCRIPT.read_text()
        self.assertIn(
            'git clone --no-local --quiet "$GIT_REPO" "$GIT_UNMAPPED_REPO"',
            source,
        )

    def test_git_plumbing_command_ceiling_is_constant_in_ci_and_scale_gates(self):
        for relative in [".github/workflows/ci.yml", ".github/workflows/scale.yml"]:
            with self.subTest(workflow=relative):
                source = (ROOT / relative).read_text()
                self.assertIn("agent_git_plumbing_commands=5", source)

    def test_path_index_patch_and_bounded_record_reports_are_extracted(self):
        source = SCRIPT.read_text()
        self.assertIn('"$WORK/out/agent_apply_patch.stdout" patch', source)
        self.assertIn('"$WORK/out/path_index_record.stdout" record', source)
        self.assertIn('"$WORK/out/path_index_empty_patch.stdout" empty_root_patch', source)
        self.assertIn('"$WORK/out/path_index_rename_patch.stdout" rename_patch', source)
        self.assertIn('cat "$WORK/path-index-"*.tsv', source)

    def test_ci_and_scheduled_workflows_gate_path_index_structure(self):
        for relative in [".github/workflows/ci.yml", ".github/workflows/scale.yml"]:
            with self.subTest(workflow=relative):
                source = (ROOT / relative).read_text()
                for prefix in ["patch", "record", "empty_root_patch", "rename_patch"]:
                    self.assertIn(
                        f"{prefix}_path_index_full_root_path_load_count=0", source
                    )
                    self.assertIn(
                        f"{prefix}_path_index_full_filesystem_path_scan_count=0",
                        source,
                    )
                    self.assertIn(
                        f"{prefix}_path_index_mode=indexed", source
                    )


if __name__ == "__main__":
    unittest.main()

import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).with_name("cli-scale-bench.sh")
ROOT = SCRIPT.parent.parent


class CliScaleBenchSourceTests(unittest.TestCase):
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

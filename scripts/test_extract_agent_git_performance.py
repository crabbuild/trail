import importlib.util
import json
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("extract-agent-git-performance.py")
SPEC = importlib.util.spec_from_file_location("agent_git_performance", SCRIPT)
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class AgentGitPerformanceTests(unittest.TestCase):
    def report(self, **overrides):
        performance = {
            "export_mode": "mapped_delta",
            "changed_path_count": 1,
            "blob_write_count": 1,
            "git_plumbing_command_count": 5,
        }
        performance.update(overrides)
        return {"git_export": {"performance": performance}}

    def test_extracts_strict_performance_metrics(self):
        self.assertEqual(
            module.extract_performance_metrics(self.report()),
            {
                "agent_git_export_mode": "mapped_delta",
                "agent_git_changed_paths": 1,
                "agent_git_blob_writes": 1,
                "agent_git_plumbing_commands": 5,
            },
        )

    def test_rejects_invalid_export_mode_type(self):
        with self.assertRaisesRegex(ValueError, "export_mode must be a string"):
            module.extract_performance_metrics(self.report(export_mode=1))

    def test_rejects_non_integer_and_negative_counters(self):
        for key, value in [
            ("changed_path_count", True),
            ("changed_path_count", "1"),
            ("changed_path_count", 1.5),
            ("changed_path_count", -1),
            ("blob_write_count", False),
            ("blob_write_count", "1"),
            ("blob_write_count", 1.5),
            ("blob_write_count", -1),
            ("git_plumbing_command_count", False),
            ("git_plumbing_command_count", "5"),
            ("git_plumbing_command_count", 4.5),
            ("git_plumbing_command_count", -1),
        ]:
            with self.subTest(key=key, value=value):
                with self.assertRaisesRegex(ValueError, "nonnegative JSON integer"):
                    module.extract_performance_metrics(self.report(**{key: value}))

    def test_writes_validated_tsv(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "apply.json"
            output = root / "metrics.tsv"
            source.write_text(json.dumps(self.report()))
            module.extract_file(source, output)
            self.assertEqual(
                output.read_text(),
                "agent_git_export_mode\tmapped_delta\n"
                "agent_git_changed_paths\t1\n"
                "agent_git_blob_writes\t1\n"
                "agent_git_plumbing_commands\t5\n",
            )


if __name__ == "__main__":
    unittest.main()

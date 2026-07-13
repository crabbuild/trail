import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("check-cli-scale-thresholds.py")
SPEC = importlib.util.spec_from_file_location("scale_thresholds", SCRIPT)
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class ThresholdMetricTests(unittest.TestCase):
    def test_reads_structural_string_metrics(self):
        with tempfile.TemporaryDirectory() as directory:
            metrics = pathlib.Path(directory) / "metrics.tsv"
            metrics.write_text(
                "agent_git_export_mode\tmapped_delta\n"
                "agent_git_changed_paths\t1\n"
            )
            parsed = module.read_metric_values(metrics)
            self.assertEqual(parsed["agent_git_export_mode"], "mapped_delta")
            self.assertEqual(parsed["agent_git_changed_paths"], 1.0)

    def test_checks_numeric_ceiling_and_string_equality_together(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            results = root / "results.tsv"
            results.write_text(
                "name\treal_seconds\tmax_rss_bytes\texit_code\n"
                "agent_git_apply\t0.25\t1024\t0\n"
            )
            metrics = root / "metrics.tsv"
            metrics.write_text(
                "agent_git_export_mode\tmapped_delta\n"
                "agent_git_changed_paths\t1\n"
            )
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    str(results),
                    "agent_git_apply=1",
                    "--metrics",
                    str(metrics),
                    "agent_git_changed_paths=1",
                    "--metric-equals",
                    "agent_git_export_mode=mapped_delta",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("checked 3 CLI scale thresholds", result.stdout)


if __name__ == "__main__":
    unittest.main()

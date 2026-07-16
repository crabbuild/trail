import importlib.util
import json
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("extract-path-index-performance.py")
SPEC = importlib.util.spec_from_file_location("path_index_performance", SCRIPT)
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class PathIndexPerformanceTests(unittest.TestCase):
    def report(self, **path_index_overrides):
        path_index = {
            "mode": "indexed",
            "lookup_count": 2,
            "full_root_path_load_count": 0,
            "full_filesystem_path_scan_count": 0,
        }
        path_index.update(path_index_overrides)
        return {
            "operation": "change-1",
            "changed_paths": [
                {"path": "README.md", "old_path": "readme.md"},
                {"path": "src/lib.rs", "old_path": None},
            ],
            "path_index": path_index,
        }

    def test_extracts_operation_scoped_metrics_and_unique_folded_touches(self):
        self.assertEqual(
            module.extract_path_index_metrics(self.report(), "patch"),
            {
                "patch_path_index_mode": "indexed",
                "patch_path_index_lookup_count": 2,
                "patch_path_index_full_root_path_load_count": 0,
                "patch_path_index_full_filesystem_path_scan_count": 0,
                "patch_path_index_touched_folded_key_count": 2,
            },
        )

    def test_rejects_missing_or_malformed_metrics(self):
        invalid = [
            ({"operation": "change-1", "changed_paths": []}, "path_index"),
            (self.report(mode=1), "mode must be a string"),
            (self.report(lookup_count=True), "nonnegative JSON integer"),
            (self.report(lookup_count=-1), "nonnegative JSON integer"),
            (
                self.report(full_root_path_load_count="0"),
                "nonnegative JSON integer",
            ),
            (
                self.report(full_filesystem_path_scan_count=-1),
                "nonnegative JSON integer",
            ),
        ]
        for payload, message in invalid:
            with self.subTest(message=message):
                with self.assertRaisesRegex(ValueError, message):
                    module.extract_path_index_metrics(payload, "patch")

    def test_rejects_reports_without_one_completed_operation(self):
        for operation in [None, ""]:
            with self.subTest(operation=operation):
                payload = self.report()
                payload["operation"] = operation
                with self.assertRaisesRegex(ValueError, "completed operation"):
                    module.extract_path_index_metrics(payload, "record")

    def test_rejects_lookup_count_above_unique_folded_touched_paths(self):
        with self.assertRaisesRegex(ValueError, "exceeds 2 unique folded touched keys"):
            module.extract_path_index_metrics(self.report(lookup_count=3), "patch")

    def test_matches_rust_per_codepoint_lowercase_for_final_sigma(self):
        payload = self.report(lookup_count=1)
        payload["changed_paths"] = [{"path": "ΟΣ", "old_path": "οσ"}]
        metrics = module.extract_path_index_metrics(payload, "patch")
        self.assertEqual(metrics["patch_path_index_touched_folded_key_count"], 1)

    def test_rejects_malformed_changed_path_endpoints(self):
        payload = self.report()
        payload["changed_paths"] = [{"path": 1}]
        with self.assertRaisesRegex(ValueError, "changed_paths.*path must be a string"):
            module.extract_path_index_metrics(payload, "patch")

    def test_writes_validated_tsv(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "report.json"
            output = root / "metrics.tsv"
            source.write_text(json.dumps(self.report()))
            module.extract_file(source, "record", output)
            self.assertEqual(
                output.read_text(),
                "record_path_index_mode\tindexed\n"
                "record_path_index_lookup_count\t2\n"
                "record_path_index_full_root_path_load_count\t0\n"
                "record_path_index_full_filesystem_path_scan_count\t0\n"
                "record_path_index_touched_folded_key_count\t2\n",
            )


if __name__ == "__main__":
    unittest.main()

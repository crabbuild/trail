import importlib.util
import json
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("check-real-framework-handoff.py")
SPEC = importlib.util.spec_from_file_location("real_framework_checker", SCRIPT)
assert SPEC and SPEC.loader
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class RealFrameworkHandoffCheckerTests(unittest.TestCase):
    def write_report(self, raw, name, value):
        (raw / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")

    def fixture(self, root, framework):
        raw = root / "raw"
        raw.mkdir()
        component_id = {
            "go": "go-vendor",
            "pnpm": "node",
            "npm": "node",
            "python": "python-venv",
            "cmake": "cmake-build",
        }[framework]
        shared_key = "key-shared"
        shared_layer = "layer-shared"
        for index, lane in enumerate(CHECKER.LANES):
            key = f"key-{index}" if framework == "go" else shared_key
            if framework == "go":
                layer = f"layer-{index}"
                storage = layer
                output_name = "vendor"
            elif framework in {"pnpm", "npm"}:
                layer = shared_layer
                storage = layer
                output_name = "node_modules"
            else:
                layer = None
                storage = f"private_{index}"
                output_name = "venv" if framework == "python" else "build-tree"
            component = {
                "component_id": component_id,
                "component_key": key,
                "layer_id": layer,
                "caches": [
                    {
                        "name": "cache",
                        "namespace_id": "cache-shared",
                        "protocol": "content_store",
                    }
                ],
                "outputs": [
                    {
                        "name": output_name,
                        "storage_identity": storage,
                    }
                ],
            }
            generation = {
                "state": "active",
                "source_root": f"root-{index}",
                "components": [component],
            }
            self.write_report(raw, f"generation-{lane}", generation)
            if index:
                parent = json.loads(
                    (raw / f"generation-{CHECKER.LANES[index - 1]}.json").read_text(
                        encoding="utf-8"
                    )
                )
                before = parent
                if framework in {"python", "cmake"}:
                    before = json.loads(json.dumps(parent))
                    before["components"][0]["outputs"][0]["storage_identity"] = (
                        f"private_before_{index}"
                    )
            else:
                before = json.loads(json.dumps(generation))
                before["source_root"] = "root-baseline"
                if framework == "go":
                    before["components"][0]["component_key"] = "key-baseline"
                    before["components"][0]["layer_id"] = "layer-baseline"
            self.write_report(raw, f"generation-before-edit-{lane}", before)
            decision = {
                "component_id": component_id,
                "decision_source": (
                    "compatible_predecessor_seed" if framework == "go" else "active_binding"
                ),
                "bytes_avoided": 100 if framework == "go" else 0,
            }
            self.write_report(raw, f"sync-{lane}", {"decisions": [decision]})
            self.write_report(
                raw,
                f"precheck-{lane}",
                {
                    "exit_code": 0,
                    "lifecycle": {
                        "checkpoint": {
                            "source_paths": [],
                            "generated_dirty_paths": index + 1,
                        }
                    },
                },
            )
            self.write_report(
                raw,
                f"edit-{lane}",
                {
                    "lifecycle": {
                        "checkpoint": {
                            "operation": f"change-{index}",
                            "root_id": f"root-{index}",
                            "source_paths": CHECKER.SOURCE_PATHS[framework],
                            "generated_dirty_paths": index + 1,
                        }
                    }
                },
            )
            self.write_report(
                raw,
                f"check-{lane}",
                {
                    "exit_code": 0,
                    "lifecycle": {
                        "checkpoint": {
                            "source_paths": [],
                            "generated_dirty_paths": index + 1,
                        }
                    },
                },
            )
            inheritance = None
            if index:
                if framework in {"go", "pnpm", "npm"}:
                    inheritance = {
                        "status": "inherited",
                        "reason": None,
                        "outputs": [],
                    }
                else:
                    inheritance = {
                        "status": "skipped",
                        "reason": "no_compatible_outputs",
                        "outputs": [
                            {
                                "component_id": component_id,
                                "decision": "private",
                                "reason": "fresh_lane_private_upper",
                            }
                        ],
                    }
            self.write_report(
                raw,
                f"spawn-{lane}",
                {
                    "lane": lane,
                    "workdir": f"/workspace/{lane}",
                    "workdir_mode": "nfs-cow",
                    "base_change": "main" if not index else f"change-{index - 1}",
                    "environment_inheritance": inheritance,
                },
            )
        self.write_report(raw, "init", {"initialized": True})
        return component_id

    def test_accepts_each_framework_contract(self):
        for framework in ("go", "pnpm", "npm", "python", "cmake"):
            with self.subTest(framework=framework), tempfile.TemporaryDirectory() as temp:
                root = pathlib.Path(temp)
                component_id = self.fixture(root, framework)
                evidence = CHECKER.check_evidence(
                    root,
                    framework,
                    "https://example.invalid/repository.git",
                    "a" * 40,
                    component_id,
                )
                self.assertEqual(evidence["framework"], framework)
                self.assertTrue(evidence["assertions"]["framework_reuse_contract_passed"])
                self.assertEqual(evidence["generated_dirty_paths"], [1, 2, 3])
                self.assertEqual(len(evidence["raw_sha256"]), 22)

    def test_rejects_an_edit_that_captures_an_unexpected_path(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            component_id = self.fixture(root, "pnpm")
            report = json.loads((root / "raw/edit-agent-b.json").read_text(encoding="utf-8"))
            report["lifecycle"]["checkpoint"]["source_paths"].append("target/output")
            self.write_report(root / "raw", "edit-agent-b", report)
            with self.assertRaisesRegex(AssertionError, "unexpected source paths"):
                CHECKER.check_evidence(root, "pnpm", "repo", "rev", component_id)

    def test_rejects_go_without_compatible_predecessor_seed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            component_id = self.fixture(root, "go")
            report = json.loads((root / "raw/sync-agent-b.json").read_text(encoding="utf-8"))
            report["decisions"][0]["decision_source"] = "singleflight_builder"
            self.write_report(root / "raw", "sync-agent-b", report)
            with self.assertRaisesRegex(AssertionError, "did not seed from its predecessor"):
                CHECKER.check_evidence(root, "go", "repo", "rev", component_id)

    def test_rejects_private_output_inheritance(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            component_id = self.fixture(root, "python")
            report = json.loads(
                (root / "raw/spawn-agent-b.json").read_text(encoding="utf-8")
            )
            report["environment_inheritance"] = {
                "status": "inherited",
                "reason": None,
                "outputs": [],
            }
            self.write_report(root / "raw", "spawn-agent-b", report)
            with self.assertRaisesRegex(AssertionError, "unexpectedly inherited"):
                CHECKER.check_evidence(root, "python", "repo", "rev", component_id)


if __name__ == "__main__":
    unittest.main()

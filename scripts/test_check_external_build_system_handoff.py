import importlib.util
import json
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("check-external-build-system-handoff.py")
SPEC = importlib.util.spec_from_file_location("external_handoff_checker", SCRIPT)
assert SPEC and SPEC.loader
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class ExternalBuildSystemHandoffCheckerTests(unittest.TestCase):
    def write(self, raw, name, value):
        (raw / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")

    def fixture(self, root, system="gradle"):
        raw = root / "raw"
        raw.mkdir()
        identity = f"trail-examples/{system}@1"
        if system == "node-native":
            identity = "trail/node@1"
            digest = "builtin:node-plan-v3"
        else:
            digest = "sha256:" + "d" * 64
        component_id = f"external-build.{system}"

        def nix_artifacts(index):
            suffix = str(index + 1)
            return [
                {
                    "name": "nix-builder",
                    "artifact_type": "oci_image",
                    "provider": "oci",
                    "reference": CHECKER.NIX_BUILDER_IMAGE,
                    "digest": CHECKER.NIX_BUILDER_DIGEST,
                    "platform": CHECKER.NIX_PLATFORM,
                    "cleanup_owner": "external",
                },
                {
                    "name": "package",
                    "artifact_type": "verified_external",
                    "provider": "nix",
                    "reference": f"/nix/store/{suffix * 32}-package",
                    "digest": "sha256:" + suffix * 64,
                    "platform": CHECKER.NIX_PLATFORM,
                    "cleanup_owner": "external",
                },
                {
                    "name": "check",
                    "artifact_type": "verified_external",
                    "provider": "nix",
                    "reference": f"/nix/store/{suffix * 32}-check",
                    "digest": "sha256:" + suffix * 64,
                    "platform": CHECKER.NIX_PLATFORM,
                    "cleanup_owner": "external",
                },
            ]

        def outputs(index):
            names = ("profile", "state") if system == "nix" else ("build",)
            return [
                {
                    "name": name,
                    "policy": "writable_private",
                    "publish": "never",
                    "layer_id": None,
                    "storage_identity": f"private_{index}_{name}",
                }
                for name in names
            ]
        self.write(
            raw,
            "distribution" if system == "node-native" else "plugin-install",
            {"canonical_identity": identity, "distribution_digest": digest},
        )
        self.write(
            raw,
            "conformance",
            {
                "exit_code": 0,
                "assertions": {
                    "hostile_plan_rejected": True,
                    "recovery_preserved_prior_generation": True,
                    "redaction_passed": True,
                },
            },
        )
        checkpoints = []
        for index, lane in enumerate(CHECKER.LANES):
            checkpoint = {
                "operation": f"change-{index}",
                "root_id": f"root-{index}",
                "source_paths": [f"src/change-{index}.txt"],
                "generated_dirty_paths": 0,
            }
            checkpoints.append(checkpoint)
            self.write(raw, f"checkpoint-{lane}", checkpoint)
            self.write(
                raw,
                f"spawn-{lane}",
                {
                    "base_change": "main" if not index else f"change-{index - 1}",
                    "workdir": f"/workspace/{lane}",
                    "workdir_mode": "nfs-cow",
                },
            )
            plan = {
                "adapter_identity": identity,
                "component_id": component_id,
                "component_key": f"key-{index}",
                "source_root": f"root-{index}",
                "tools": {} if system == "nix" else {"tool": "sha256:tool"},
                "external_artifacts": nix_artifacts(index) if system == "nix" else [],
                "capabilities": {
                    "network": "none" if system == "nix" else "outbound-deny",
                    "shell": (
                        "approved-process-tree"
                        if system == "node-native"
                        else "none" if system == "nix" else "deny"
                    ),
                    "scripts": (
                        "exact-committed-approval" if system == "node-native" else "deny"
                    ),
                    "secrets": "none" if system == "nix" else "deny",
                },
                "adapter_distribution_digest": digest,
            }
            self.write(raw, f"plan-{lane}", plan)
            self.write(raw, f"plan-{lane}-repeat", plan)
            component = {
                "component_id": component_id,
                "component_key": f"key-{index}",
                "layer_id": None,
                "outputs": outputs(index),
                "caches": (
                    []
                    if system == "nix"
                    else [{"name": "downloads", "namespace_id": "cache-shared"}]
                ),
                "external_artifacts": nix_artifacts(index) if system == "nix" else [],
            }
            self.write(
                raw,
                f"sync-{lane}",
                {
                    "generation": {
                        "state": "active",
                        "source_root": f"root-{index}",
                        "components": [component],
                    },
                    "decisions": [
                        {"component_id": component_id, "desired_key": f"key-{index}"}
                    ],
                },
            )
            self.write(raw, f"validation-{lane}", {"exit_code": 0})

        self.write(
            raw,
            "spawn-invalidation",
            {
                "base_change": checkpoints[-1]["operation"],
                "workdir": "/workspace/invalidation",
                "workdir_mode": "nfs-cow",
            },
        )
        invalidation_checkpoint = {
            "operation": "change-invalidation",
            "root_id": "root-invalidation",
            "source_paths": ["settings.gradle"],
            "generated_dirty_paths": 0,
        }
        self.write(raw, "checkpoint-invalidation", invalidation_checkpoint)
        invalidation_plan = {
            "adapter_identity": identity,
            "component_id": component_id,
            "component_key": "key-invalidation",
            "source_root": "root-invalidation",
            "tools": {} if system == "nix" else {"tool": "sha256:tool"},
            "external_artifacts": nix_artifacts(2) if system == "nix" else [],
            "capabilities": {
                "network": "none" if system == "nix" else "outbound-deny",
                "shell": (
                    "approved-process-tree"
                    if system == "node-native"
                    else "none" if system == "nix" else "deny"
                ),
                "scripts": (
                    "exact-committed-approval" if system == "node-native" else "deny"
                ),
                "secrets": "none" if system == "nix" else "deny",
            },
            "adapter_distribution_digest": digest,
        }
        self.write(raw, "plan-invalidation", invalidation_plan)
        self.write(raw, "plan-invalidation-repeat", invalidation_plan)
        invalidation_component = {
            "component_id": component_id,
            "component_key": "key-invalidation",
            "layer_id": None,
            "outputs": outputs("invalidation"),
            "caches": (
                []
                if system == "nix"
                else [{"name": "downloads", "namespace_id": "cache-shared"}]
            ),
            "external_artifacts": nix_artifacts(2) if system == "nix" else [],
        }
        self.write(
            raw,
            "sync-invalidation",
            {
                "generation": {
                    "state": "active",
                    "source_root": "root-invalidation",
                    "components": [invalidation_component],
                },
                "decisions": [
                    {"component_id": component_id, "desired_key": "key-invalidation"}
                ],
            },
        )
        self.write(raw, "validation-invalidation", {"exit_code": 0})
        if system == "node-native":
            self.write(
                raw,
                "security-network-denial",
                {
                    "exit_code": 0,
                    "network_error": "EPERM",
                    "outbound_connect_denied": True,
                },
            )
            self.write(
                raw,
                "security-write-denial",
                {
                    "exit_code": 2,
                    "canary_created": False,
                    "active_generation": None,
                    "undeclared_write_denied": True,
                },
            )
        return component_id

    def check(self, root, system="gradle"):
        component_id = self.fixture(root, system)
        return CHECKER.check_evidence(root, system, "repo", "revision", component_id)

    def test_accepts_all_external_system_contracts(self):
        for system in CHECKER.SYSTEMS:
            with self.subTest(system=system), tempfile.TemporaryDirectory() as temp:
                evidence = self.check(pathlib.Path(temp), system)
                self.assertEqual(evidence["schema"], "trail.ecosystem-certification/v1")
                self.assertEqual(
                    evidence["distribution"]["kind"],
                    "built-in" if system == "node-native" else "external-adapter",
                )
                self.assertEqual(len(evidence["lane_ancestry"]), 3)

    def test_accepts_windows_workdirs_when_verified_on_any_host(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            component_id = self.fixture(root)
            for index, lane in enumerate(CHECKER.LANES):
                self.write(
                    root / "raw",
                    f"spawn-{lane}",
                    {
                        "base_change": "main" if not index else f"change-{index - 1}",
                        "workdir": f"D:\\trail\\{lane}",
                        "workdir_mode": "dokan-cow",
                    },
                )
            evidence = CHECKER.check_evidence(
                root, "gradle", "repo", "revision", component_id
            )
            self.assertEqual(evidence["backend"], "dokan-cow")
            self.assertEqual(
                evidence["workdirs"],
                [f"D:\\trail\\{lane}" for lane in CHECKER.LANES],
            )

    def test_rejects_windows_workdirs_that_differ_only_by_case(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            component_id = self.fixture(root)
            for lane, workdir in zip(
                CHECKER.LANES,
                (r"D:\trail\lane", r"d:\TRAIL\LANE", r"D:\trail\lane-c"),
                strict=True,
            ):
                path = root / "raw" / f"spawn-{lane}.json"
                report = json.loads(path.read_text(encoding="utf-8"))
                report["workdir"] = workdir
                self.write(root / "raw", f"spawn-{lane}", report)
            with self.assertRaisesRegex(AssertionError, "distinct absolute workdirs"):
                CHECKER.check_evidence(
                    root, "gradle", "repo", "revision", component_id
                )

    def test_rejects_relative_workdir(self):
        self.mutate_and_reject(
            "spawn-agent-b",
            lambda report: report.update(workdir="workspace/agent-b"),
            "distinct absolute workdirs",
        )

    def mutate_and_reject(self, name, mutate, message):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            component_id = self.fixture(root)
            path = root / "raw" / f"{name}.json"
            report = json.loads(path.read_text(encoding="utf-8"))
            mutate(report)
            self.write(root / "raw", name, report)
            with self.assertRaisesRegex(AssertionError, message):
                CHECKER.check_evidence(root, "gradle", "repo", "revision", component_id)

    def test_rejects_package_substitution(self):
        self.mutate_and_reject(
            "plan-agent-b",
            lambda report: report.update(adapter_distribution_digest="sha256:" + "e" * 64),
            "nondeterministic|distribution digest",
        )

    def test_rejects_nondeterministic_plan(self):
        self.mutate_and_reject(
            "plan-agent-b-repeat",
            lambda report: report.update(component_key="key-other"),
            "nondeterministic",
        )

    def test_rejects_wrong_ancestry(self):
        self.mutate_and_reject(
            "spawn-agent-c",
            lambda report: report.update(base_change="change-unrelated"),
            "did not start from its parent",
        )

    def test_rejects_cache_drift(self):
        self.mutate_and_reject(
            "sync-agent-b",
            lambda report: report["generation"]["components"][0]["caches"][0].update(
                namespace_id="cache-other"
            ),
            "cache declarations changed",
        )

    def test_rejects_private_output_reuse(self):
        self.mutate_and_reject(
            "sync-agent-c",
            lambda report: report["generation"]["components"][0]["outputs"][0].update(
                storage_identity="private_1_build"
            ),
            "storage was reused",
        )

    def mutate_nix_and_reject(self, name, mutate, message):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            component_id = self.fixture(root, "nix")
            path = root / "raw" / f"{name}.json"
            report = json.loads(path.read_text(encoding="utf-8"))
            mutate(report)
            self.write(root / "raw", name, report)
            with self.assertRaisesRegex(AssertionError, message):
                CHECKER.check_evidence(root, "nix", "repo", "revision", component_id)

    def test_nix_rejects_builder_substitution(self):
        self.mutate_nix_and_reject(
            "plan-agent-a-repeat",
            lambda report: report["external_artifacts"][0].update(
                reference="nixos/nix@sha256:" + "e" * 64,
                digest="sha256:" + "e" * 64,
            ),
            "nondeterministic|pinned Nix builder",
        )

    def test_nix_rejects_writable_or_malformed_store_identity(self):
        self.mutate_nix_and_reject(
            "sync-agent-b",
            lambda report: report["generation"]["components"][0][
                "external_artifacts"
            ][1].update(cleanup_owner="trail", reference="/tmp/package"),
            "verified immutable Nix store identity",
        )

    def test_rejects_failed_validation(self):
        self.mutate_and_reject(
            "validation-agent-c",
            lambda report: report.update(exit_code=1),
            "validations failed",
        )

    def test_rejects_raw_tampering_after_seal(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            component_id = self.fixture(root)
            evidence = CHECKER.check_evidence(
                root, "gradle", "repo", "revision", component_id
            )
            (root / "evidence.json").write_text(
                json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
            validation = json.loads(
                (root / "raw/validation-agent-c.json").read_text(encoding="utf-8")
            )
            validation["detail"] = "tampered"
            self.write(root / "raw", "validation-agent-c", validation)
            with self.assertRaisesRegex(AssertionError, "authoritative raw reports"):
                CHECKER.verify_sealed_evidence(root)


if __name__ == "__main__":
    unittest.main()

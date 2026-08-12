#!/usr/bin/env python3
"""Validate and seal external build-system Agent A -> B -> C evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
from pathlib import Path, PurePosixPath, PureWindowsPath
from typing import Any


LANES = ("agent-a", "agent-b", "agent-c")
SYSTEMS = {"node-native", "bazel", "gradle", "maven", "nix"}
NIX_BUILDER_IMAGE = (
    "nixos/nix@sha256:286285edfc390096bd7e8aada40c5044dadff1eb0b60f28b193eef7ed52e5925"
)
NIX_BUILDER_DIGEST = "sha256:" + "286285edfc390096bd7e8aada40c5044dadff1eb0b60f28b193eef7ed52e5925"
NIX_PLATFORM = "linux/arm64"


def portable_absolute_path_identity(value: Any) -> tuple[str, str] | None:
    """Return a stable identity for an absolute POSIX or Windows evidence path."""
    if not isinstance(value, str):
        return None
    windows_path = PureWindowsPath(value)
    if windows_path.is_absolute():
        return ("windows", str(windows_path).casefold())
    posix_path = PurePosixPath(value)
    if posix_path.is_absolute():
        return ("posix", str(posix_path))
    return None


def load_report(raw: Path, name: str) -> dict[str, Any]:
    value = json.loads((raw / f"{name}.json").read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise AssertionError(f"{name}.json is not a JSON object")
    return value


def select_component(generation: dict[str, Any], component_id: str) -> dict[str, Any]:
    matches = [
        component
        for component in generation.get("components", [])
        if component.get("component_id") == component_id
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"expected one {component_id!r} component, found "
            f"{[item.get('component_id') for item in generation.get('components', [])]!r}"
        )
    return matches[0]


def contains_value(value: Any, expected: str) -> bool:
    if value == expected:
        return True
    if isinstance(value, dict):
        return any(contains_value(item, expected) for item in value.values())
    if isinstance(value, list):
        return any(contains_value(item, expected) for item in value)
    return False


def assert_plan_security(plan: dict[str, Any], lane: str, system: str) -> None:
    capabilities = plan.get("capabilities")
    if not isinstance(capabilities, dict):
        raise AssertionError(f"{lane} plan omitted capabilities")
    network = capabilities.get("network")
    expected_denial = network == "none" if system == "nix" else (
        isinstance(network, str) and "deny" in network
    )
    if not expected_denial:
        raise AssertionError(f"{lane} plan did not deny outbound network: {network!r}")
    expected_secret_denial = "none" if system == "nix" else "deny"
    if capabilities.get("secrets") != expected_secret_denial:
        raise AssertionError(f"{lane} plan did not deny secrets")
    if system == "node-native":
        if capabilities.get("shell") != "approved-process-tree" or capabilities.get(
            "scripts"
        ) != "exact-committed-approval":
            raise AssertionError(f"{lane} plan lost exact lifecycle approval bounds")
    elif capabilities.get("shell") != ("none" if system == "nix" else "deny"):
        raise AssertionError(f"{lane} plan did not deny shell")


def assert_plan_matches_sync(
    lane: str,
    plan: dict[str, Any],
    checkpoint: dict[str, Any],
    sync: dict[str, Any],
    component_id: str,
) -> dict[str, Any]:
    generation = sync.get("generation")
    if not isinstance(generation, dict) or generation.get("state") != "active":
        raise AssertionError(f"{lane} did not publish an active generation")
    component = select_component(generation, component_id)
    if plan.get("source_root") != checkpoint.get("root_id"):
        raise AssertionError(f"{lane} plan source root does not match its checkpoint")
    if generation.get("source_root") != checkpoint.get("root_id"):
        raise AssertionError(f"{lane} generation source root does not match its checkpoint")
    if plan.get("component_key") != component.get("component_key"):
        raise AssertionError(f"{lane} plan and generation component identities differ")
    decisions = [
        decision
        for decision in sync.get("decisions", [])
        if decision.get("component_id") == component_id
    ]
    if len(decisions) != 1 or decisions[0].get("desired_key") != plan.get("component_key"):
        raise AssertionError(f"{lane} sync decision does not match its plan")
    return component


def assert_private_outputs(
    components: list[dict[str, Any]], expected_names: set[str] | None = None
) -> tuple[list[dict[str, str]], set[str]]:
    output_storage: list[dict[str, str]] = []
    all_storage: set[str] = set()
    for component in components:
        outputs = component.get("outputs")
        if not isinstance(outputs, list) or not outputs:
            raise AssertionError("each external component must declare private output")
        names = {output.get("name") for output in outputs}
        if expected_names is None:
            expected_names = names
        elif names != expected_names:
            raise AssertionError("private output declarations changed across lanes")
        lane_storage: dict[str, str] = {}
        for output in outputs:
            storage = output.get("storage_identity")
            if (
                output.get("policy") != "writable_private"
                or output.get("publish") != "never"
                or output.get("layer_id") is not None
                or not isinstance(storage, str)
                or not storage.startswith("private_")
            ):
                raise AssertionError(f"unsafe external output contract: {output!r}")
            if storage in all_storage:
                raise AssertionError("lane-private output storage was reused across lanes")
            all_storage.add(storage)
            lane_storage[output["name"]] = storage
        output_storage.append(lane_storage)
    return output_storage, expected_names or set()


def nix_artifact_identities(
    report: dict[str, Any], label: str
) -> tuple[dict[str, str], dict[str, tuple[str, str]]]:
    artifacts = report.get("external_artifacts")
    if not isinstance(artifacts, list):
        raise AssertionError(f"{label} omitted Nix external artifacts")
    builders = [artifact for artifact in artifacts if artifact.get("name") == "nix-builder"]
    if len(builders) != 1:
        raise AssertionError(f"{label} must declare exactly one pinned Nix builder")
    builder = builders[0]
    if builder != {
        "name": "nix-builder",
        "artifact_type": "oci_image",
        "provider": "oci",
        "reference": NIX_BUILDER_IMAGE,
        "digest": NIX_BUILDER_DIGEST,
        "platform": NIX_PLATFORM,
        "cleanup_owner": "external",
    }:
        raise AssertionError(f"{label} changed the exact pinned Nix builder identity")
    stores = {
        artifact.get("name"): artifact
        for artifact in artifacts
        if artifact.get("name") != "nix-builder"
    }
    if set(stores) != {"package", "check"} or len(artifacts) != 3:
        raise AssertionError(f"{label} must declare exactly package and check Nix stores")
    identities: dict[str, tuple[str, str]] = {}
    for name, artifact in stores.items():
        reference = artifact.get("reference")
        digest = artifact.get("digest")
        digest_hex = digest.removeprefix("sha256:") if isinstance(digest, str) else ""
        store_name = reference.removeprefix("/nix/store/") if isinstance(reference, str) else ""
        if (
            artifact.get("artifact_type") != "verified_external"
            or artifact.get("provider") != "nix"
            or artifact.get("platform") != NIX_PLATFORM
            or artifact.get("cleanup_owner") != "external"
            or not isinstance(reference, str)
            or not reference.startswith("/nix/store/")
            or "/" in store_name
            or len(store_name) < 34
            or store_name[32] != "-"
            or len(digest_hex) != 64
            or any(character not in "0123456789abcdef" for character in digest_hex)
        ):
            raise AssertionError(
                f"{label} {name!r} is not a verified immutable Nix store identity"
            )
        identities[name] = (reference, digest)
    return {"external:nix-builder": NIX_BUILDER_IMAGE}, identities


def check_evidence(
    evidence_dir: Path,
    system: str,
    repository: str,
    revision: str,
    component_id: str,
) -> dict[str, Any]:
    if system not in SYSTEMS:
        raise AssertionError(f"unsupported external build system {system!r}")
    raw = evidence_dir / "raw"
    distribution_report_name = "distribution" if system == "node-native" else "plugin-install"
    package = load_report(raw, distribution_report_name)
    conformance = load_report(raw, "conformance")
    spawns = [load_report(raw, f"spawn-{lane}") for lane in LANES]
    checkpoints = [load_report(raw, f"checkpoint-{lane}") for lane in LANES]
    plans = [load_report(raw, f"plan-{lane}") for lane in LANES]
    repeated_plans = [load_report(raw, f"plan-{lane}-repeat") for lane in LANES]
    syncs = [load_report(raw, f"sync-{lane}") for lane in LANES]
    validations = [load_report(raw, f"validation-{lane}") for lane in LANES]

    identity = package.get("canonical_identity")
    package_digest = package.get("distribution_digest")
    if not isinstance(identity, str) or not identity:
        raise AssertionError("plugin install report omitted canonical identity")
    expected_digest_prefix = "builtin:" if system == "node-native" else "sha256:"
    if not isinstance(package_digest, str) or not package_digest.startswith(
        expected_digest_prefix
    ):
        raise AssertionError("plugin install report omitted distribution digest")
    if conformance.get("exit_code") != 0:
        raise AssertionError("adapter conformance did not pass")
    assertions = conformance.get("assertions")
    if not isinstance(assertions, dict) or not assertions or not all(
        value is True for value in assertions.values()
    ):
        raise AssertionError("adapter conformance assertions are incomplete or failed")

    if any(plan != repeated for plan, repeated in zip(plans, repeated_plans, strict=True)):
        raise AssertionError("external adapter plan is nondeterministic")
    if any(plan.get("adapter_identity") != identity for plan in plans):
        raise AssertionError("plan adapter identity does not match installed distribution")
    if any(not contains_value(plan, package_digest) for plan in plans):
        raise AssertionError("plan identity does not bind the installed distribution digest")
    if system == "nix":
        nix_plan_identities = [
            nix_artifact_identities(plan, f"{lane} plan")
            for lane, plan in zip(LANES, plans, strict=True)
        ]
        tools = [identity[0] for identity in nix_plan_identities]
    else:
        nix_plan_identities = []
        tools = [plan.get("tools") for plan in plans]
        if any(not isinstance(item, dict) or not item for item in tools):
            raise AssertionError("each plan must record exact tool identities")
    if any(item != tools[0] for item in tools[1:]):
        raise AssertionError("tool identities changed across source-only lanes")
    for lane, plan in zip(LANES, plans, strict=True):
        assert_plan_security(plan, lane, system)

    workdirs = [spawn.get("workdir") for spawn in spawns]
    backends = [spawn.get("workdir_mode") for spawn in spawns]
    workdir_identities = [portable_absolute_path_identity(item) for item in workdirs]
    if None in workdir_identities or len(set(workdir_identities)) != 3:
        raise AssertionError("A, B, and C need distinct absolute workdirs")
    if len(set(backends)) != 1 or backends[0] not in {
        "fuse-cow",
        "nfs-cow",
        "dokan-cow",
    }:
        raise AssertionError(f"A, B, and C need one transparent-COW backend: {backends!r}")
    for index in (1, 2):
        if spawns[index].get("base_change") != checkpoints[index - 1].get("operation"):
            raise AssertionError(f"{LANES[index]} did not start from its parent checkpoint")
    for lane, checkpoint in zip(LANES, checkpoints, strict=True):
        if not checkpoint.get("source_paths"):
            raise AssertionError(f"{lane} checkpoint omitted semantic source paths")
        generated_dirty = checkpoint.get("generated_dirty_paths")
        if not isinstance(generated_dirty, int) or generated_dirty < 0:
            raise AssertionError(f"{lane} checkpoint has invalid generated-path accounting")

    components = [
        assert_plan_matches_sync(lane, plan, checkpoint, sync, component_id)
        for lane, plan, checkpoint, sync in zip(
            LANES, plans, checkpoints, syncs, strict=True
        )
    ]
    if system == "nix":
        nix_component_identities = [
            nix_artifact_identities(component, f"{lane} generation")
            for lane, component in zip(LANES, components, strict=True)
        ]
        if any(
            component_identity != plan_identity
            for component_identity, plan_identity in zip(
                nix_component_identities, nix_plan_identities, strict=True
            )
        ):
            raise AssertionError("Nix plan and active generation artifact identities differ")
        for name in ("package", "check"):
            if len(
                {
                    identity[1][name]
                    for identity in nix_component_identities
                }
            ) != 3:
                raise AssertionError(
                    f"Agent A, B, and C must have distinct Nix {name} store identities"
                )
    component_keys = [component["component_key"] for component in components]
    if len(set(component_keys)) != 3:
        raise AssertionError("source-sensitive external component keys must be distinct")
    caches = [component.get("caches", []) for component in components]
    if any(item != caches[0] for item in caches[1:]):
        raise AssertionError("correctness-neutral cache declarations changed across lanes")
    output_storage, output_names = assert_private_outputs(components)
    if any(validation.get("exit_code") != 0 for validation in validations):
        raise AssertionError("one or more semantic validations failed")

    invalidation_spawn = load_report(raw, "spawn-invalidation")
    invalidation_checkpoint = load_report(raw, "checkpoint-invalidation")
    invalidation_plan = load_report(raw, "plan-invalidation")
    invalidation_plan_repeat = load_report(raw, "plan-invalidation-repeat")
    invalidation_sync = load_report(raw, "sync-invalidation")
    invalidation_validation = load_report(raw, "validation-invalidation")
    if invalidation_spawn.get("base_change") != checkpoints[-1].get("operation"):
        raise AssertionError("invalidation lane did not start from Agent C")
    invalidation_generated_dirty = invalidation_checkpoint.get("generated_dirty_paths")
    if (
        not isinstance(invalidation_generated_dirty, int)
        or invalidation_generated_dirty < 0
        or not invalidation_checkpoint.get("source_paths")
    ):
        raise AssertionError("invalidation checkpoint is incomplete or polluted")
    if invalidation_plan != invalidation_plan_repeat:
        raise AssertionError("invalidation plan is nondeterministic")
    if invalidation_plan.get("adapter_identity") != identity or not contains_value(
        invalidation_plan, package_digest
    ):
        raise AssertionError("invalidation plan lost adapter distribution identity")
    invalidation_nix_identity = (
        nix_artifact_identities(invalidation_plan, "invalidation plan")
        if system == "nix"
        else None
    )
    invalidation_tools = (
        invalidation_nix_identity[0]
        if invalidation_nix_identity is not None
        else invalidation_plan.get("tools")
    )
    if invalidation_tools != tools[0]:
        raise AssertionError("invalidation plan changed tool identity")
    assert_plan_security(invalidation_plan, "invalidation", system)
    invalidation_component = assert_plan_matches_sync(
        "invalidation",
        invalidation_plan,
        invalidation_checkpoint,
        invalidation_sync,
        component_id,
    )
    if system == "nix":
        invalidation_component_identity = nix_artifact_identities(
            invalidation_component, "invalidation generation"
        )
        if invalidation_component_identity != invalidation_nix_identity:
            raise AssertionError(
                "Nix invalidation plan and active generation artifact identities differ"
            )
        if invalidation_component_identity[1] != nix_component_identities[-1][1]:
            raise AssertionError(
                "lockfile authority-only invalidation unexpectedly changed Nix store results"
            )
    if invalidation_component["component_key"] == component_keys[-1]:
        raise AssertionError("identity-input invalidation reused Agent C's component key")
    if invalidation_component.get("caches", []) != caches[-1]:
        raise AssertionError("identity invalidation lost correctness-neutral caches")
    invalidation_storage, _ = assert_private_outputs(
        [invalidation_component], expected_names=output_names
    )
    if set(invalidation_storage[0].values()) & {
        value for lane_storage in output_storage for value in lane_storage.values()
    }:
        raise AssertionError("identity invalidation reused prior private output storage")
    if invalidation_validation.get("exit_code") != 0:
        raise AssertionError("identity invalidation validation failed")

    security_assertions: dict[str, bool] = {}
    if system == "node-native":
        network_denial = load_report(raw, "security-network-denial")
        write_denial = load_report(raw, "security-write-denial")
        if (
            network_denial.get("exit_code") != 0
            or network_denial.get("network_error") not in {"EPERM", "EACCES"}
            or network_denial.get("outbound_connect_denied") is not True
        ):
            raise AssertionError("Node lifecycle outbound-network denial evidence failed")
        if (
            not isinstance(write_denial.get("exit_code"), int)
            or write_denial["exit_code"] == 0
            or write_denial.get("canary_created") is not False
            or write_denial.get("active_generation") is not None
            or write_denial.get("undeclared_write_denied") is not True
        ):
            raise AssertionError("Node lifecycle undeclared-write denial evidence failed")
        security_assertions = {
            "real_outbound_connect_denied": True,
            "real_undeclared_write_denied_without_activation": True,
        }

    expected_names = {
        f"{distribution_report_name}.json",
        "conformance.json",
        *(f"spawn-{lane}.json" for lane in LANES),
        *(f"checkpoint-{lane}.json" for lane in LANES),
        *(f"plan-{lane}.json" for lane in LANES),
        *(f"plan-{lane}-repeat.json" for lane in LANES),
        *(f"sync-{lane}.json" for lane in LANES),
        *(f"validation-{lane}.json" for lane in LANES),
        "spawn-invalidation.json",
        "checkpoint-invalidation.json",
        "plan-invalidation.json",
        "plan-invalidation-repeat.json",
        "sync-invalidation.json",
        "validation-invalidation.json",
    }
    if system == "node-native":
        expected_names.update(
            {"security-network-denial.json", "security-write-denial.json"}
        )
    raw_hashes = {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(raw.glob("*.json"))
    }
    if set(raw_hashes) != expected_names:
        raise AssertionError(
            f"raw evidence set mismatch: missing={sorted(expected_names - set(raw_hashes))!r} "
            f"extra={sorted(set(raw_hashes) - expected_names)!r}"
        )

    lane_ancestry = [
        {
            "lane": lane,
            "base_change": spawn["base_change"],
            "checkpoint_operation": checkpoint["operation"],
            "checkpoint_root": checkpoint["root_id"],
        }
        for lane, spawn, checkpoint in zip(LANES, spawns, checkpoints, strict=True)
    ]
    cache_namespaces = [
        {cache["name"]: cache["namespace_id"] for cache in component.get("caches", [])}
        for component in components
    ]
    return {
        "schema": "trail.ecosystem-certification/v1",
        "framework": system,
        "repository": repository,
        "revision": revision,
        "distribution": {
            "kind": "built-in" if system == "node-native" else "external-adapter",
            "adapter_identity": identity,
            "distribution_digest": package_digest,
            "package_digest": None if system == "node-native" else package_digest,
        },
        "platform": {
            "operating_system": platform.system().lower(),
            "architecture": platform.machine().lower(),
            "workspace_backend": backends[0],
        },
        "backend": backends[0],
        "lanes": list(LANES),
        "lane_ancestry": lane_ancestry,
        "validations": validations,
        "workdirs": workdirs,
        "component_id": component_id,
        "adapter_identity": identity,
        "tool_identities": tools[0],
        "source_roots": [component_sync["generation"]["source_root"] for component_sync in syncs],
        "component_keys": component_keys,
        "layer_ids": [component.get("layer_id") for component in components],
        "cache_namespaces": cache_namespaces,
        "output_storage": output_storage,
        "generated_dirty_paths": [
            checkpoint["generated_dirty_paths"] for checkpoint in checkpoints
        ],
        "invalidation": {
            "source_root": invalidation_sync["generation"]["source_root"],
            "source_paths": invalidation_checkpoint["source_paths"],
            "before_component_key": component_keys[-1],
            "after_component_key": invalidation_component["component_key"],
            "cache_namespaces_preserved": True,
            "semantic_check_passed": True,
        },
        "conformance": assertions,
        "assertions": {
            "three_distinct_source_roots": True,
            "each_child_spawned_from_parent_semantic_checkpoint": True,
            "exact_adapter_distribution_bound_into_every_plan": True,
            "deterministic_planning": True,
            "exact_tool_identity_stable": True,
            "outbound_network_and_secrets_denied": True,
            "shell_or_approved_process_tree_policy_bounded": True,
            "correctness_neutral_cache_namespace_preserved": True,
            "lane_private_outputs_never_shared": True,
            "identity_input_change_rejected_stale_component": True,
            "all_semantic_validations_passed": True,
            "common_malicious_package_conformance_passed": True,
            "nix_external_immutable_identity_verified": system == "nix",
            **security_assertions,
        },
        "raw_sha256": raw_hashes,
    }


def verify_sealed_evidence(evidence_dir: Path) -> dict[str, Any]:
    evidence = json.loads((evidence_dir / "evidence.json").read_text(encoding="utf-8"))
    if not isinstance(evidence, dict):
        raise AssertionError("evidence.json is not a JSON object")
    if evidence.get("schema") != "trail.ecosystem-certification/v1":
        raise AssertionError(f"unsupported evidence schema: {evidence.get('schema')!r}")
    expected = check_evidence(
        evidence_dir,
        evidence.get("framework"),
        evidence.get("repository"),
        evidence.get("revision"),
        evidence.get("component_id"),
    )
    if evidence != expected:
        mismatches = sorted(
            key
            for key in set(evidence) | set(expected)
            if evidence.get(key) != expected.get(key)
        )
        raise AssertionError(
            "sealed evidence does not match authoritative raw reports: "
            f"fields={mismatches!r}"
        )
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("evidence_dir", type=Path)
    parser.add_argument("system", nargs="?")
    parser.add_argument("repository", nargs="?")
    parser.add_argument("revision", nargs="?")
    parser.add_argument("component_id", nargs="?")
    args = parser.parse_args()
    if args.verify:
        evidence = verify_sealed_evidence(args.evidence_dir)
    else:
        missing = [
            name
            for name in ("system", "repository", "revision", "component_id")
            if getattr(args, name) is None
        ]
        if missing:
            parser.error(f"sealing evidence requires: {', '.join(missing)}")
        evidence = check_evidence(
            args.evidence_dir,
            args.system,
            args.repository,
            args.revision,
            args.component_id,
        )
        (args.evidence_dir / "evidence.json").write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        verify_sealed_evidence(args.evidence_dir)
    print(json.dumps(evidence, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

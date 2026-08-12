#!/usr/bin/env python3
"""Validate and seal one real-framework Agent A -> B -> C evidence directory."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
from pathlib import Path
from typing import Any


LANES = ("agent-a", "agent-b", "agent-c")
SOURCE_PATHS = {
    "go": ["version/version.go", "version/version_test.go"],
    "go-workspace": ["common/must.go", "common/must_test.go"],
    "yarn": ["index.js", "test.js"],
    "bun": ["src/app.ts", "tests/app.test.ts"],
    "uv": ["src/pyprojectx/__init__.py", "tests/unit/test_trail_qualification.py"],
    "pnpm": [
        "src/constants.ts",
        "tests/http-helpers/index.test.ts",
    ],
    "npm": ["src/test/version.test.ts", "src/version.ts"],
    "python": ["src/tap/line.py", "tests/test_line.py"],
    "cmake": ["util/hash.cc", "util/hash.h"],
    "cmake-modern": ["examples/minimal.cpp"],
}
INVALIDATION_PATHS = {
    "yarn": [".yarnrc"],
    "bun": ["bunfig.toml"],
}


def load_report(raw: Path, name: str) -> dict[str, Any]:
    value = json.loads((raw / f"{name}.json").read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise AssertionError(f"{name}.json is not a JSON object")
    return value


def select_component(generation: dict[str, Any], component_id: str) -> dict[str, Any]:
    matches = [
        item
        for item in generation["components"]
        if item["component_id"] == component_id
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"expected one {component_id!r} component, found "
            f"{[item['component_id'] for item in generation['components']]!r}"
        )
    return matches[0]


def verify_sealed_evidence(evidence_dir: Path) -> dict[str, Any]:
    """Recompute canonical evidence from raw reports and reject any drift."""
    evidence_path = evidence_dir / "evidence.json"
    evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
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


def check_evidence(
    evidence_dir: Path,
    framework: str,
    repository: str,
    revision: str,
    component_id: str,
) -> dict[str, Any]:
    if framework not in {
        "go",
        "go-workspace",
        "yarn",
        "bun",
        "uv",
        "pnpm",
        "npm",
        "python",
        "cmake",
        "cmake-modern",
    }:
        raise AssertionError(f"unsupported framework {framework!r}")
    raw = evidence_dir / "raw"
    generations = [load_report(raw, f"generation-{lane}") for lane in LANES]
    before_generations = [
        load_report(raw, f"generation-before-edit-{lane}") for lane in LANES
    ]
    syncs = [load_report(raw, f"sync-{lane}") for lane in LANES]
    plans = [load_report(raw, f"plan-{lane}") for lane in LANES]
    spawns = [load_report(raw, f"spawn-{lane}") for lane in LANES]
    prechecks = [load_report(raw, f"precheck-{lane}") for lane in LANES]
    edits = [load_report(raw, f"edit-{lane}") for lane in LANES]
    checks = [load_report(raw, f"check-{lane}") for lane in LANES]
    components = [select_component(generation, component_id) for generation in generations]
    workdirs = [spawn["workdir"] for spawn in spawns]
    workdir_modes = [spawn.get("workdir_mode") for spawn in spawns]

    if not all(generation["state"] == "active" for generation in generations):
        raise AssertionError("all final generations must be active")
    adapter_identities = [plan.get("adapter_identity") for plan in plans]
    if len(set(adapter_identities)) != 1 or not adapter_identities[0]:
        raise AssertionError(f"adapter identity changed or is missing: {adapter_identities!r}")
    tool_identities = [plan.get("tools") for plan in plans]
    if any(not isinstance(tools, dict) or not tools for tools in tool_identities):
        raise AssertionError("each plan must record at least one executable identity")
    if any(tools != tool_identities[0] for tools in tool_identities[1:]):
        raise AssertionError("tool executable identities changed across source-only lanes")
    for lane, plan, before in zip(LANES, plans, before_generations, strict=True):
        if plan.get("component_id") != component_id:
            raise AssertionError(f"{lane} plan selected the wrong component")
        if plan.get("source_root") != before.get("source_root"):
            raise AssertionError(f"{lane} plan is not pinned to its pre-edit source root")
        if not isinstance(plan.get("component_key"), str) or not plan["component_key"]:
            raise AssertionError(f"{lane} plan omitted its canonical component key")
    if len({generation["source_root"] for generation in generations}) != 3:
        raise AssertionError("A, B, and C must have distinct source roots")
    if len(set(workdirs)) != 3 or any(not Path(workdir).is_absolute() for workdir in workdirs):
        raise AssertionError("A, B, and C must have distinct absolute lane workdirs")
    if len(set(workdir_modes)) != 1 or workdir_modes[0] not in {
        "fuse-cow",
        "nfs-cow",
        "dokan-cow",
    }:
        raise AssertionError(
            f"A, B, and C must use one transparent-COW backend: {workdir_modes!r}"
        )
    if not all(report["exit_code"] == 0 for report in [*prechecks, *checks]):
        raise AssertionError("every parent and edited framework check must exit successfully")
    for lane, report in zip(LANES, [*prechecks], strict=True):
        checkpoint = report["lifecycle"]["checkpoint"]
        if checkpoint["source_paths"]:
            raise AssertionError(
                f"{lane} parent build checkpointed source paths: {checkpoint!r}"
            )
    generated_dirty_paths = []
    for lane, edit in zip(LANES, edits, strict=True):
        checkpoint = edit["lifecycle"]["checkpoint"]
        if checkpoint["source_paths"] != SOURCE_PATHS[framework]:
            raise AssertionError(f"{lane} checkpointed unexpected source paths: {checkpoint!r}")
        generated_dirty = checkpoint["generated_dirty_paths"]
        if not isinstance(generated_dirty, int) or generated_dirty < 0:
            raise AssertionError(
                f"{lane} reported invalid generated-path accounting: {checkpoint!r}"
            )
        generated_dirty_paths.append(generated_dirty)
    for lane, report in zip(LANES, checks, strict=True):
        checkpoint = report["lifecycle"]["checkpoint"]
        if checkpoint["source_paths"]:
            raise AssertionError(
                f"{lane} edited build checkpointed source paths: {checkpoint!r}"
            )

    for child_index, child in enumerate(LANES[1:], start=1):
        parent_operation = edits[child_index - 1]["lifecycle"]["checkpoint"]["operation"]
        if spawns[child_index]["base_change"] != parent_operation:
            raise AssertionError(
                f"{child} did not start from its parent semantic checkpoint"
            )
    for lane, generation, edit in zip(LANES, generations, edits, strict=True):
        checkpoint = edit["lifecycle"]["checkpoint"]
        if generation["source_root"] != checkpoint["root_id"]:
            raise AssertionError(f"{lane} final generation is not pinned to its edited source")

    cache_namespaces = [
        {cache["name"]: cache["namespace_id"] for cache in item["caches"]}
        for item in components
    ]
    if any(set(caches) != set(cache_namespaces[0]) for caches in cache_namespaces[1:]):
        raise AssertionError("cache declarations differ across lanes")
    for cache_name in cache_namespaces[0]:
        if len({caches[cache_name] for caches in cache_namespaces}) != 1:
            raise AssertionError(f"cache namespace {cache_name!r} was not inherited")

    component_keys = [item["component_key"] for item in components]
    layer_ids = [item["layer_id"] for item in components]
    output_storage = [
        {output["name"]: output["storage_identity"] for output in item["outputs"]}
        for item in components
    ]
    if any(not storage for storage in output_storage):
        raise AssertionError("each lane must report at least one output")

    if framework in {"go", "go-workspace"}:
        if len(set(component_keys)) != 3:
            raise AssertionError("Go source edits must produce exact distinct component keys")
        if not all(layer_ids) or len(set(layer_ids)) != 3:
            raise AssertionError("Go source edits must publish distinct exact vendor layers")
        for lane, sync in zip(LANES[1:], syncs[1:], strict=True):
            decisions = [
                decision
                for decision in sync["decisions"]
                if decision["component_id"] == component_id
            ]
            if len(decisions) != 1:
                raise AssertionError(f"{lane} has unexpected decisions: {decisions!r}")
            decision = decisions[0]
            if decision["decision_source"] != "compatible_predecessor_seed":
                raise AssertionError(f"{lane} did not seed from its predecessor: {decision!r}")
            if not isinstance(decision["bytes_avoided"], int) or decision["bytes_avoided"] <= 0:
                raise AssertionError(f"{lane} avoided no predecessor bytes: {decision!r}")
    elif framework in {"yarn", "bun", "pnpm", "npm"}:
        if len(set(component_keys)) != 1:
            raise AssertionError("Node dependency identity changed after source-only edits")
        if not layer_ids[0] or len(set(layer_ids)) != 1:
            raise AssertionError("Node lanes did not reuse one exact dependency layer")
    else:
        if framework == "uv":
            if len(set(component_keys)) != 3:
                raise AssertionError("uv project source edits must change private environment identity")
        elif len(set(component_keys)) != 1:
            raise AssertionError("private environment identity changed after source-only edits")
        if layer_ids != [None, None, None]:
            raise AssertionError("Python/CMake private outputs must not publish shared layers")
        if any(
            not storage_identity.startswith("private_")
            for storage in output_storage
            for storage_identity in storage.values()
        ):
            raise AssertionError("Python/CMake outputs must use private storage contracts")

    shared_outputs = framework in {
        "go",
        "go-workspace",
        "yarn",
        "bun",
        "pnpm",
        "npm",
    }
    for child_index, (child, parent_generation) in enumerate(
        zip(LANES[1:], generations[:-1], strict=True), start=1
    ):
        inheritance = spawns[child_index].get("environment_inheritance")
        if shared_outputs:
            if not isinstance(inheritance, dict) or inheritance.get("status") != "inherited":
                raise AssertionError(f"{child} did not report inherited outputs: {inheritance!r}")
            inherited = select_component(before_generations[child_index], component_id)
            parent = select_component(parent_generation, component_id)
            if inherited["component_key"] != parent["component_key"]:
                raise AssertionError(f"{child} did not inherit its parent component key")
            if inherited["layer_id"] != parent["layer_id"]:
                raise AssertionError(f"{child} did not inherit its parent layer")
            if inherited["caches"] != parent["caches"]:
                raise AssertionError(f"{child} did not inherit its parent caches")
        else:
            if not isinstance(inheritance, dict):
                raise AssertionError(f"{child} has no private-output decision report")
            if (
                inheritance.get("status") != "skipped"
                or inheritance.get("reason") != "no_compatible_outputs"
            ):
                raise AssertionError(
                    f"{child} unexpectedly inherited a lane-private output: {inheritance!r}"
                )
            decisions = [
                item
                for item in inheritance.get("outputs", [])
                if item.get("component_id") == component_id
            ]
            if not decisions or any(
                item.get("decision") != "private"
                or item.get("reason") != "fresh_lane_private_upper"
                for item in decisions
            ):
                raise AssertionError(
                    f"{child} did not require a fresh lane-private output: {decisions!r}"
                )
            before = select_component(before_generations[child_index], component_id)
            parent = select_component(parent_generation, component_id)
            if before["component_key"] != parent["component_key"]:
                raise AssertionError(
                    f"{child} private environment changed dependency identity before its edit"
                )
            if before["caches"] != parent["caches"]:
                raise AssertionError(f"{child} private environment lost parent caches")

    first_before = select_component(before_generations[0], component_id)
    if framework in {"go", "go-workspace", "uv"}:
        if first_before["component_key"] == components[0]["component_key"]:
            raise AssertionError("source-sensitive environment identity did not change after edit")
    elif first_before["component_key"] != components[0]["component_key"]:
        raise AssertionError("source-only edit changed dependency/build environment identity")

    invalidation = None
    if framework in INVALIDATION_PATHS:
        invalidation_spawn = load_report(raw, "spawn-invalidation")
        invalidation_before_report = load_report(raw, "generation-before-invalidation")
        invalidation_edit = load_report(raw, "invalidation-edit")
        invalidation_sync = load_report(raw, "sync-invalidation")
        invalidation_check = load_report(raw, "check-invalidation")
        invalidation_generation = load_report(raw, "generation-invalidation")
        invalidation_before = select_component(invalidation_before_report, component_id)
        invalidation_after = select_component(invalidation_generation, component_id)
        if invalidation_spawn["base_change"] != edits[-1]["lifecycle"]["checkpoint"]["operation"]:
            raise AssertionError("invalidation lane did not start from Agent C")
        checkpoint = invalidation_edit["lifecycle"]["checkpoint"]
        if checkpoint["source_paths"] != INVALIDATION_PATHS[framework]:
            raise AssertionError(f"invalidation changed unexpected source paths: {checkpoint!r}")
        if invalidation_generation["source_root"] != checkpoint["root_id"]:
            raise AssertionError("invalidation generation is not pinned to its policy edit")
        if invalidation_before["component_key"] != components[-1]["component_key"]:
            raise AssertionError("invalidation lane did not inherit Agent C's component")
        if invalidation_after["component_key"] == invalidation_before["component_key"]:
            raise AssertionError("manager policy invalidation reused a stale component key")
        if not invalidation_after["layer_id"] or invalidation_after["layer_id"] == invalidation_before["layer_id"]:
            raise AssertionError("manager policy invalidation reused a stale dependency layer")
        if invalidation_after["caches"] != invalidation_before["caches"]:
            raise AssertionError("manager policy invalidation lost correctness-neutral caches")
        if invalidation_check["exit_code"] != 0:
            raise AssertionError("invalidated dependency layer failed semantic validation")
        if invalidation_check["lifecycle"]["checkpoint"]["source_paths"]:
            raise AssertionError("invalidation validation checkpointed unexpected source")
        decisions = [
            item
            for item in invalidation_sync["decisions"]
            if item["component_id"] == component_id
        ]
        if len(decisions) != 1:
            raise AssertionError(f"invalidation has unexpected decisions: {decisions!r}")
        invalidation = {
            "lane": "invalidation",
            "source_root": invalidation_generation["source_root"],
            "policy_paths": INVALIDATION_PATHS[framework],
            "before_component_key": invalidation_before["component_key"],
            "after_component_key": invalidation_after["component_key"],
            "before_layer_id": invalidation_before["layer_id"],
            "after_layer_id": invalidation_after["layer_id"],
            "cache_namespaces_preserved": True,
            "semantic_check_passed": True,
        }

    raw_hashes = {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(raw.glob("*.json"))
    }
    expected_names = {
        "init.json",
        *(f"spawn-{lane}.json" for lane in LANES),
        *(f"precheck-{lane}.json" for lane in LANES),
        *(f"generation-before-edit-{lane}.json" for lane in LANES),
        *(f"edit-{lane}.json" for lane in LANES),
        *(f"sync-{lane}.json" for lane in LANES),
        *(f"plan-{lane}.json" for lane in LANES),
        *(f"check-{lane}.json" for lane in LANES),
        *(f"generation-{lane}.json" for lane in LANES),
    }
    if framework in INVALIDATION_PATHS:
        expected_names.update(
            {
                "spawn-invalidation.json",
                "generation-before-invalidation.json",
                "invalidation-edit.json",
                "sync-invalidation.json",
                "check-invalidation.json",
                "generation-invalidation.json",
            }
        )
    if set(raw_hashes) != expected_names:
        raise AssertionError(
            f"raw evidence set mismatch: missing={sorted(expected_names - set(raw_hashes))!r} "
            f"extra={sorted(set(raw_hashes) - expected_names)!r}"
        )

    lane_ancestry = []
    validations = []
    for lane, spawn, edit, precheck, check in zip(
        LANES, spawns, edits, prechecks, checks, strict=True
    ):
        checkpoint = edit["lifecycle"]["checkpoint"]
        lane_ancestry.append(
            {
                "lane": lane,
                "base_change": spawn["base_change"],
                "checkpoint_operation": checkpoint["operation"],
                "checkpoint_root": checkpoint["root_id"],
            }
        )
        validations.append(
            {
                "lane": lane,
                "parent_exit_code": precheck["exit_code"],
                "edited_exit_code": check["exit_code"],
            }
        )

    return {
        "schema": "trail.ecosystem-certification/v1",
        "framework": framework,
        "repository": repository,
        "revision": revision,
        "distribution": {
            "kind": "built-in",
            "adapter_identity": adapter_identities[0],
            "package_digest": None,
        },
        "platform": {
            "operating_system": platform.system().lower(),
            "architecture": platform.machine().lower(),
            "workspace_backend": workdir_modes[0],
        },
        "backend": workdir_modes[0],
        "lanes": list(LANES),
        "lane_ancestry": lane_ancestry,
        "validations": validations,
        "workdirs": workdirs,
        "component_id": component_id,
        "adapter_identity": adapter_identities[0],
        "tool_identities": tool_identities[0],
        "source_roots": [generation["source_root"] for generation in generations],
        "component_keys": component_keys,
        "layer_ids": layer_ids,
        "cache_namespaces": cache_namespaces,
        "output_storage": output_storage,
        "generated_dirty_paths": generated_dirty_paths,
        "invalidation": invalidation,
        "assertions": {
            "three_distinct_source_roots": True,
            "each_child_spawned_from_parent_semantic_checkpoint": True,
            "shared_parent_generation_inherited_before_each_child_edit": shared_outputs,
            "private_outputs_reprovisioned_per_child_lane": not shared_outputs,
            "exact_framework_source_and_test_paths_per_edit": True,
            "parent_semantics_valid_before_each_edit": True,
            "edited_semantics_valid_after_each_edit": True,
            "dependency_identity_stable_for_source_independent_adapters": framework
            not in {"go", "go-workspace", "uv"},
            "go_vendor_identity_tracks_source_sensitive_vendor_inputs": framework
            in {"go", "go-workspace"},
            "go_multi_module_workspace_graph_verified": framework == "go-workspace",
            "uv_project_identity_tracks_source_authority": framework == "uv",
            "cmake_incremental_recompile_and_link_behavior_verified": framework
            in {"cmake", "cmake-modern"},
            "cmake_preset_ninja_ccache_private_output_verified": framework
            == "cmake-modern",
            "stale_framework_output_rejected_by_lane_marker": True,
            "generated_paths_excluded_from_source_checkpoint": True,
            "all_framework_checks_passed": True,
            "framework_reuse_contract_passed": True,
            "manager_policy_invalidation_published_new_exact_layer": framework
            in INVALIDATION_PATHS,
        },
        "raw_sha256": raw_hashes,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--verify",
        action="store_true",
        help="verify an existing evidence.json against its authoritative raw reports",
    )
    parser.add_argument("evidence_dir", type=Path)
    parser.add_argument("framework", nargs="?")
    parser.add_argument("repository", nargs="?")
    parser.add_argument("revision", nargs="?")
    parser.add_argument("component_id", nargs="?")
    args = parser.parse_args()
    if args.verify:
        evidence = verify_sealed_evidence(args.evidence_dir)
        print(json.dumps(evidence, indent=2, sort_keys=True))
        return 0
    missing = [
        name
        for name in ("framework", "repository", "revision", "component_id")
        if getattr(args, name) is None
    ]
    if missing:
        parser.error(f"sealing evidence requires: {', '.join(missing)}")
    evidence = check_evidence(
        args.evidence_dir,
        args.framework,
        args.repository,
        args.revision,
        args.component_id,
    )
    encoded = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    (args.evidence_dir / "evidence.json").write_text(encoded, encoding="utf-8")
    verify_sealed_evidence(args.evidence_dir)
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

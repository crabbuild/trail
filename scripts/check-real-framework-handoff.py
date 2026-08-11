#!/usr/bin/env python3
"""Validate and seal one real-framework Agent A -> B -> C evidence directory."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


LANES = ("agent-a", "agent-b", "agent-c")


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


def check_evidence(
    evidence_dir: Path,
    framework: str,
    repository: str,
    revision: str,
    component_id: str,
) -> dict[str, Any]:
    if framework not in {"go", "pnpm", "npm", "python", "cmake"}:
        raise AssertionError(f"unsupported framework {framework!r}")
    raw = evidence_dir / "raw"
    generations = [load_report(raw, f"generation-{lane}") for lane in LANES]
    syncs = [load_report(raw, f"sync-{lane}") for lane in LANES]
    spawns = [load_report(raw, f"spawn-{lane}") for lane in LANES]
    edits = [load_report(raw, f"edit-{lane}") for lane in LANES]
    checks = [load_report(raw, f"check-{lane}") for lane in LANES]
    components = [select_component(generation, component_id) for generation in generations]

    if not all(generation["state"] == "active" for generation in generations):
        raise AssertionError("all final generations must be active")
    if len({generation["source_root"] for generation in generations}) != 3:
        raise AssertionError("A, B, and C must have distinct source roots")
    if not all(check["exit_code"] == 0 for check in checks):
        raise AssertionError("every framework check must exit successfully")
    for lane, edit in zip(LANES, edits, strict=True):
        checkpoint = edit["lifecycle"]["checkpoint"]
        if checkpoint["source_paths"] != ["README.md"]:
            raise AssertionError(f"{lane} checkpointed unexpected source paths: {checkpoint!r}")
        if checkpoint["generated_dirty_paths"] != 0:
            raise AssertionError(f"{lane} checkpointed generated paths: {checkpoint!r}")

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

    if framework == "go":
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
    elif framework in {"pnpm", "npm"}:
        if len(set(component_keys)) != 1:
            raise AssertionError("Node dependency identity changed after source-only edits")
        if not layer_ids[0] or len(set(layer_ids)) != 1:
            raise AssertionError("Node lanes did not reuse one exact dependency layer")
    else:
        if len(set(component_keys)) != 1:
            raise AssertionError("private environment identity changed after source-only edits")
        if layer_ids != [None, None, None]:
            raise AssertionError("Python/CMake private outputs must not publish shared layers")
        identities = [next(iter(storage.values())) for storage in output_storage]
        if len(set(identities)) != 3:
            raise AssertionError("Python/CMake outputs must remain lane-private")

    shared_outputs = framework in {"go", "pnpm", "npm"}
    for child_index, (child, parent_generation) in enumerate(
        zip(LANES[1:], generations[:-1], strict=True), start=1
    ):
        inheritance = spawns[child_index].get("environment_inheritance")
        if shared_outputs:
            if not isinstance(inheritance, dict) or inheritance.get("status") != "inherited":
                raise AssertionError(f"{child} did not report inherited outputs: {inheritance!r}")
            inherited = select_component(
                load_report(raw, f"generation-before-edit-{child}"), component_id
            )
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

    raw_hashes = {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(raw.glob("*.json"))
    }
    expected_names = {
        "init.json",
        *(f"spawn-{lane}.json" for lane in LANES),
        *(f"edit-{lane}.json" for lane in LANES),
        *(f"sync-{lane}.json" for lane in LANES),
        *(f"check-{lane}.json" for lane in LANES),
        *(f"generation-{lane}.json" for lane in LANES),
    }
    if shared_outputs:
        expected_names.update(
            {
                "generation-before-edit-agent-b.json",
                "generation-before-edit-agent-c.json",
            }
        )
    if set(raw_hashes) != expected_names:
        raise AssertionError(
            f"raw evidence set mismatch: missing={sorted(expected_names - set(raw_hashes))!r} "
            f"extra={sorted(set(raw_hashes) - expected_names)!r}"
        )

    return {
        "schema": "trail.real-framework-handoff/v1",
        "framework": framework,
        "repository": repository,
        "revision": revision,
        "backend": "nfs-cow",
        "lanes": list(LANES),
        "component_id": component_id,
        "source_roots": [generation["source_root"] for generation in generations],
        "component_keys": component_keys,
        "layer_ids": layer_ids,
        "cache_namespaces": cache_namespaces,
        "output_storage": output_storage,
        "assertions": {
            "three_distinct_source_roots": True,
            "shared_parent_generation_inherited_before_each_child_edit": shared_outputs,
            "private_outputs_reprovisioned_per_child_lane": not shared_outputs,
            "exactly_one_readme_source_path_per_edit": True,
            "zero_generated_paths_checkpointed": True,
            "all_framework_checks_passed": True,
            "framework_reuse_contract_passed": True,
        },
        "raw_sha256": raw_hashes,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence_dir", type=Path)
    parser.add_argument("framework")
    parser.add_argument("repository")
    parser.add_argument("revision")
    parser.add_argument("component_id")
    args = parser.parse_args()
    evidence = check_evidence(
        args.evidence_dir,
        args.framework,
        args.repository,
        args.revision,
        args.component_id,
    )
    encoded = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    (args.evidence_dir / "evidence.json").write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

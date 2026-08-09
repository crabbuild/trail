#!/usr/bin/env python3
"""Fail closed when layered-lane scale evidence is missing or overstated."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any


MATRIX = ((10_000, 1), (100_000, 5), (1_000_000, 20))
NATIVE_GATES = {"nfs_macos", "fuse_linux", "dokan_windows"}


def fail(message: str) -> None:
    raise SystemExit(f"layered-lane scale evidence: {message}")


def load(path: Path) -> dict[str, Any]:
    if not path.is_file():
        fail(f"missing {path}")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read {path}: {error}")
    if not isinstance(value, dict):
        fail(f"{path} must contain a JSON object")
    return value


def require_nonnegative(value: Any, field: str, path: Path) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        fail(f"{path}: {field} must be a non-negative integer")
    return value


def check_source(path: Path, entries: int, lanes: int) -> None:
    report = load(path)
    if report.get("schema") != "trail.layered-lane-scale/v1":
        fail(f"{path}: unexpected source scale schema")
    if report.get("tracked_paths") != entries or report.get("lanes") != lanes:
        fail(f"{path}: source scale matrix identity is incorrect")
    if report.get("indexed_paths_per_view") != 1:
        fail(f"{path}: source views did not remain lazy")
    require_nonnegative(report.get("exclusive_physical_bytes"), "exclusive_physical_bytes", path)
    skipped = report.get("skipped_native_gates")
    if not isinstance(skipped, list) or not set(skipped).issubset(NATIVE_GATES):
        fail(f"{path}: skipped_native_gates is invalid")


def check_artifact(path: Path, entries: int, lanes: int) -> None:
    report = load(path)
    if report.get("schema") != "trail.artifact-lane-scale/v1":
        fail(f"{path}: unexpected artifact scale schema")
    if report.get("artifact_entries") != entries or report.get("lanes") != lanes:
        fail(f"{path}: artifact scale matrix identity is incorrect")
    if report.get("backend") != "cas-lazy-unmounted" or report.get("backend_qualified") is not True:
        fail(f"{path}: portable CAS-lazy experiment was not qualified")
    if report.get("native_backend_qualified") is not False:
        fail(f"{path}: unmounted experiment must not qualify a native backend")
    if set(report.get("skipped_native_gates", [])) != NATIVE_GATES:
        fail(f"{path}: every native backend must remain explicitly skipped")

    reuse = report.get("content_reuse")
    if not isinstance(reuse, dict):
        fail(f"{path}: missing content_reuse")
    logical = require_nonnegative(reuse.get("logical_bytes"), "content_reuse.logical_bytes", path)
    authoritative = require_nonnegative(
        reuse.get("authoritative_encoded_bytes"),
        "content_reuse.authoritative_encoded_bytes",
        path,
    )
    if logical <= authoritative or reuse.get("shared_tree_roots") != 1:
        fail(f"{path}: immutable content was not reused from one authoritative tree")
    if reuse.get("lane_bindings") != lanes:
        fail(f"{path}: not every lane was bound to the shared tree")

    amplification = report.get("materialization_amplification")
    if not isinstance(amplification, dict):
        fail(f"{path}: missing materialization_amplification")
    if amplification.get("materialization_count") != 0 or amplification.get("materialized_physical_bytes") != 0:
        fail(f"{path}: CAS-lazy attachment unexpectedly materialized the artifact")
    if amplification.get("naive_per_lane_logical_bytes") != logical * lanes:
        fail(f"{path}: naive per-lane amplification baseline is incorrect")
    for field in ("copied_bytes", "projected_bytes", "prefetched_bytes"):
        if amplification.get(field) != 0:
            fail(f"{path}: {field} must be zero for the unmounted lazy experiment")

    private = report.get("private_deltas")
    if not isinstance(private, dict) or private.get("lane_count") != lanes:
        fail(f"{path}: private delta lane count is incorrect")
    if require_nonnegative(private.get("minimum_physical_bytes"), "private_deltas.minimum_physical_bytes", path) == 0:
        fail(f"{path}: lane-private deltas were not observed")
    phases = report.get("phase_latencies_ms")
    if not isinstance(phases, dict):
        fail(f"{path}: missing phase latency evidence")
    for phase in ("artifact_build", "envelope_publish", "lane_attach", "private_write", "accounting"):
        require_nonnegative(phases.get(phase), f"phase_latencies_ms.{phase}", path)
    objects = report.get("object_count")
    if not isinstance(objects, dict) or require_nonnegative(objects.get("published"), "object_count.published", path) == 0:
        fail(f"{path}: artifact object publication was not measured")


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: check-layered-lane-scale-evidence.py EVIDENCE_DIR")
    directory = Path(sys.argv[1])
    for entries, lanes in MATRIX:
        check_source(directory / f"paths-{entries}-lanes-{lanes}.json", entries, lanes)
        check_artifact(directory / f"artifacts-{entries}-lanes-{lanes}.json", entries, lanes)
    print(f"verified layered-lane scale evidence in {directory}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Run one owning-host native COW qualification and record unavailable backends."""

from __future__ import annotations

import argparse
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


BACKENDS = {
    "nfs_macos": {
        "platform": "darwin",
        "flag": "TRAIL_RUN_NFS_COW_TESTS",
        "native_test": "nfs_mount_reads_artifact_manifest_without_layer_materialization",
    },
    "fuse_linux": {
        "platform": "linux",
        "flag": "TRAIL_RUN_FUSE_COW_TESTS",
        "native_test": "fuse_mount_reads_artifact_manifest_without_layer_materialization",
    },
    "dokan_windows": {
        "platform": "win32",
        "flag": "TRAIL_RUN_DOKAN_COW_TESTS",
        "native_test": "dokan_mount_reads_artifact_manifest_without_layer_materialization",
    },
}

COMMON_CHECKS = (
    (
        "fork_inheritance",
        "integration",
        "lane_environment_inheritance",
        "lane_fork_inherits_verified_immutable_layer_with_fresh_private_uppers",
    ),
    (
        "materialization_recovery",
        "lib",
        "",
        "artifact_materialization_cache_is_tree_keyed_verified_and_copy_safe",
    ),
    (
        "promotion",
        "lib",
        "",
        "manual_private_output_promotion_is_journaled_and_preserves_private_bytes",
    ),
    (
        "source_export",
        "lib",
        "",
        "source_export_execution_checkpoints_normal_source_and_reports_git_handoff",
    ),
    (
        "retirement",
        "lib",
        "",
        "builtins_plugins_and_repository_v2_share_artifact_pipeline_conformance",
    ),
    (
        "publication_recovery",
        "lib",
        "",
        "singleflight_waiters_cancel_and_dead_owners_are_fenced",
    ),
    (
        "lane_storage_1_5_20",
        "lib",
        "",
        "artifact_accounting_does_not_multiply_shared_authority_across_1_5_20_lanes",
    ),
)


def fail(message: str) -> None:
    raise SystemExit(f"artifact native COW matrix: {message}")


def host_platform() -> str:
    if sys.platform.startswith("win"):
        return "win32"
    if sys.platform == "darwin":
        return "darwin"
    if sys.platform.startswith("linux"):
        return "linux"
    return sys.platform


def command_for(kind: str, target: str, test_name: str) -> list[str]:
    command = ["cargo", "test", "-p", "trail"]
    if kind == "lib":
        command.append("--lib")
    else:
        command.extend(("--test", target))
    command.extend((test_name, "--locked", "--", "--nocapture"))
    return command


def run_check(stage: str, command: list[str], scope: str) -> dict[str, Any]:
    started = time.monotonic()
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    elapsed_ms = int((time.monotonic() - started) * 1000)
    sys.stdout.write(result.stdout)
    passed = (
        result.returncode == 0
        and "running 1 test" in result.stdout
        and "1 passed; 0 failed" in result.stdout
    )
    return {
        "stage": stage,
        "status": "passed" if passed else "failed",
        "scope": scope,
        "command": command,
        "test_count": 1 if passed else 0,
        "duration_ms": elapsed_ms,
        "exit_code": result.returncode,
    }


def tool_output(*command: str) -> str | None:
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    if result.returncode != 0:
        return None
    value = result.stdout.strip()
    return value or None


def validate(report: dict[str, Any], owning_backend: str) -> None:
    if report.get("schema") != "trail.artifact-native-cow-matrix/v1":
        fail("internal report schema mismatch")
    backends = report.get("backends")
    if not isinstance(backends, list) or [row.get("backend") for row in backends] != list(BACKENDS):
        fail("backend rows are missing, duplicated, or unordered")
    for row in backends:
        backend = row["backend"]
        if backend == owning_backend:
            if row.get("status") != "passed" or row.get("qualification_kind") != "composed_owning_host":
                fail(f"{backend} did not produce owning-host qualification")
            checks = row.get("checks")
            expected_stages = ["immutable_lower_private_whiteout", *[check[0] for check in COMMON_CHECKS]]
            if not isinstance(checks, list) or [check.get("stage") for check in checks] != expected_stages:
                fail(f"{backend} check matrix is incomplete or unordered")
            if any(check.get("status") != "passed" or check.get("test_count") != 1 for check in checks):
                fail(f"{backend} contains a failed or skipped check")
            if checks[0].get("scope") != "native_mounted_backend":
                fail(f"{backend} lacks mounted-backend evidence")
            if any(check.get("scope") != "shared_cas_lifecycle_on_owning_host" for check in checks[1:]):
                fail(f"{backend} lifecycle evidence has an unsupported scope")
        elif row.get("status") != "unverified" or row.get("reason") != "not_owning_platform":
            fail(f"{backend} must remain explicitly unverified")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    current = host_platform()
    owning = [name for name, config in BACKENDS.items() if config["platform"] == current]
    if len(owning) != 1:
        fail(f"unsupported owning platform {current!r}")
    owning_backend = owning[0]
    config = BACKENDS[owning_backend]
    if os.environ.get(str(config["flag"])) != "1":
        fail(f"set {config['flag']}=1; refusing a native test that could silently skip")

    native_command = command_for("lib", "", str(config["native_test"]))
    checks = [
        run_check(
            "immutable_lower_private_whiteout",
            native_command,
            "native_mounted_backend",
        )
    ]
    for stage, kind, target, test_name in COMMON_CHECKS:
        checks.append(
            run_check(
                stage,
                command_for(kind, target, test_name),
                "shared_cas_lifecycle_on_owning_host",
            )
        )

    rows = []
    for backend, backend_config in BACKENDS.items():
        if backend == owning_backend:
            rows.append(
                {
                    "backend": backend,
                    "platform": backend_config["platform"],
                    "status": "passed" if all(check["status"] == "passed" for check in checks) else "failed",
                    "qualification_kind": "composed_owning_host",
                    "composition": {
                        "native_adapter_scope": "immutable lower lookup plus private copy-up and whiteout",
                        "shared_authority_scope": "inheritance, materialization, promotion, export, retirement, recovery, and accounting",
                    },
                    "checks": checks,
                }
            )
        else:
            rows.append(
                {
                    "backend": backend,
                    "platform": backend_config["platform"],
                    "status": "unverified",
                    "reason": "not_owning_platform",
                    "checks": [],
                }
            )

    report = {
        "schema": "trail.artifact-native-cow-matrix/v1",
        "host": {
            "platform": current,
            "operating_system": platform.platform(),
            "architecture": platform.machine(),
        },
        "toolchain": {
            "trail_commit": tool_output("git", "rev-parse", "HEAD"),
            "rustc": tool_output("rustc", "--version"),
        },
        "backends": rows,
    }
    validate(report, owning_backend)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"verified {owning_backend} artifact COW matrix: {args.output}")


if __name__ == "__main__":
    main()

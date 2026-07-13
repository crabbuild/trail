#!/usr/bin/env python3
import json
import pathlib
import sys


def nonnegative_json_integer(value: object, key: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{key} must be a nonnegative JSON integer")
    return value


def extract_performance_metrics(payload: object) -> dict[str, object]:
    if not isinstance(payload, dict):
        raise ValueError("agent Git apply JSON must be an object")
    export = payload.get("git_export")
    if not isinstance(export, dict):
        raise ValueError("agent Git apply JSON must contain a git_export object")
    performance = export.get("performance")
    if not isinstance(performance, dict):
        raise ValueError("agent Git apply JSON must contain git_export.performance")

    export_mode = performance.get("export_mode")
    if not isinstance(export_mode, str):
        raise ValueError("export_mode must be a string")
    changed_paths = nonnegative_json_integer(
        performance.get("changed_path_count"), "changed_path_count"
    )
    blob_writes = nonnegative_json_integer(
        performance.get("blob_write_count"), "blob_write_count"
    )
    plumbing_commands = nonnegative_json_integer(
        performance.get("git_plumbing_command_count"), "git_plumbing_command_count"
    )
    return {
        "agent_git_export_mode": export_mode,
        "agent_git_changed_paths": changed_paths,
        "agent_git_blob_writes": blob_writes,
        "agent_git_plumbing_commands": plumbing_commands,
    }


def extract_file(source: pathlib.Path, output: pathlib.Path) -> None:
    metrics = extract_performance_metrics(json.loads(source.read_text()))
    output.write_text(
        "".join(f"{key}\t{value}\n" for key, value in metrics.items())
    )


def main() -> int:
    if len(sys.argv) != 3:
        print(
            "usage: extract-agent-git-performance.py APPLY.json METRICS.tsv",
            file=sys.stderr,
        )
        return 2
    try:
        extract_file(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"invalid agent Git performance report: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

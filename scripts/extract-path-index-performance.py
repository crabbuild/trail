#!/usr/bin/env python3
import json
import pathlib
import re
import sys
import unicodedata


PREFIX = re.compile(r"^[a-z][a-z0-9_]*$")


def nonnegative_json_integer(value: object, key: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{key} must be a nonnegative JSON integer")
    return value


def folded_path_key(path: str) -> str:
    nfkc = unicodedata.normalize("NFKC", path)
    lowered = "".join(character.lower() for character in nfkc)
    return unicodedata.normalize("NFC", lowered)


def touched_folded_keys(payload: dict[str, object]) -> set[str]:
    changed_paths = payload.get("changed_paths")
    if not isinstance(changed_paths, list) or not changed_paths:
        raise ValueError("completed operation changed_paths must be a non-empty array")
    touched = set()
    for index, change in enumerate(changed_paths):
        if not isinstance(change, dict):
            raise ValueError(f"changed_paths[{index}] must be an object")
        path = change.get("path")
        if not isinstance(path, str) or not path:
            raise ValueError(f"changed_paths[{index}].path must be a string")
        touched.add(folded_path_key(path))
        old_path = change.get("old_path")
        if old_path is not None:
            if not isinstance(old_path, str) or not old_path:
                raise ValueError(f"changed_paths[{index}].old_path must be a string or null")
            touched.add(folded_path_key(old_path))
    return touched


def extract_path_index_metrics(payload: object, prefix: str) -> dict[str, object]:
    if not PREFIX.fullmatch(prefix):
        raise ValueError("metric prefix must use lowercase letters, digits, and underscores")
    if not isinstance(payload, dict):
        raise ValueError("patch or record JSON must be an object")
    operation = payload.get("operation")
    if not isinstance(operation, str) or not operation:
        raise ValueError("path-index metrics require one completed operation")
    path_index = payload.get("path_index")
    if not isinstance(path_index, dict):
        raise ValueError("operation JSON must contain a path_index object")

    mode = path_index.get("mode")
    if not isinstance(mode, str) or not mode:
        raise ValueError("mode must be a string")
    lookup_count = nonnegative_json_integer(
        path_index.get("lookup_count"), "lookup_count"
    )
    full_root_loads = nonnegative_json_integer(
        path_index.get("full_root_path_load_count"), "full_root_path_load_count"
    )
    full_filesystem_scans = nonnegative_json_integer(
        path_index.get("full_filesystem_path_scan_count"),
        "full_filesystem_path_scan_count",
    )
    touched_count = len(touched_folded_keys(payload))
    if lookup_count > touched_count:
        raise ValueError(
            f"lookup_count {lookup_count} exceeds {touched_count} unique folded touched keys"
        )

    return {
        f"{prefix}_path_index_mode": mode,
        f"{prefix}_path_index_lookup_count": lookup_count,
        f"{prefix}_path_index_full_root_path_load_count": full_root_loads,
        f"{prefix}_path_index_full_filesystem_path_scan_count": full_filesystem_scans,
        f"{prefix}_path_index_touched_folded_key_count": touched_count,
    }


def extract_file(source: pathlib.Path, prefix: str, output: pathlib.Path) -> None:
    metrics = extract_path_index_metrics(json.loads(source.read_text()), prefix)
    output.write_text("".join(f"{key}\t{value}\n" for key, value in metrics.items()))


def main() -> int:
    if len(sys.argv) != 4:
        print(
            "usage: extract-path-index-performance.py REPORT.json PREFIX METRICS.tsv",
            file=sys.stderr,
        )
        return 2
    try:
        extract_file(pathlib.Path(sys.argv[1]), sys.argv[2], pathlib.Path(sys.argv[3]))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"invalid path-index performance report: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

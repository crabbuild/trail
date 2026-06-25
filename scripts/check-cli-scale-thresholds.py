#!/usr/bin/env python3
import csv
import pathlib
import sys


def main() -> int:
    if len(sys.argv) < 3:
        print(
            "usage: check-cli-scale-thresholds.py RESULTS.tsv name=max_seconds ...",
            file=sys.stderr,
        )
        return 2

    results_path = pathlib.Path(sys.argv[1])
    thresholds = {}
    for spec in sys.argv[2:]:
        if "=" not in spec:
            print(f"invalid threshold `{spec}`; expected name=max_seconds", file=sys.stderr)
            return 2
        name, value = spec.split("=", 1)
        try:
            thresholds[name] = float(value)
        except ValueError:
            print(f"invalid threshold seconds for `{name}`: {value}", file=sys.stderr)
            return 2

    with results_path.open(newline="") as handle:
        rows = {
            row["name"]: row
            for row in csv.DictReader(handle, delimiter="\t")
            if row.get("name")
        }

    failures = []
    for name, max_seconds in thresholds.items():
        row = rows.get(name)
        if row is None:
            failures.append(f"{name}: missing from {results_path}")
            continue
        exit_code = int(row["exit_code"])
        seconds = float(row["real_seconds"])
        if exit_code != 0:
            failures.append(f"{name}: exit_code={exit_code}")
        if seconds > max_seconds:
            failures.append(f"{name}: {seconds:.2f}s > {max_seconds:.2f}s")

    if failures:
        print("CLI scale threshold failures:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print(f"checked {len(thresholds)} CLI scale thresholds in {results_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

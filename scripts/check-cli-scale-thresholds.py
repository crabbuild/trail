#!/usr/bin/env python3
import csv
import pathlib
import sys


def main() -> int:
    if len(sys.argv) < 3:
        print(
            "usage: check-cli-scale-thresholds.py RESULTS.tsv name=max_seconds ... "
            "[--metrics METRICS.tsv key=max_value ...] "
            "[--metric-equals key=expected_value ...]",
            file=sys.stderr,
        )
        return 2

    results_path = pathlib.Path(sys.argv[1])
    args = sys.argv[2:]
    metrics_path = None
    metric_specs = []
    metric_equality_specs = []
    if "--metrics" in args:
        marker = args.index("--metrics")
        if marker + 1 >= len(args):
            print("missing METRICS.tsv after --metrics", file=sys.stderr)
            return 2
        metrics_path = pathlib.Path(args[marker + 1])
        metric_args = args[marker + 2 :]
        args = args[:marker]
        if "--metric-equals" in metric_args:
            equals_marker = metric_args.index("--metric-equals")
            metric_specs = metric_args[:equals_marker]
            metric_equality_specs = metric_args[equals_marker + 1 :]
        else:
            metric_specs = metric_args
    elif "--metric-equals" in args:
        print("--metric-equals requires --metrics METRICS.tsv", file=sys.stderr)
        return 2

    thresholds = {}
    for spec in args:
        if "=" not in spec:
            print(f"invalid threshold `{spec}`; expected name=max_seconds", file=sys.stderr)
            return 2
        name, value = spec.split("=", 1)
        try:
            thresholds[name] = float(value)
        except ValueError:
            print(f"invalid threshold seconds for `{name}`: {value}", file=sys.stderr)
            return 2

    metric_thresholds = {}
    for spec in metric_specs:
        if "=" not in spec:
            print(f"invalid metric threshold `{spec}`; expected key=max_value", file=sys.stderr)
            return 2
        key, value = spec.split("=", 1)
        try:
            metric_thresholds[key] = float(value)
        except ValueError:
            print(f"invalid metric threshold value for `{key}`: {value}", file=sys.stderr)
            return 2

    metric_equalities = {}
    for spec in metric_equality_specs:
        if "=" not in spec:
            print(
                f"invalid metric equality `{spec}`; expected key=expected_value",
                file=sys.stderr,
            )
            return 2
        key, value = spec.split("=", 1)
        if not key or not value:
            print(
                f"invalid metric equality `{spec}`; expected key=expected_value",
                file=sys.stderr,
            )
            return 2
        metric_equalities[key] = value

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

    if metrics_path is not None:
        metrics = read_metric_values(metrics_path)
        missing_hint = format_available_metrics(metrics)
        for key, max_value in metric_thresholds.items():
            value = metrics.get(key)
            if value is None:
                failures.append(f"{key}: missing from {metrics_path}{missing_hint}")
                continue
            if not isinstance(value, (int, float)):
                failures.append(f"{key}: expected numeric value, found {value!r}")
                continue
            if value > max_value:
                failures.append(f"{key}: {value:.0f} > {max_value:.0f}")
        for key, expected in metric_equalities.items():
            value = metrics.get(key)
            if value is None:
                failures.append(f"{key}: missing from {metrics_path}{missing_hint}")
                continue
            equal = str(value) == expected
            if isinstance(value, (int, float)):
                try:
                    equal = value == float(expected)
                except ValueError:
                    equal = False
            if not equal:
                failures.append(f"{key}: {value!r} != {expected!r}")

    if failures:
        print("CLI scale threshold failures:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    checked = len(thresholds) + len(metric_thresholds) + len(metric_equalities)
    print(f"checked {checked} CLI scale thresholds")
    return 0


def read_metric_values(metrics_path: pathlib.Path) -> dict[str, object]:
    metrics = {}
    with metrics_path.open(newline="") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if not line:
                continue
            parts = line.split("\t")
            if len(parts) != 2:
                continue
            key, value = parts
            key = key.strip()
            value = value.strip()
            try:
                metrics[key] = float(value)
            except ValueError:
                metrics[key] = value
    add_derived_file_metrics(metrics_path, metrics)
    return metrics


def read_metrics(metrics_path: pathlib.Path) -> dict[str, object]:
    """Backward-compatible alias for callers importing the old helper name."""
    return read_metric_values(metrics_path)


def add_derived_file_metrics(metrics_path: pathlib.Path, metrics: dict[str, object]) -> None:
    root = metrics_path.parent
    add_file_size_metric(
        metrics,
        "sqlite_bytes",
        root / "repo" / ".trail" / "index" / "trail.sqlite",
    )
    add_file_size_metric(
        metrics,
        "git_sqlite_bytes",
        root / "git-repo" / ".trail" / "index" / "trail.sqlite",
    )


def add_file_size_metric(
    metrics: dict[str, object],
    key: str,
    path: pathlib.Path,
) -> None:
    if key not in metrics and path.is_file():
        metrics[key] = float(path.stat().st_size)


def format_available_metrics(metrics: dict[str, object]) -> str:
    if not metrics:
        return "; no valid metrics were read"
    keys = sorted(metrics)
    preview = ", ".join(keys[:12])
    if len(keys) > 12:
        preview += f", ... ({len(keys)} total)"
    return f"; available metrics: {preview}"


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Run one command with bounded evidence and whole-process-tree supervision."""

from __future__ import annotations

import argparse
import json
import os
import selectors
import signal
import subprocess
import sys
import time
from pathlib import Path


def process_tree_rss(root_pid: int) -> int:
    try:
        output = subprocess.check_output(
            ["ps", "-axo", "pid=,ppid=,rss="], text=True, stderr=subprocess.DEVNULL
        )
    except (OSError, subprocess.CalledProcessError):
        return 0
    rows: dict[int, tuple[int, int]] = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) == 3 and all(field.isdigit() for field in fields):
            rows[int(fields[0])] = (int(fields[1]), int(fields[2]) * 1024)
    descendants = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, (parent, _) in rows.items():
            if parent in descendants and pid not in descendants:
                descendants.add(pid)
                changed = True
    return sum(rows.get(pid, (0, 0))[1] for pid in descendants)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument("--max-output-bytes", type=int, required=True)
    parser.add_argument("--stdout", type=Path, required=True)
    parser.add_argument("--stderr", type=Path, required=True)
    parser.add_argument("--rss", type=Path, required=True)
    parser.add_argument("--meta", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if args.timeout_seconds <= 0 or args.max_output_bytes <= 0 or not command:
        parser.error("positive bounds and a command are required")

    interrupted: list[int] = []

    def on_signal(signum: int, _frame: object) -> None:
        interrupted.append(signum)

    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, on_signal)

    started = time.monotonic()
    child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             start_new_session=True, bufsize=0)
    assert child.stdout is not None and child.stderr is not None
    selector = selectors.DefaultSelector()
    selector.register(child.stdout, selectors.EVENT_READ, "stdout")
    selector.register(child.stderr, selectors.EVENT_READ, "stderr")
    streams = {"stdout": args.stdout.open("wb"), "stderr": args.stderr.open("wb")}
    sizes = {"stdout": 0, "stderr": 0}
    truncated = {"stdout": False, "stderr": False}
    peak = 1
    timed_out = False
    termination_started: float | None = None
    next_rss_sample = started
    try:
        while selector.get_map() or child.poll() is None:
            now = time.monotonic()
            if now >= next_rss_sample:
                peak = max(peak, process_tree_rss(child.pid))
                next_rss_sample = now + 0.1
            if termination_started is None and (interrupted or now - started >= args.timeout_seconds):
                timed_out = not interrupted
                termination_started = now
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            elif termination_started is not None and now - termination_started >= 1.0 and child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            for key, _ in selector.select(0.02):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                name = key.data
                available = max(0, args.max_output_bytes - sizes[name])
                if available:
                    streams[name].write(chunk[:available])
                    sizes[name] += min(len(chunk), available)
                if len(chunk) > available:
                    truncated[name] = True
        child.wait()
    finally:
        for stream in streams.values():
            stream.flush()
            os.fsync(stream.fileno())
            stream.close()
        args.rss.write_text(f"{peak}\n", encoding="ascii")
        elapsed = time.monotonic() - started
        args.meta.write_text(json.dumps({
            "schema_version": 1,
            "elapsed_seconds": elapsed,
            "peak_process_tree_rss_bytes": peak,
            "timed_out": timed_out,
            "interrupted_signal": interrupted[0] if interrupted else None,
            "stdout_truncated": truncated["stdout"],
            "stderr_truncated": truncated["stderr"],
            "max_output_bytes_per_stream": args.max_output_bytes,
        }, sort_keys=True) + "\n", encoding="utf-8")
    if timed_out:
        return 124
    if interrupted:
        return 128 + interrupted[0]
    return child.returncode


if __name__ == "__main__":
    raise SystemExit(main())

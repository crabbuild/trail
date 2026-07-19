#!/usr/bin/env python3
"""Contracts for the bounded scale-harness command supervisor."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


RUNNER = Path(__file__).with_name("scale-command-runner.py")


class ScaleCommandRunnerTests(unittest.TestCase):
    def test_timeout_bounds_output_and_kills_the_complete_process_group(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            child_pid = root / "child.pid"
            program = (
                "import os,pathlib,subprocess,sys,time; "
                f"p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); pathlib.Path({str(child_pid)!r}).write_text(str(p.pid)); "
                "sys.stdout.write('x'*200000); sys.stdout.flush(); time.sleep(30)"
            )
            command = [sys.executable, str(RUNNER), "--timeout-seconds", "0.25",
                       "--max-output-bytes", "4096", "--stdout", str(root / "stdout"),
                       "--stderr", str(root / "stderr"), "--rss", str(root / "rss"),
                       "--meta", str(root / "meta.json"), "--", sys.executable, "-c", program]
            result = subprocess.run(command, text=True, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
            self.assertLessEqual((root / "stdout").stat().st_size, 4096)
            meta = json.loads((root / "meta.json").read_text())
            self.assertTrue(meta["timed_out"])
            self.assertTrue(meta["stdout_truncated"])
            self.assertGreater(meta["peak_process_tree_rss_bytes"], 0)
            pid = int(child_pid.read_text())
            probe = subprocess.run(["ps", "-p", str(pid), "-o", "stat="], text=True, capture_output=True)
            self.assertTrue(probe.returncode != 0 or "Z" in probe.stdout, probe.stdout)


if __name__ == "__main__":
    unittest.main()

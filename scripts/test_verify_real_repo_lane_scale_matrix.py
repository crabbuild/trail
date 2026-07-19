#!/usr/bin/env python3
"""Contract tests for the disposable 64/128 real-repository scale matrix."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
ORCHESTRATOR = SCRIPT_DIR / "verify-real-repo-lane-scale-matrix.sh"
TEMP_BASE = Path("/Volumes/Workspace")


FAKE_TRAIL = r'''#!/usr/bin/env python3
import json, os, pathlib, sys

args = sys.argv[1:]
if args == ["--version"]:
    print("trail 0.0.0-disposable-fake")
    raise SystemExit(0)
if args == ["init", "--help"]:
    print("usage: trail init [--from-git]")
    raise SystemExit(0)
if len(args) == 5 and args[0] == "--workspace" and args[2:] == ["--json", "init", "--from-git"]:
    workspace = pathlib.Path(args[1])
    trail = workspace / ".trail"
    if trail.exists() or trail.is_symlink():
        print("copied .trail was not removed", file=sys.stderr)
        raise SystemExit(17)
    trail.mkdir()
    (trail / "initialized-from-git").write_text("fresh\n", encoding="utf-8")
    log = pathlib.Path(os.environ["FAKE_TRAIL_INIT_LOG"])
    with log.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps({"workspace": str(workspace.resolve())}, sort_keys=True) + "\n")
    print(json.dumps({"branch": "main", "import_mode": "git_tracked"}, sort_keys=True))
    raise SystemExit(0)
print("unsupported fake Trail invocation: " + repr(args), file=sys.stderr)
raise SystemExit(64)
'''


FAKE_INNER = r'''#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys

repo = pathlib.Path(os.environ["TRAIL_SCALE_REPO"]).resolve()
output = pathlib.Path(os.environ["TRAIL_SCALE_OUTPUT"])
run_id = os.environ["TRAIL_SCALE_RUN_ID"]
lanes = int(os.environ["TRAIL_SCALE_LANES"])
ref = os.environ["TRAIL_SCALE_GIT_REF"]
owner_path = pathlib.Path(os.environ["TRAIL_SCALE_DISPOSABLE_OWNER_FILE"])
if os.environ.get("TRAIL_SCALE_DISPOSABLE_WORKSPACE") != "1":
    raise SystemExit("missing disposable marker")
if not owner_path.is_file() or owner_path.is_symlink() or owner_path.parent != repo / ".trail":
    raise SystemExit("unsafe disposable owner file")
owner = json.loads(owner_path.read_text(encoding="utf-8"))
expected = {
    "schema_version": 1,
    "kind": "trail_scale_disposable_workspace",
    "canonical_repo": os.environ["FAKE_CANONICAL_REPO"],
    "disposable_repo": str(repo),
    "output": os.path.abspath(os.path.normpath(str(output))),
    "run_id": run_id,
}
if owner != expected:
    raise SystemExit("owner binding mismatch: " + repr(owner))

output.mkdir(parents=True, exist_ok=False)
call = {
    "lanes": lanes,
    "repo": str(repo),
    "trail_dir": str((repo / ".trail").resolve()),
    "output": str(output),
    "run_id": run_id,
    "ref": ref,
    "owner": owner,
}
with open(os.environ["FAKE_INNER_LOG"], "a", encoding="utf-8") as stream:
    stream.write(json.dumps(call, sort_keys=True) + "\n")
if os.environ.get("FAKE_INNER_FAIL_LANES") == str(lanes):
    (output / "retained-failure.txt").write_text("injected\n", encoding="utf-8")
    print("injected inner failure", file=sys.stderr)
    raise SystemExit(23)

def git(*args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()

parent = git("rev-parse", "HEAD")
tree = git("rev-parse", "HEAD^{tree}")
commit = subprocess.check_output(
    ["git", "-C", str(repo), "commit-tree", tree, "-p", parent, "-m", f"fake scale {lanes}"],
    text=True,
).strip()
subprocess.run(["git", "-C", str(repo), "update-ref", ref, commit, ""], check=True)
(output / "checker.out").write_text(
    json.dumps({"status": "PASS", "lanes": lanes, "commands": 1, "faults": 0}, sort_keys=True) + "\n",
    encoding="utf-8",
)
(output / "inner-evidence.json").write_text(json.dumps({"commit": commit}, sort_keys=True) + "\n")
print("fake inner PASS")
'''


GIT_COLLISION_WRAPPER = r'''#!/usr/bin/env python3
import os, pathlib, subprocess, sys

args = sys.argv[1:]
real = os.environ["FAKE_REAL_GIT"]
source = os.environ.get("FAKE_CAS_SOURCE")
marker = pathlib.Path(os.environ["FAKE_CAS_MARKER"]) if os.environ.get("FAKE_CAS_MARKER") else None
if source and marker and "fetch" in args and not marker.exists():
    ref = os.environ["FAKE_CAS_REF"]
    subprocess.run([real, "-C", source, "update-ref", ref, "HEAD", ""], check=True)
    marker.write_text("created\n", encoding="utf-8")
os.execv(real, [real, *args])
'''


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def worktree_inventory(root: Path) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []

    def visit(directory: Path) -> None:
        for path in sorted(directory.iterdir(), key=lambda item: os.fsencode(item.name)):
            relative = path.relative_to(root).as_posix()
            if relative == ".git" or relative.startswith(".git/"):
                continue
            if relative == ".trail" or relative.startswith(".trail/"):
                continue
            metadata = path.lstat()
            mode = stat.S_IMODE(metadata.st_mode)
            if stat.S_ISLNK(metadata.st_mode):
                rows.append({"path": relative, "type": "symlink", "mode": mode,
                             "digest": sha256(os.fsencode(os.readlink(path)))})
            elif stat.S_ISDIR(metadata.st_mode):
                rows.append({"path": relative, "type": "directory", "mode": mode, "digest": None})
                visit(path)
            elif stat.S_ISREG(metadata.st_mode):
                rows.append({"path": relative, "type": "regular", "mode": mode,
                             "digest": sha256(path.read_bytes())})
            else:
                rows.append({"path": relative, "type": "other", "mode": mode, "digest": None})

    visit(root)
    return rows


class DisposableScaleMatrixTests(unittest.TestCase):
    def setUp(self) -> None:
        if not TEMP_BASE.is_dir() or platform_filesystem(TEMP_BASE) != "apfs":
            self.skipTest("APFS /Volumes/Workspace is required")
        self.temp = tempfile.TemporaryDirectory(dir=TEMP_BASE)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.output = self.root / "matrix-output"
        self.fake_trail = self.root / "fake-trail"
        self.fake_inner = self.root / "fake-inner"
        self.inner_log = self.root / "inner.jsonl"
        self.init_log = self.root / "init.jsonl"
        self.source.mkdir()
        subprocess.run(["git", "init", "-q", "-b", "main", str(self.source)], check=True)
        self.git("config", "user.email", "trail@example.com")
        self.git("config", "user.name", "Trail")
        (self.source / ".gitignore").write_text(".trail/\nignored.txt\nignored-dir/\n", encoding="utf-8")
        (self.source / "README.md").write_text("tracked\n", encoding="utf-8")
        (self.source / "tracked-dir").mkdir()
        (self.source / "tracked-dir/data.txt").write_text("tracked data\n", encoding="utf-8")
        os.symlink("README.md", self.source / "tracked-link")
        self.git("add", ".")
        self.git("commit", "-q", "-m", "baseline")
        (self.source / "untracked.txt").write_text("untracked\n", encoding="utf-8")
        (self.source / "untracked-dir").mkdir()
        (self.source / "untracked-dir/data.bin").write_bytes(b"\x00untracked\n")
        (self.source / "ignored.txt").write_text("ignored\n", encoding="utf-8")
        (self.source / "ignored-dir").mkdir()
        (self.source / "ignored-dir/data.txt").write_text("ignored directory\n", encoding="utf-8")
        (self.source / ".trail").mkdir()
        (self.source / ".trail/stale-source-state").write_text("remove only in copies\n", encoding="utf-8")
        for path, source in ((self.fake_trail, FAKE_TRAIL), (self.fake_inner, FAKE_INNER)):
            path.write_text(source, encoding="utf-8")
            path.chmod(0o755)

    def git(self, *args: str) -> str:
        return subprocess.check_output(["git", "-C", str(self.source), *args], text=True).strip()

    def source_state(self) -> dict[str, object]:
        env = dict(os.environ, GIT_OPTIONAL_LOCKS="0")
        refs = subprocess.check_output(
            ["git", "-C", str(self.source), "for-each-ref", "--format=%(refname)%00%(objectname)"],
            env=env,
        ).splitlines()
        status_bytes = subprocess.check_output(
            ["git", "-C", str(self.source), "status", "--porcelain=v2", "-z",
             "--untracked-files=all", "--ignored=matching"],
            env=env,
        )
        index = self.source / ".git/index"
        return {
            "head": self.git("rev-parse", "HEAD"),
            "symbolic_head": self.git("symbolic-ref", "HEAD"),
            "refs": [line.decode("utf-8") for line in refs],
            "status": status_bytes.hex(),
            "index_mode": stat.S_IMODE(index.lstat().st_mode),
            "index_digest": sha256(index.read_bytes()),
            "worktree": worktree_inventory(self.source),
        }

    def run_matrix(self, *, publish: str = "0", extra_env: dict[str, str] | None = None,
                   output: Path | None = None) -> subprocess.CompletedProcess[str]:
        self.assertTrue(ORCHESTRATOR.is_file(), f"missing orchestrator: {ORCHESTRATOR}")
        env = dict(
            os.environ,
            TRAIL_BIN=str(self.fake_trail),
            TRAIL_SCALE_REPO=str(self.source),
            TRAIL_SCALE_MATRIX_OUTPUT=str(output or self.output),
            TRAIL_SCALE_INNER_HARNESS=str(self.fake_inner),
            TRAIL_SCALE_MATRIX_RUN_ID="contract",
            TRAIL_SCALE_MATRIX_PUBLISH=publish,
            TRAIL_SCALE_FILES_PER_LANE="1",
            TRAIL_SCALE_FAULT_PHASE="all",
            FAKE_INNER_LOG=str(self.inner_log),
            FAKE_TRAIL_INIT_LOG=str(self.init_log),
            FAKE_CANONICAL_REPO=str(self.source.resolve()),
        )
        if extra_env:
            env.update(extra_env)
        return subprocess.run([str(ORCHESTRATOR)], env=env, text=True, capture_output=True)

    def calls(self) -> list[dict[str, object]]:
        if not self.inner_log.exists():
            return []
        return [json.loads(line) for line in self.inner_log.read_text(encoding="utf-8").splitlines()]

    def test_two_disposable_runs_preserve_source_and_copy_every_worktree_kind(self) -> None:
        baseline = self.source_state()
        result = self.run_matrix()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual([call["lanes"] for call in calls], [64, 128])
        self.assertEqual(len({call["repo"] for call in calls}), 2)
        self.assertEqual(len({call["trail_dir"] for call in calls}), 2)
        self.assertEqual(len({call["run_id"] for call in calls}), 2)
        self.assertEqual(len({call["ref"] for call in calls}), 2)
        for call in calls:
            copy = Path(str(call["repo"]))
            self.assertEqual(worktree_inventory(copy), baseline["worktree"])
            self.assertFalse((copy / ".trail/stale-source-state").exists())
            self.assertTrue((copy / ".trail/initialized-from-git").is_file())
            self.assertTrue((copy / ".trail/scale-disposable-owner.json").is_file())
        self.assertEqual(self.source_state(), baseline)
        for count in (64, 128):
            proof = json.loads((self.output / f"runs/{count}/proof.json").read_text(encoding="utf-8"))
            self.assertEqual(proof["lanes"], count)
            self.assertEqual(proof["tree"], self.git("rev-parse", "HEAD^{tree}"))
            self.assertTrue(Path(proof["bundle"]).is_file())
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-64"))
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-128"))

    def test_inner_failure_retains_copy_and_evidence_without_source_mutation(self) -> None:
        baseline = self.source_state()
        result = self.run_matrix(extra_env={"FAKE_INNER_FAIL_LANES": "64"})
        self.assertNotEqual(result.returncode, 0)
        calls = self.calls()
        self.assertEqual([call["lanes"] for call in calls], [64])
        self.assertTrue(Path(str(calls[0]["repo"])).is_dir())
        self.assertTrue((Path(str(calls[0]["output"])) / "retained-failure.txt").is_file())
        self.assertEqual(self.source_state(), baseline)
        self.assertFalse((self.output / "runs/128").exists())

    def test_publication_creates_only_two_refs_without_moving_source_checkout(self) -> None:
        baseline = self.source_state()
        result = self.run_matrix(publish="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        final = self.source_state()
        self.assertEqual(final["head"], baseline["head"])
        self.assertEqual(final["symbolic_head"], baseline["symbolic_head"])
        self.assertEqual(final["status"], baseline["status"])
        self.assertEqual(final["index_digest"], baseline["index_digest"])
        self.assertEqual(final["worktree"], baseline["worktree"])
        original_refs = set(baseline["refs"])
        added_refs = set(final["refs"]) - original_refs
        self.assertEqual(len(added_refs), 2)
        for count in (64, 128):
            proof = json.loads((self.output / f"runs/{count}/proof.json").read_text(encoding="utf-8"))
            self.assertEqual(self.git("rev-parse", proof["ref"]), proof["commit"])

    def test_publication_transaction_refuses_a_racing_ref_without_partial_publish(self) -> None:
        baseline = self.source_state()
        wrapper_dir = self.root / "fake-git-bin"
        wrapper_dir.mkdir()
        wrapper = wrapper_dir / "git"
        wrapper.write_text(GIT_COLLISION_WRAPPER, encoding="utf-8")
        wrapper.chmod(0o755)
        collision_ref = "refs/heads/codex/trail-scale-contract-128"
        marker = self.root / "collision-created"
        real_git = shutil.which("git")
        assert real_git is not None
        result = self.run_matrix(
            publish="1",
            extra_env={
                "PATH": str(wrapper_dir) + os.pathsep + os.environ["PATH"],
                "FAKE_REAL_GIT": real_git,
                "FAKE_CAS_SOURCE": str(self.source),
                "FAKE_CAS_MARKER": str(marker),
                "FAKE_CAS_REF": collision_ref,
            },
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(marker.is_file())
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-64"))
        self.assertEqual(self.git("rev-parse", collision_ref), baseline["head"])
        final = self.source_state()
        self.assertEqual(final["head"], baseline["head"])
        self.assertEqual(final["symbolic_head"], baseline["symbolic_head"])
        self.assertEqual(final["status"], baseline["status"])
        self.assertEqual(final["index_digest"], baseline["index_digest"])
        self.assertEqual(final["worktree"], baseline["worktree"])

    def test_relative_source_is_rejected_before_any_copy_or_inner_run(self) -> None:
        result = self.run_matrix(extra_env={"TRAIL_SCALE_REPO": "relative/source"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("TRAIL_SCALE_REPO must be absolute", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse(self.output.exists())

    def git_ref_exists(self, ref: str) -> bool:
        return subprocess.run(
            ["git", "-C", str(self.source), "show-ref", "--verify", "--quiet", ref]
        ).returncode == 0


def platform_filesystem(path: Path) -> str:
    if sys_platform() == "Darwin":
        output = subprocess.check_output(["diskutil", "info", str(path)], text=True)
        for line in output.splitlines():
            if "File System Personality:" in line:
                return line.split(":", 1)[1].strip().lower()
    return "unknown"


def sys_platform() -> str:
    return subprocess.check_output(["uname", "-s"], text=True).strip()


if __name__ == "__main__":
    unittest.main()

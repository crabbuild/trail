#!/usr/bin/env python3
"""Contract tests for the disposable 64/128 real-repository scale matrix."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import signal
import stat
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
ORCHESTRATOR = SCRIPT_DIR / "verify-real-repo-lane-scale-matrix.sh"
CLONE_HELPER = SCRIPT_DIR / "apfs-clone-tree.py"
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
    if os.environ.get("FAKE_TRAIL_TAMPER_PIN_AFTER_INIT") == "1":
        executable = pathlib.Path(sys.argv[0])
        executable.chmod(0o755)
        with executable.open("a", encoding="utf-8") as stream:
            stream.write("# tampered pinned binary\n")
    print(json.dumps({"branch": "main", "import_mode": "git_tracked"}, sort_keys=True))
    raise SystemExit(0)
print("unsupported fake Trail invocation: " + repr(args), file=sys.stderr)
raise SystemExit(64)
'''


FAKE_INNER = r'''#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys, time

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
if os.environ.get("FAKE_INNER_TAMPER_OWNER_LANES") == str(lanes):
    owner_path.write_text("{}\n", encoding="utf-8")

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
if os.environ.get("FAKE_INNER_MUTATE_SOURCE_TRAIL_LANES") == str(lanes):
    canonical = pathlib.Path(os.environ["FAKE_CANONICAL_REPO"])
    (canonical / ".trail/misrouted-inner-state").write_text("mutated\n", encoding="utf-8")
if os.environ.get("FAKE_INNER_BLOCK_LANES") == str(lanes):
    pathlib.Path(os.environ["FAKE_INNER_PID_FILE"]).write_text(
        json.dumps({"pid": os.getpid(), "pgid": os.getpgrp()}) + "\n", encoding="utf-8"
    )
    while True:
        time.sleep(1)

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
if os.environ.get("FAKE_INNER_TAMPER_PRIOR_LANES") == str(lanes):
    kind = os.environ["FAKE_INNER_TAMPER_PRIOR_KIND"]
    prior_run = output.parents[1] / "64"
    if kind == "checker":
        (prior_run / "evidence/checker.out").write_text("tampered after proof\n", encoding="utf-8")
    elif kind == "bundle":
        with (prior_run / "final.bundle").open("ab") as stream:
            stream.write(b"tampered")
    elif kind == "proof":
        proof_path = prior_run / "proof.json"
        proof = json.loads(proof_path.read_text(encoding="utf-8"))
        proof["lanes"] = 999
        proof_path.write_text(json.dumps(proof, sort_keys=True) + "\n", encoding="utf-8")
    else:
        raise SystemExit("unsupported tamper kind")
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


GIT_SHOW_REF_ERROR_WRAPPER = r'''#!/usr/bin/env python3
import os, sys
args = sys.argv[1:]
if "show-ref" in args:
    print("injected show-ref operational failure", file=sys.stderr)
    raise SystemExit(2)
real = os.environ["FAKE_REAL_GIT"]
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
        os.link(self.source / "README.md", self.source / "tracked-hardlink")
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
        os.chmod(self.source / "tracked-dir", 0o751)
        os.utime(self.source / "tracked-dir", ns=(1_700_000_000_000_000_000,
                                                   1_700_000_000_000_000_000))
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

    def matrix_env(self, *, publish: str = "0", extra_env: dict[str, str] | None = None,
                   output: Path | None = None) -> dict[str, str]:
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
        return env

    def run_matrix(self, *, publish: str = "0", extra_env: dict[str, str] | None = None,
                   output: Path | None = None) -> subprocess.CompletedProcess[str]:
        self.assertTrue(ORCHESTRATOR.is_file(), f"missing orchestrator: {ORCHESTRATOR}")
        env = self.matrix_env(publish=publish, extra_env=extra_env, output=output)
        return subprocess.run(["bash", str(ORCHESTRATOR)], env=env, text=True, capture_output=True)

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
            self.assertEqual(
                (copy / "README.md").stat().st_ino,
                (copy / "tracked-hardlink").stat().st_ino,
            )
            self.assertEqual(stat.S_IMODE((copy / "tracked-dir").stat().st_mode), 0o751)
            self.assertEqual((copy / "tracked-dir").stat().st_mtime_ns,
                             (self.source / "tracked-dir").stat().st_mtime_ns)
            self.assertFalse((copy / ".trail/stale-source-state").exists())
            self.assertTrue((copy / ".trail/initialized-from-git").is_file())
            self.assertTrue((copy / ".trail/scale-disposable-owner.json").is_file())
        self.assertEqual(self.source_state(), baseline)
        for count in (64, 128):
            proof = json.loads((self.output / f"runs/{count}/proof.json").read_text(encoding="utf-8"))
            self.assertEqual(proof["lanes"], count)
            self.assertEqual(proof["tree"], self.git("rev-parse", "HEAD^{tree}"))
            self.assertTrue(Path(proof["bundle"]).is_file())
            manifest = json.loads(
                (self.output / f"runs/{count}/clone-manifest.json").read_text(encoding="utf-8")
            )
            self.assertEqual(manifest["status"], "PASS")
            self.assertEqual(manifest["clone_api"], "clonefile(2)")
            self.assertFalse(manifest["byte_copy_fallback"])
            counters = manifest["counters"]
            self.assertEqual(counters["clonefile_calls_attempted"],
                             counters["clonefile_calls_succeeded"])
            self.assertEqual(counters["regular_paths"],
                             counters["clonefile_calls_succeeded"] + counters["hardlinks_created"])
            self.assertEqual(manifest["source_tree_sha256"], manifest["destination_tree_sha256"])
            self.assertEqual(manifest["source_inventory_sha256"],
                             manifest["destination_inventory_sha256"])
            self.assertEqual(manifest["source_device"], manifest["destination_device"])
            for clone_call in manifest["clonefile_calls"]:
                self.assertTrue(clone_call["success"])
                self.assertEqual(clone_call["source_device"], clone_call["destination_device"])
                self.assertEqual(clone_call["size"], clone_call["destination_size"])
        self.assertEqual(stat.S_IMODE((self.output / "pinned").stat().st_mode), 0o555)
        self.assertEqual(stat.S_IMODE((self.output / "pinned/trail").stat().st_mode), 0o555)
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

    def test_clonefile_failure_aborts_without_byte_copy_fallback(self) -> None:
        interposer = self.build_clonefile_failure_interposer()
        destination = self.root / "failed-clone"
        destination.mkdir()
        manifest_path = self.root / "failed-clone-manifest.json"
        env = dict(os.environ, DYLD_INSERT_LIBRARIES=str(interposer))
        result = subprocess.run(
            ["python3", str(CLONE_HELPER), str(self.source), str(destination), str(manifest_path)],
            env=env, text=True, capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])
        self.assertTrue(manifest_path.is_file(), result.stderr)
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual(manifest["status"], "FAIL")
        self.assertFalse(manifest["byte_copy_fallback"])
        self.assertEqual(manifest["counters"]["clonefile_calls_attempted"], 1)
        self.assertEqual(manifest["counters"]["clonefile_calls_succeeded"], 0)
        self.assertFalse((destination / "README.md").exists())

    def test_clonefile_errno_and_single_file_failures_are_retained(self) -> None:
        interposer = self.build_clonefile_failure_interposer()
        for error_number in (5, 18, 45):  # EIO, EXDEV, ENOTSUP on Darwin
            with self.subTest(errno=error_number):
                source = self.root / f"errno-source-{error_number}"
                destination = self.root / f"errno-destination-{error_number}"
                manifest_path = self.root / f"errno-manifest-{error_number}.json"
                source.mkdir(); destination.mkdir()
                (source / "only.txt").write_text("clone me\n", encoding="utf-8")
                env = dict(os.environ, DYLD_INSERT_LIBRARIES=str(interposer),
                           FAKE_CLONEFILE_ERRNO=str(error_number))
                result = subprocess.run(
                    ["python3", str(CLONE_HELPER), str(source), str(destination), str(manifest_path)],
                    env=env, text=True, capture_output=True,
                )
                self.assertNotEqual(result.returncode, 0)
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
                self.assertEqual(manifest["failure"]["errno"], error_number)
                self.assertEqual(manifest["counters"]["clonefile_calls_attempted"], 1)
                self.assertEqual(manifest["counters"]["clonefile_calls_succeeded"], 0)
                self.assertFalse((destination / "only.txt").exists())

        source = self.root / "single-failure-source"
        destination = self.root / "single-failure-destination"
        manifest_path = self.root / "single-failure-manifest.json"
        source.mkdir(); destination.mkdir()
        (source / "a.txt").write_text("first\n", encoding="utf-8")
        (source / "b.txt").write_text("second\n", encoding="utf-8")
        env = dict(os.environ, DYLD_INSERT_LIBRARIES=str(interposer),
                   FAKE_CLONEFILE_FAIL_BASENAME="b.txt", FAKE_CLONEFILE_ERRNO="5")
        result = subprocess.run(
            ["python3", str(CLONE_HELPER), str(source), str(destination), str(manifest_path)],
            env=env, text=True, capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(manifest_path.is_file(), result.stderr)
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual(manifest["counters"]["clonefile_calls_attempted"], 2)
        self.assertEqual(manifest["counters"]["clonefile_calls_succeeded"], 1)
        self.assertTrue((destination / "a.txt").is_file())
        self.assertFalse((destination / "b.txt").exists())

    def test_clone_helper_rejects_special_entries_and_source_races(self) -> None:
        special_source = self.root / "special-source"
        special_destination = self.root / "special-destination"
        special_manifest = self.root / "special-manifest.json"
        special_source.mkdir(); special_destination.mkdir()
        os.mkfifo(special_source / "unsupported.fifo")
        result = subprocess.run(
            ["python3", str(CLONE_HELPER), str(special_source), str(special_destination),
             str(special_manifest)], text=True, capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)
        value = json.loads(special_manifest.read_text(encoding="utf-8"))
        self.assertEqual(value["counters"]["special_entries_rejected"], 1)
        self.assertFalse(value["byte_copy_fallback"])

        interposer = self.build_clonefile_failure_interposer()
        race_source = self.root / "race-source"
        race_destination = self.root / "race-destination"
        race_manifest = self.root / "race-manifest.json"
        race_source.mkdir(); race_destination.mkdir()
        (race_source / "race.txt").write_text("before\n", encoding="utf-8")
        env = dict(os.environ, DYLD_INSERT_LIBRARIES=str(interposer),
                   FAKE_CLONEFILE_FAIL_BASENAME="never-match",
                   FAKE_CLONEFILE_MUTATE_BASENAME="race.txt")
        result = subprocess.run(
            ["python3", str(CLONE_HELPER), str(race_source), str(race_destination),
             str(race_manifest)], env=env, text=True, capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(race_manifest.is_file(), result.stderr)
        value = json.loads(race_manifest.read_text(encoding="utf-8"))
        self.assertEqual(value["status"], "FAIL")
        self.assertIn("raced", value["failure"]["message"])
        self.assertFalse(value["byte_copy_fallback"])

    def test_source_root_trail_mutation_is_detected(self) -> None:
        result = self.run_matrix(
            extra_env={"FAKE_INNER_MUTATE_SOURCE_TRAIL_LANES": "64"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("source repository drifted", result.stderr)
        self.assertEqual([call["lanes"] for call in self.calls()], [64])
        self.assertFalse((self.output / "runs/128").exists())

    def test_publication_revalidates_stored_checker_hash(self) -> None:
        result = self.run_matrix(
            publish="1",
            extra_env={"FAKE_INNER_TAMPER_PRIOR_LANES": "128",
                       "FAKE_INNER_TAMPER_PRIOR_KIND": "checker"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("proof revalidation", result.stderr)
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-64"))
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-128"))

    def test_publication_revalidates_exact_proof_binding(self) -> None:
        result = self.run_matrix(
            publish="1",
            extra_env={"FAKE_INNER_TAMPER_PRIOR_LANES": "128",
                       "FAKE_INNER_TAMPER_PRIOR_KIND": "proof"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("proof revalidation", result.stderr)
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-64"))
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-128"))

    def test_publication_revalidates_bundle_hash_and_heads(self) -> None:
        result = self.run_matrix(
            publish="1",
            extra_env={"FAKE_INNER_TAMPER_PRIOR_LANES": "128",
                       "FAKE_INNER_TAMPER_PRIOR_KIND": "bundle"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("proof revalidation", result.stderr)
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-64"))
        self.assertFalse(self.git_ref_exists("refs/heads/codex/trail-scale-contract-128"))

    def test_tampered_owner_is_rejected_after_inner_returns(self) -> None:
        result = self.run_matrix(extra_env={"FAKE_INNER_TAMPER_OWNER_LANES": "64"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("owner binding", result.stderr)
        self.assertEqual([call["lanes"] for call in self.calls()], [64])
        self.assertFalse((self.output / "runs/64/proof.json").exists())

    def test_pinned_binary_is_rehashed_after_init_before_inner(self) -> None:
        result = self.run_matrix(extra_env={"FAKE_TRAIL_TAMPER_PIN_AFTER_INIT": "1"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("pinned TRAIL_BIN changed", result.stderr)
        self.assertEqual(self.calls(), [])

    def test_existing_final_ref_is_rejected_before_copy_even_without_publish(self) -> None:
        ref = "refs/heads/codex/trail-scale-contract-64"
        self.git("update-ref", ref, "HEAD", "")
        baseline = self.git("rev-parse", ref)
        result = self.run_matrix(publish="0")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])
        self.assertFalse(self.output.exists())
        self.assertEqual(self.git("rev-parse", ref), baseline)

    def test_show_ref_operational_error_is_not_treated_as_absence(self) -> None:
        wrapper_dir = self.root / "show-ref-error-bin"
        wrapper_dir.mkdir()
        wrapper = wrapper_dir / "git"
        wrapper.write_text(GIT_SHOW_REF_ERROR_WRAPPER, encoding="utf-8")
        wrapper.chmod(0o755)
        real_git = shutil.which("git")
        assert real_git is not None
        result = self.run_matrix(extra_env={
            "PATH": str(wrapper_dir) + os.pathsep + os.environ["PATH"],
            "FAKE_REAL_GIT": real_git,
        })
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("show-ref operational failure", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse(self.output.exists())

    def test_symlinked_source_overlap_cross_device_and_destructive_trail_are_rejected(self) -> None:
        source_link = self.root / "source-link"
        os.symlink(self.source, source_link)
        linked_output = self.root / "linked-output"
        result = self.run_matrix(extra_env={"TRAIL_SCALE_REPO": str(source_link)},
                                 output=linked_output)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(linked_output.exists())

        overlap_output = self.source / "nested-output"
        result = self.run_matrix(output=overlap_output)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(overlap_output.exists())

        with tempfile.TemporaryDirectory(dir="/private/tmp") as other_parent:
            cross_output = Path(other_parent) / "matrix-output"
            result = self.run_matrix(output=cross_output)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("same device", result.stderr)
            self.assertFalse(cross_output.exists())

        shutil.rmtree(self.source / ".trail")
        external = self.root / "external-trail-target"
        external.mkdir()
        sentinel = external / "sentinel"
        sentinel.write_text("retain\n", encoding="utf-8")
        os.symlink(external, self.source / ".trail")
        destructive_output = self.root / "destructive-output"
        result = self.run_matrix(output=destructive_output)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(sentinel.is_file())

    def test_term_is_forwarded_to_inner_group_and_retains_marker(self) -> None:
        self.assert_signal_forwarded(signal.SIGTERM, "TERM")

    def test_int_is_forwarded_to_inner_group_and_retains_marker(self) -> None:
        self.assert_signal_forwarded(signal.SIGINT, "INT")

    def test_hup_is_forwarded_to_inner_group_and_retains_marker(self) -> None:
        self.assert_signal_forwarded(signal.SIGHUP, "HUP")

    def assert_signal_forwarded(self, signal_number: int, signal_name: str) -> None:
        pid_file = self.root / f"blocking-inner-{signal_name}.json"
        env = self.matrix_env(extra_env={
            "FAKE_INNER_BLOCK_LANES": "64",
            "FAKE_INNER_PID_FILE": str(pid_file),
        })
        process = subprocess.Popen(
            ["bash", str(ORCHESTRATOR)], env=env, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, start_new_session=True,
        )
        child_pid: int | None = None
        terminated = False
        try:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and not pid_file.is_file():
                if process.poll() is not None:
                    break
                time.sleep(0.05)
            self.assertTrue(pid_file.is_file(), "inner harness did not reach blocking state")
            child_pid = int(json.loads(pid_file.read_text(encoding="utf-8"))["pid"])
            os.killpg(process.pid, signal_number)
            try:
                process.communicate(timeout=5)
                terminated = True
            except subprocess.TimeoutExpired:
                terminated = False
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
            elif child_pid is not None and process_exists(child_pid):
                os.kill(child_pid, signal.SIGKILL)
        self.assertTrue(terminated, f"orchestrator left its inner harness running after {signal_name}")
        self.assertEqual(process.returncode, 128 + signal_number)
        self.assertFalse(process_exists(child_pid))
        marker = self.output / "runs/64/signal-failure.json"
        self.assertTrue(marker.is_file())
        value = json.loads(marker.read_text(encoding="utf-8"))
        self.assertEqual(value["signal"], signal_name)
        self.assertEqual(value["exit_status"], 128 + signal_number)

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

    def build_clonefile_failure_interposer(self) -> Path:
        source = self.root / "fail-clonefile.c"
        library = self.root / "fail-clonefile.dylib"
        source.write_text(
            """
#include <errno.h>
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/clonefile.h>
static int fail_clonefile(const char *src, const char *dst, int flags) {
    const char *base = strrchr(src, '/'); base = base ? base + 1 : src;
    const char *mutate = getenv("FAKE_CLONEFILE_MUTATE_BASENAME");
    if (mutate && strcmp(base, mutate) == 0) {
        int fd = open(src, O_WRONLY | O_APPEND);
        if (fd >= 0) { (void)write(fd, "x", 1); close(fd); }
    }
    const char *failure = getenv("FAKE_CLONEFILE_FAIL_BASENAME");
    if (!failure || strcmp(base, failure) == 0) {
        const char *configured = getenv("FAKE_CLONEFILE_ERRNO");
        errno = configured ? atoi(configured) : EIO;
        return -1;
    }
    return clonefileat(AT_FDCWD, src, AT_FDCWD, dst, flags);
}
#define INTERPOSE(replacement, replacee) __attribute__((used)) static struct { \\
    const void *replacement; const void *replacee; \\
} interpose_##replacee __attribute__((section("__DATA,__interpose"))) = { \\
    (const void *)(uintptr_t)&replacement, (const void *)(uintptr_t)&replacee \\
}
INTERPOSE(fail_clonefile, clonefile);
""".lstrip(),
            encoding="utf-8",
        )
        subprocess.run(
            ["cc", "-dynamiclib", "-o", str(library), str(source)],
            check=True, capture_output=True, text=True,
        )
        return library


def platform_filesystem(path: Path) -> str:
    if sys_platform() == "Darwin":
        output = subprocess.check_output(["diskutil", "info", str(path)], text=True)
        for line in output.splitlines():
            if "File System Personality:" in line:
                return line.split(":", 1)[1].strip().lower()
    return "unknown"


def sys_platform() -> str:
    return subprocess.check_output(["uname", "-s"], text=True).strip()


def process_exists(pid: int | None) -> bool:
    if pid is None:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


if __name__ == "__main__":
    unittest.main()

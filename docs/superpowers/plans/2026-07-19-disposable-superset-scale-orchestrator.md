# Disposable Superset Scale Orchestrator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a tested fail-closed 64/128 scale matrix wrapper that operates only on independent APFS disposable copies and optionally publishes two dedicated refs without moving the source checkout.

**Architecture:** A new Bash entry point owns validation, copy lifecycle, inner-harness invocation, proof creation, and transactional ref publication. Embedded Python helpers produce deterministic filesystem/Git snapshots and exact JSON bindings. A separate Python `unittest` suite supplies fake Trail and inner-harness executables and verifies the process-level contract.

**Tech Stack:** Bash 3.2-compatible shell, Python 3 standard library plus direct macOS `clonefile(2)` binding, Git plumbing, `unittest`.

## Global Constraints

- Work only in the isolated clone; never invoke the real Superset matrix.
- The coordinated release also modifies `scripts/verify-real-repo-lane-scale.sh` and its test to enforce the disposable owner contract; those companion edits are developed outside this isolated orchestrator branch.
- The source, output parent, pinned binary, and both copies must be same-device APFS paths.
- Recursive deletion is allowed only for the exact validated copied `.trail` directory.
- Failed and successful copies/evidence are retained.
- Publication defaults off and is possible only after both checker PASS proofs.
- Inner authorization uses `TRAIL_SCALE_DISPOSABLE_WORKSPACE=1` and the exact owner-file JSON contract from the design.

---

### Task 1: Process-level contract tests

**Files:**
- Create: `scripts/test_verify_real_repo_lane_scale_matrix.py`
- Test: `scripts/test_verify_real_repo_lane_scale_matrix.py`

**Interfaces:**
- Consumes: executable path `scripts/verify-real-repo-lane-scale-matrix.sh` and environment documented in the design.
- Produces: fake Trail CLI, fake inner harness, source fixture with tracked/untracked/ignored/symlink state, and assertions covering success/failure/publication.

- [ ] **Step 1: Build the isolated fixture and fake executables**

Create a `unittest.TestCase` whose setup initializes a source Git repository under `/Volumes/Workspace` when available. Add a committed tracked file, ignored regular file and directory, untracked regular file, relative symlink, and pre-existing `.trail/stale` state. The fake Trail must support:

```text
trail --version
trail init --help
trail --workspace COPY --json init --from-git
```

The init command must reject an existing `.trail`, create a new copy-local `.trail`, and record the initialized copy path. The fake inner harness must validate the exact owner-file JSON and marker, record one invocation, create an evidence `checker.out` JSON PASS, create a commit on the requested dedicated ref, and support an injected failure before commit.

- [ ] **Step 2: Write the independent-run and preservation test**

Run the orchestrator with publication disabled and assert:

```python
self.assertEqual([call["lanes"] for call in calls], [64, 128])
self.assertNotEqual(calls[0]["repo"], calls[1]["repo"])
self.assertNotEqual(calls[0]["trail_dir"], calls[1]["trail_dir"])
self.assertEqual(snapshot_source(), baseline_source)
self.assertTrue((output / "runs/64/proof.json").is_file())
self.assertTrue((output / "runs/128/proof.json").is_file())
```

Compare copy entries to the fixture for tracked, ignored, untracked, directory, and symlink content/type/mode. Assert copied stale `.trail` is absent and fake init state is present.

- [ ] **Step 3: Write the failure-containment test**

Inject a 64-run inner failure. Assert nonzero exit, one invocation, retained copy/log/evidence, no source ref additions, unchanged source snapshot, and no 128 invocation.

- [ ] **Step 4: Write publication and CAS tests**

With publication enabled, assert both unique refs exist at the proof commits while source HEAD, symbolic branch, index digest, worktree snapshot, and all original refs are unchanged. In a fresh fixture, pre-create the second expected publication ref and assert preflight refusal, zero inner invocations, zero first publication ref, and unchanged source state apart from the deliberately pre-created collision.

- [ ] **Step 5: Run tests to verify RED**

Run:

```bash
python3 -m unittest scripts/test_verify_real_repo_lane_scale_matrix.py -v
```

Expected: failure because `scripts/verify-real-repo-lane-scale-matrix.sh` does not exist.

---

### Task 2: Validation, snapshot, and disposable-copy lifecycle

**Files:**
- Create: `scripts/verify-real-repo-lane-scale-matrix.sh`
- Test: `scripts/test_verify_real_repo_lane_scale_matrix.py`

**Interfaces:**
- Consumes: `TRAIL_BIN`, `TRAIL_SCALE_REPO`, `TRAIL_SCALE_MATRIX_OUTPUT`, optional run/publish/inner settings.
- Produces: deterministic `source-baseline.json`, per-copy inventory, pinned binary, exact mktemp copy paths, owner bindings, and inner invocation logs.

- [ ] **Step 1: Add fail-closed scalar/path validation**

Implement `die`, `sha256_file`, `canonical_dir`, `device_id`, `filesystem_type`, `validate_copy_target`, and `assert_source_snapshot`. Reject unsafe identifiers, non-absolute paths, symlinks, existing output, overlapping source/output, dirty tracked/index state, non-APFS or differing devices, and publication values other than `0`/`1`.

- [ ] **Step 2: Add deterministic inventories**

Embed Python that uses `os.scandir(..., follow_symlinks=False)` and `os.lstat` to record every entry's relative path, type, mode, nanosecond mtime, size, and SHA-256 of regular bytes or symlink target. Produce both a full-copy inventory and a full source invariant inventory including root `.trail`; separately record HEAD, symbolic HEAD, complete sorted refs, porcelain status with ignored entries, and Git index type/mode/digest.

- [ ] **Step 3: Pin and validate Trail**

Use the secure no-follow, exclusive byte-copy path to create `pinned/trail`, fsync it, set mode `0555` and its directory non-writable, require matching SHA-256, run `--version`, run `init --help`, and require the help text to contain exact `--from-git` support.

- [ ] **Step 4: Create and initialize each copy**

For counts `64 128`, use:

```bash
copy=$(mktemp -d "$run_root/copy-$count.XXXXXX")
python3 scripts/apfs-clone-tree.py "$source" "$copy" "$run_dir/clone-manifest.json"
```

Validate exact prefix/parent/realpath/device/APFS and require the manifest path to be outside source and destination. Require a clonefile-only PASS manifest with complete syscall, hardlink, device, size, and digest accounting; compare full inventories; validate and remove only `$copy/.trail`; invoke the pinned binary with `--workspace "$copy" --json init --from-git`; and create the exact owner JSON beneath the new `$copy/.trail`. Never byte-copy-fallback a disposable tree.

- [ ] **Step 5: Invoke the inner harness once**

Build all inner variables in one `run_inner_harness` function so the hardening interface is localized. Set exact copy/output/run/ref/lane variables plus:

```text
TRAIL_SCALE_DISPOSABLE_WORKSPACE=1
TRAIL_SCALE_DISPOSABLE_OWNER_FILE=COPY/.trail/scale-disposable-owner.json
```

Capture stdout/stderr and exit code without deleting anything. Require zero exit and checker JSON `{status: PASS, lanes: COUNT, ...}`.

- [ ] **Step 6: Run tests to verify lifecycle GREEN**

Run the focused unittest command. Expected: independent/preservation and failure-containment cases pass; publication cases remain failing until Task 3.

---

### Task 3: Bundle proof and atomic optional publication

**Files:**
- Modify: `scripts/verify-real-repo-lane-scale-matrix.sh`
- Test: `scripts/test_verify_real_repo_lane_scale_matrix.py`

**Interfaces:**
- Consumes: passing inner evidence and each copy's dedicated ref.
- Produces: verified bundle, commit/tree proof, optional two-ref atomic source publication.

- [ ] **Step 1: Produce proof after each PASS**

Resolve the copy baseline, dedicated commit, and tree; require exactly one parent equal to baseline and `rev-list --count baseline..commit == 1`. Create `final.bundle`, run `git bundle verify`, and write `proof.json` containing schema version, lanes, run ID, copy, evidence, ref, baseline, commit, tree, bundle, and SHA-256 values.

- [ ] **Step 2: Add publication preflight**

Before any run, derive both final source refs and require both absent in every publish mode, distinguishing absent exit 1 from `show-ref` errors. Recheck absence and the complete source invariant after both proofs, immediately before object import.

- [ ] **Step 3: Revalidate, import, and atomically create refs**

Parse each original proof once and capture its validated commit/tree/ref/digest. Revalidate the exact schema/bindings, all stored hashes, checker PASS, bundle verification and exact heads, and the exact one-commit parent topology while securely staging the already-open bundle; write a sealed proof record bound to that staged path and digest. Rehash each read-only staged bundle immediately before fetching its captured ref with `git fetch --no-tags --no-write-fetch-head STAGED_BUNDLE REF`, verify imported commit/tree identities from captured values, then use one `git update-ref --stdin` transaction with two captured `create REF COMMIT` commands. This makes absence the CAS condition and prevents partial ref creation or proof/bundle TOCTOU substitution.

- [ ] **Step 4: Recheck final source state**

Require unchanged source HEAD, symbolic HEAD, index, worktree/status/inventory, and unchanged original refs. Require exactly the two derived ref additions at the proof commits. Write `matrix-summary.json` only after this verification and only from the sealed proof records, never by reopening the mutable originals.

- [ ] **Step 5: Run all focused tests GREEN**

Run:

```bash
python3 -m unittest scripts/test_verify_real_repo_lane_scale_matrix.py -v
```

Expected: all independent, preservation, failure, publication, and CAS tests pass with no warnings.

---

### Task 4: Final verification and commit

**Files:**
- Verify: `scripts/verify-real-repo-lane-scale-matrix.sh`
- Verify: `scripts/test_verify_real_repo_lane_scale_matrix.py`
- Verify: design and plan documents

**Interfaces:**
- Consumes: completed implementation.
- Produces: committed isolated-clone result and exact RED/GREEN evidence.

- [ ] **Step 1: Run shell syntax and Python compilation**

```bash
bash -n scripts/verify-real-repo-lane-scale-matrix.sh
python3 -m py_compile scripts/test_verify_real_repo_lane_scale_matrix.py
```

- [ ] **Step 2: Run the focused suite fresh**

```bash
python3 -m unittest scripts/test_verify_real_repo_lane_scale_matrix.py -v
```

Expected: all tests pass, zero failures/errors/skips.

- [ ] **Step 3: Inspect scope and whitespace**

```bash
git diff --check
git status --short
```

Expected: only the new orchestrator, its new test, and approved docs are changed.

- [ ] **Step 4: Commit implementation**

```bash
git add scripts/verify-real-repo-lane-scale-matrix.sh scripts/test_verify_real_repo_lane_scale_matrix.py
git commit -m "test: orchestrate disposable Superset scale matrix"
```

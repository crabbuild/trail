# Disposable Superset Scale Orchestrator Design

## Goal

Add a fail-closed wrapper that runs the existing real-repository lane-scale harness independently at 64 and 128 lanes without mutating the source Superset checkout. Each run uses its own same-volume APFS copy, Trail database, run identity, Git ref, and evidence directory. Publishing the two resulting commits back to the source repository is explicit and optional.

## Interface

The new executable is `scripts/verify-real-repo-lane-scale-matrix.sh`.

Required environment:

- `TRAIL_BIN`: absolute executable candidate binary.
- `TRAIL_SCALE_REPO`: absolute source Git worktree.
- `TRAIL_SCALE_MATRIX_OUTPUT`: absolute, absent output path whose existing parent is on the same APFS device as the source.

Optional environment:

- `TRAIL_SCALE_INNER_HARNESS`: absolute executable inner harness; defaults to `scripts/verify-real-repo-lane-scale.sh`.
- `TRAIL_SCALE_MATRIX_RUN_ID`: safe identifier; defaults to a timestamp, PID, and random suffix.
- `TRAIL_SCALE_MATRIX_PUBLISH`: `0` by default; `1` imports both commits and creates their dedicated source refs only after both runs pass.
- Existing scale/fault-attestation variables are forwarded where applicable. Lane count, output, run ID, Git ref, repository, and marker variables are always set by the orchestrator.

The inner harness receives these additional exact markers:

- `TRAIL_SCALE_DISPOSABLE_WORKSPACE=1`
- `TRAIL_SCALE_DISPOSABLE_OWNER_FILE=<exact COPY/.trail/scale-disposable-owner.json>`

The owner file is a regular non-symlink JSON file created after copy-local Trail initialization. It has exactly `{schema_version: 1, kind: "trail_scale_disposable_workspace", canonical_repo, disposable_repo, output, run_id}` and binds the resolved source, resolved copy, normalized absolute evidence output, and exact run ID. The companion hardening change modifies the inner harness and its tests to validate both variables and the exact binding before exercising mutation authority. The harness continues to honor explicit `TRAIL_SCALE_REPO`, `TRAIL_SCALE_OUTPUT`, `TRAIL_SCALE_RUN_ID`, `TRAIL_SCALE_GIT_REF`, `TRAIL_SCALE_LANES`, candidate identity, and fault-attestation inputs, and leaves a JSON checker result with `status: PASS` in `checker.out` on success.

## Preflight and source invariant

The orchestrator rejects relative, symlinked, missing, overlapping, non-Git, dirty tracked/index, cross-device, or non-APFS inputs. The candidate binary and inner harness are regular absolute executable files. A private, read-only byte copy of `TRAIL_BIN` is securely made under the output root, its SHA-256 is checked, and `trail init --help` must successfully advertise `--from-git` before any copied `.trail` is removed.

Before copying, the orchestrator records:

- exact `HEAD` object and symbolic target;
- sorted complete Git ref names and targets;
- porcelain status including ignored/untracked entries;
- the exact Git index file type, mode, and digest;
- every source filesystem entry including the root `.trail` subtree, directories, regular files, symlinks, ignored entries, modes, nanosecond mtimes, sizes, and content/target digests;
- a full copy-comparison inventory including `.git` and `.trail` before copied Trail state is removed.

The source invariant is rechecked after preparation, after each inner run, before optional publication, and after publication. Before publication every snapshot, including the source root `.trail`, must match byte-for-byte. After publication, worktree/status/HEAD/index state including `.trail` must still match, and the only ref additions may be the two exact run refs.

## Copy and run lifecycle

For lane counts 64 and 128, the orchestrator obtains an exact empty destination from `mktemp -d "$output/copy-$count.XXXXXX"`. It verifies the destination's canonical parent, prefix, device, and APFS type, then invokes `scripts/apfs-clone-tree.py` with a manifest path outside both trees. The helper rejects manifests beneath the source or destination, calls macOS `clonefile(2)` directly once for every unique regular-file inode, and recreates additional paths as hardlinks. It never falls back to byte copying. Directories, modes, timestamps, symlinks, Git metadata, tracked, untracked, and ignored entries are preserved; special entries, source races, and any failed clone syscall abort with a retained FAIL manifest. The PASS manifest binds source/destination devices, accounts for every regular path and syscall, records source/destination sizes, and requires matching independent tree/inventory digests.

The full source and copy inventories must match before mutation. Only `destination/.trail` may then be recursively removed, after checking the exact canonical destination, target type, prefix, device, and parent relationship. The pinned Trail binary initializes the copy with:

```text
trail --workspace COPY --json init --from-git
```

The inner harness is invoked exactly once for that copy with a unique run ID, a unique `refs/heads/codex/trail-scale-...` ref, an exclusive evidence directory, explicit lane count, and the disposable owner binding. The 64 and 128 runs never share a path or `.trail` tree.

An inner run passes only when its process exits zero and its `checker.out` is a single JSON object whose `status` is `PASS` and lane count matches. A failed run stops the matrix, retains its copy, logs, and evidence, leaves the source invariant unchanged, and performs no publication.

## Proof and optional publication

After each pass the orchestrator resolves the dedicated ref and requires its commit to have exactly one parent equal to the copied baseline HEAD with exactly one commit in `baseline..commit`. It records the commit and tree IDs, creates a Git bundle containing the dedicated ref, verifies the bundle, and writes a JSON proof plus SHA-256 digests.

Both final run refs must be absent before any copy regardless of publish mode, and `git show-ref` operational failures are not treated as absence. Only after both proofs pass may `TRAIL_SCALE_MATRIX_PUBLISH=1` change the source Git database. Immediately before import, the orchestrator parses each proof once; revalidates its exact schema/binding, binary/checker/bundle hash, checker PASS, exact one-commit parent topology, `git bundle verify`, and single exact `list-heads` result; and copies the already-open validated bundle into a read-only publication staging directory. Commit, tree, ref, staged path, and digest are captured once. Each staged bundle is rehashed immediately before its validated captured ref is fetched with `--no-write-fetch-head`, followed by one `git update-ref --stdin` transaction using only captured values and absent-only create semantics. A pre-existing or racing ref aborts publication; master, HEAD, index, worktree, and every other ref remain unchanged.

The release binary pin is deliberately separate from tree cloning: it is a byte copy created with `O_NOFOLLOW|O_EXCL`, complete-write checks, file and directory `fsync`, mode `0555`, and a non-writable pin directory. Its digest is checked before initialization, before each inner invocation, and after each run.

## Failure handling and tests

There is no automatic recursive cleanup of run copies. This intentionally retains evidence and avoids destructive recovery logic. The only recursive deletion is the copied `.trail`, guarded by exact target validations.

The inner harness runs in a dedicated child process group. `INT`, `TERM`, and `HUP` traps forward the signal to that group, wait for it (with a bounded KILL watchdog), retain a signal-failure marker, and exit with the conventional `128 + signal` status so no inner process is orphaned.

The new test file `scripts/test_verify_real_repo_lane_scale_matrix.py` uses a fake immutable Trail CLI and fake inner harness. It proves:

- two independent 64/128 invocations and distinct `.trail`, run, ref, and evidence identities;
- tracked, untracked, ignored, directory, symlink, hardlink, mode, and timestamp preservation in both copies;
- direct clonefile failure without byte-copy fallback, complete manifests, special-entry/race rejection, and device/size/digest accounting;
- removal only of copied pre-existing `.trail` and initialization of fresh copy-local Trail state;
- failure containment and retained evidence/copies;
- exact source HEAD/ref/index/status/filesystem preservation including source `.trail`;
- bundle/commit/tree proof production;
- optional publication of two unique refs without moving the source checkout;
- refusal when either publication ref already exists or ref lookup errors, with no partial new ref;
- owner/proof/bundle/checker/binary tamper rejection and INT/TERM/HUP child-group containment.

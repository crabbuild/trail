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
- `TRAIL_SCALE_DISPOSABLE_OWNER_FILE=<absolute owner file under copied .trail>`

The owner file is a regular non-symlink JSON file created after copy-local Trail initialization. It has exactly `{schema_version: 1, kind: "trail_scale_disposable_workspace", canonical_repo, disposable_repo, output, run_id}` and binds the resolved source, resolved copy, normalized absolute evidence output, and exact run ID. The hardened inner harness is expected to validate both variables and the exact binding before exercising mutation authority. It must continue to honor explicit `TRAIL_SCALE_REPO`, `TRAIL_SCALE_OUTPUT`, `TRAIL_SCALE_RUN_ID`, `TRAIL_SCALE_GIT_REF`, `TRAIL_SCALE_LANES`, candidate identity, and fault-attestation inputs, and must leave a JSON checker result with `status: PASS` in `checker.out` on success.

## Preflight and source invariant

The orchestrator rejects relative, symlinked, missing, overlapping, non-Git, dirty tracked/index, cross-device, or non-APFS inputs. The candidate binary and inner harness are regular absolute executable files. A private, read-only COW copy of `TRAIL_BIN` is made under the output root, its SHA-256 is checked, and `trail init --help` must successfully advertise `--from-git` before any copied `.trail` is removed.

Before copying, the orchestrator records:

- exact `HEAD` object and symbolic target;
- sorted complete Git ref names and targets;
- porcelain status including ignored/untracked entries;
- the exact Git index file type, mode, and digest;
- every source filesystem entry except the root `.trail` subtree, including directories, regular files, symlinks, ignored entries, modes, sizes, and content/target digests;
- a full copy-comparison inventory including `.git` and `.trail` before copied Trail state is removed.

The source invariant is rechecked after preparation, after each inner run, before optional publication, and after publication. Before publication every snapshot must match byte-for-byte. After publication, worktree/status/HEAD/index/non-`.trail` worktree state must still match, and the only ref additions may be the two exact run refs.

## Copy and run lifecycle

For lane counts 64 and 128, the orchestrator obtains an exact empty destination from `mktemp -d "$output/copy-$count.XXXXXX"`. It verifies the destination's canonical parent, prefix, device, and APFS type, then uses macOS `cp -cRp source/. destination/` to preserve Git metadata plus tracked, untracked, ignored, directory, and symlink state using APFS COW clones.

The full source and copy inventories must match before mutation. Only `destination/.trail` may then be recursively removed, after checking the exact canonical destination, target type, prefix, device, and parent relationship. The pinned Trail binary initializes the copy with:

```text
trail --workspace COPY --json init --from-git
```

The inner harness is invoked exactly once for that copy with a unique run ID, a unique `refs/heads/codex/trail-scale-...` ref, an exclusive evidence directory, explicit lane count, and the disposable owner binding. The 64 and 128 runs never share a path or `.trail` tree.

An inner run passes only when its process exits zero and its `checker.out` is a single JSON object whose `status` is `PASS` and lane count matches. A failed run stops the matrix, retains its copy, logs, and evidence, leaves the source invariant unchanged, and performs no publication.

## Proof and optional publication

After each pass the orchestrator resolves the dedicated ref, verifies the commit descends from the copy's original HEAD, records the commit and tree IDs, creates a Git bundle containing the dedicated ref, verifies the bundle, and writes a JSON proof plus SHA-256 digests.

Only after both proofs pass may `TRAIL_SCALE_MATRIX_PUBLISH=1` change the source Git database. Both bundles are fetched with `--no-write-fetch-head`, their exact commit/tree identities are rechecked, then one `git update-ref --stdin` transaction creates both dedicated refs with absent-only CAS semantics. A pre-existing or racing ref aborts publication; master, HEAD, index, worktree, and every other ref remain unchanged.

## Failure handling and tests

There is no automatic recursive cleanup of run copies. This intentionally retains evidence and avoids destructive recovery logic. The only recursive deletion is the copied `.trail`, guarded by exact target validations.

The new test file `scripts/test_verify_real_repo_lane_scale_matrix.py` uses a fake immutable Trail CLI and fake inner harness. It proves:

- two independent 64/128 invocations and distinct `.trail`, run, ref, and evidence identities;
- tracked, untracked, ignored, directory, and symlink preservation in both copies;
- removal only of copied pre-existing `.trail` and initialization of fresh copy-local Trail state;
- failure containment and retained evidence/copies;
- exact source HEAD/ref/index/status/filesystem preservation;
- bundle/commit/tree proof production;
- optional publication of two unique refs without moving the source checkout;
- refusal when either publication ref already exists, with no partial new ref.

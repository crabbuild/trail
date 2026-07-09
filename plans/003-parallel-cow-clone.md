# Plan 003: Parallelize copy-on-write cloning

> **Executor instructions**: Follow this plan step by step. Run each verification command before moving to the next step. If a STOP condition occurs, stop and report instead of broadening scope. When done, update the status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 6bb6fa7..HEAD -- crates/trail/src/db/util/fs_cow.rs crates/trail/src/db/lane/lifecycle.rs crates/trail/src/db/lane/workdir/manifest.rs`
>
> If any in-scope file changed since this plan was written, compare the "Current state" section against live code before proceeding. Treat a semantic mismatch as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/002-single-pass-workdir-manifest.md`
- **Category**: perf
- **Planned at**: commit `6bb6fa7`, 2026-06-27

## Why this matters

After source and destination full scans are removed, serial file cloning becomes the next visible startup cost. CoW clone operations are per-file filesystem work and do not require SQLite access, so bounded parallelism should reduce materialized lane/worktree startup time on large roots.

## Current state

- `crates/trail/src/db/util/fs_cow.rs` owns CoW cloning. Around lines 10-37, `materialize_from_workspace_cow` loops serially over the target entries.
- The same file has per-file stamped validation around lines 61-83 and clone implementation around lines 124-160.
- The `trail` crate already depends on workspace `rayon`, so no new dependency should be needed.
- Plan 002 should have introduced a materialization report or equivalent stamp return path that parallel workers can populate.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Format | `make fmt-check` | exit 0 |
| Check | `cargo check -p trail` | exit 0, no compiler errors |
| Focused tests | `cargo test -p trail parallel_cow_clone` | all new tests pass |
| Lane regression | `cargo test -p trail lane` | all matching lane tests pass |
| Smoke benchmark | `make bench-cli-scale-smoke` | exit 0 and prints benchmark summary |

## Scope

**In scope**:

- `crates/trail/src/db/util/fs_cow.rs`
- small call-site adjustments in `crates/trail/src/db/lane/lifecycle.rs`
- materialization report integration in `crates/trail/src/db/lane/workdir/manifest.rs` if needed
- focused lane/CoW tests

**Out of scope**:

- new global thread pool management
- SQLite access from Rayon workers
- source-index schema changes
- streaming root materialization; that is plan 004

## Git workflow

- Branch: `advisor/003-parallel-cow-clone`
- Commit style: conventional commits, for example `perf: parallelize CoW file cloning`
- Do not push or open a PR unless explicitly asked.

## Steps

### Step 1: Extract worker-safe per-file clone logic

In `fs_cow.rs`, extract the current per-file clone and validation logic into a helper that only receives immutable data and paths. It must not borrow a SQLite connection and must not mutate shared maps directly.

The helper should return a per-path result containing:

- cloned/skipped/unavailable status
- optional destination stamp if plan 002's report path exists
- enough path context for deterministic error reporting

**Verify**: `cargo check -p trail` -> exit 0.

### Step 2: Add a serial capability probe

Before spawning parallel work, choose the first eligible file and run the clone path serially. If CoW is unavailable on this filesystem, return the existing fallback result without starting Rayon work.

Keep behavior for zero-file targets unchanged.

**Verify**: `cargo test -p trail parallel_cow_clone_probe` -> new probe tests pass after they are added.

### Step 3: Clone remaining files with Rayon

Use `rayon::prelude::*` to process remaining entries with `par_iter`. Keep `create_dir_all` idempotent, collect results into a deterministic `BTreeMap`, and merge the serial probe result back into the same report.

On any worker error:

- remove files created by this CoW attempt where practical
- do not write a clean manifest
- return the existing error/fallback shape expected by callers

**Verify**: `cargo check -p trail` -> exit 0.

### Step 4: Add focused tests

Add tests with prefix `parallel_cow_clone` covering:

- many-file materialization succeeds and produces a clean workdir
- executable bits survive
- one source stamp mismatch prevents a false clean result
- fallback object materialization still runs if CoW is rejected

Use platform guards if the existing CoW tests already have them.

**Verify**: `cargo test -p trail parallel_cow_clone` -> all new tests pass.

### Step 5: Run the production gate and benchmark

Run:

```sh
make fmt-check
cargo check -p trail
cargo test -p trail
make bench-cli-scale-smoke
TRAIL_SCALE_MATERIALIZED=1 TRAIL_SCALE_FILES=100000 make bench-cli-scale-smoke
```

If the smoke target ignores `TRAIL_SCALE_FILES`, use the nearest existing scale target from the Makefile and record that substitution.

**Verify**: all commands exit 0. Materialized startup should improve relative to the post-plan-002 baseline, or the executor must report the measured numbers and likely reason.

## Test plan

- New tests with prefix `parallel_cow_clone`.
- Existing lane regression via `cargo test -p trail lane`.
- Full crate tests before DONE: `cargo test -p trail`.
- Scale smoke with materialized workdir enabled.

## Done criteria

- [ ] Eligible CoW materialization uses bounded Rayon parallelism after a serial probe.
- [ ] Workers do not access SQLite or shared mutable state unsafely.
- [ ] Partial CoW success cannot produce a clean manifest.
- [ ] Fallback behavior remains deterministic.
- [ ] Shared validation gate exits 0.
- [ ] `plans/README.md` status row is updated.

## STOP conditions

Stop and report if:

- Parallel workers need shared SQLite state.
- Cleanup after partial worker failure is unreliable enough to risk corrupting an existing workdir.
- Benchmarks show worse p95 materialized startup on the target filesystem.
- The change requires a custom thread pool before proving default Rayon is insufficient.

## Maintenance notes

- Reviewers should check error aggregation and partial cleanup carefully.
- Future filesystem clone backends must keep the per-file helper worker-safe.
- If p95 regresses on APFS or Linux reflink filesystems, keep a serial fallback behind a small internal switch.

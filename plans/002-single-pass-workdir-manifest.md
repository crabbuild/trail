# Plan 002: Write clean workdir manifests from materialization stamps

> **Executor instructions**: Follow this plan step by step. Run each verification command before moving to the next step. If a STOP condition occurs, stop and report instead of broadening scope. When done, update the status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 6bb6fa7..HEAD -- trail/src/db/lane/workdir/manifest.rs trail/src/db/lane/lifecycle.rs trail/src/db/storage/content.rs trail/src/db/util/fs_cow.rs`
>
> If any in-scope file changed since this plan was written, compare the "Current state" section against live code before proceeding. Treat a semantic mismatch as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `6bb6fa7`, 2026-06-27

## Why this matters

After lane files are cloned or written, Trail scans the destination workdir to build the clean manifest. That duplicates work because materialization just touched every destination file. Capturing destination stamps during materialization removes a full post-write walk and makes the CoW startup path substantially flatter on large worktrees.

## Current state

- `trail/src/db/lane/workdir/manifest.rs` writes clean manifests. Around lines 106-160, `write_clean_workdir_manifest` calls `scan_workdir_file_stamps(dir)`.
- The same module has `write_clean_workdir_manifest_from_disk_manifest` around lines 162-203, and it also calls `scan_workdir_file_stamps(dir)`.
- `scan_workdir_file_stamps` around lines 333-379 canonicalizes the root, walks visible files, stats each file, and builds `WorkdirFileStamp` values.
- `trail/src/db/lane/lifecycle.rs` writes the clean workdir manifest after materialization around line 300.
- `trail/src/db/util/fs_cow.rs` and `trail/src/db/storage/content.rs` already have the file-level write/clone points where destination stamps can be captured.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Format | `make fmt-check` | exit 0 |
| Check | `cargo check -p trail` | exit 0, no compiler errors |
| Focused tests | `cargo test -p trail workdir_manifest_from_materialization_stamps` | all new tests pass |
| Lane regression | `cargo test -p trail lane` | all matching lane tests pass |
| Smoke benchmark | `make bench-cli-scale-smoke` | exit 0 and prints benchmark summary |

## Scope

**In scope**:

- `trail/src/db/lane/workdir/manifest.rs`
- `trail/src/db/lane/lifecycle.rs`
- `trail/src/db/storage/content.rs`
- `trail/src/db/util/fs_cow.rs`
- focused workdir/lane tests

**Out of scope**:

- manifest file format changes
- worktree source-index schema changes
- public CLI behavior changes
- parallelization of CoW clone work; that is plan 003

## Git workflow

- Branch: `advisor/002-single-pass-workdir-manifest`
- Commit style: conventional commits, for example `perf: write workdir manifests from materialization stamps`
- Do not push or open a PR unless explicitly asked.

## Steps

### Step 1: Introduce a materialization report type

Add a small internal report type near the materialization helpers or manifest module:

```rust
pub(crate) struct MaterializedWorkdir {
    pub files_written: usize,
    pub stamps: BTreeMap<String, WorkdirFileStamp>,
}
```

The exact location can follow existing module ownership, but keep `WorkdirFileStamp` serialization details inside `manifest.rs` if possible.

**Verify**: `cargo check -p trail` -> exit 0.

### Step 2: Add a manifest writer that accepts complete stamps

In `manifest.rs`, add a function equivalent to:

```rust
pub(crate) fn write_clean_workdir_manifest_from_stamps(
    dir: &Path,
    root_id: &Cid,
    files: &BTreeMap<String, FileEntry>,
    stamps: BTreeMap<String, WorkdirFileStamp>,
) -> Result<()>
```

Rules:

- Verify the stamp map has exactly the same path set as `files`.
- Preserve the current manifest JSON shape.
- Reject missing or extra stamps.
- Do not call `scan_workdir_file_stamps`.
- Keep existing scan-based writers for import, repair, and fallback paths.

**Verify**: `cargo test -p trail workdir_manifest_from_materialization_stamps` -> new manifest tests pass after they are added in Step 3.

### Step 3: Add manifest equivalence and rejection tests

Add tests with prefix `workdir_manifest_from_materialization_stamps` covering:

- a stamp-based manifest parses to the same structure as a scan-based manifest on a small fixture
- missing stamp is rejected
- extra stamp is rejected
- executable metadata is represented the same way as the scan-based path

If JSON byte order is unstable, parse both manifests and compare values.

**Verify**: `cargo test -p trail workdir_manifest_from_materialization_stamps` -> all new tests pass.

### Step 4: Capture destination stamps during materialization

Update ordinary object materialization and successful CoW clone paths to return destination stamps after file content, executable mode, xattrs, and optional sync have been applied. Use the same metadata fields as `scan_workdir_file_stamps`.

If a path cannot produce a stamp, return an incomplete report and keep callers on the scan-based manifest path.

**Verify**: `cargo check -p trail` -> exit 0.

### Step 5: Use report-based manifest writing in lane creation

In `lifecycle.rs`, when materialization returns a complete report, call the new stamp-based manifest writer. Fall back to the existing scan-based writer only when the report is unavailable or incomplete.

**Verify**: `cargo test -p trail lane` -> all matching lane tests pass.

### Step 6: Run the production gate and benchmark

Run:

```sh
make fmt-check
cargo check -p trail
cargo test -p trail
make bench-cli-scale-smoke
TRAIL_SCALE_MATERIALIZED=1 make bench-cli-scale-smoke
```

**Verify**: all commands exit 0. Materialized startup should improve relative to the post-plan-001 baseline, or the executor must report the measured numbers and likely reason.

## Test plan

- New manifest tests with prefix `workdir_manifest_from_materialization_stamps`.
- Existing lane regression via `cargo test -p trail lane`.
- Dirty detection check: add or reuse a test that modifies a destination file after manifest creation and confirms status is dirty.
- Full crate tests before DONE: `cargo test -p trail`.

## Done criteria

- [ ] Complete materialization reports can write clean manifests without scanning the destination workdir.
- [ ] Incomplete reports are rejected or fall back to scan-based manifest writing.
- [ ] Manifest format remains unchanged.
- [ ] Dirty detection still reports modified destination files.
- [ ] Shared validation gate exits 0.
- [ ] `plans/README.md` status row is updated.

## STOP conditions

Stop and report if:

- Complete destination stamps cannot be captured after all metadata changes are applied.
- Preserving the manifest format is not possible.
- A partial/best-effort materialization path would be able to write a clean manifest.
- The change requires broad unrelated API churn outside the in-scope files.

## Maintenance notes

- Reviewers should compare stamp semantics against `scan_workdir_file_stamps`; this plan is only correct if both paths mean the same thing.
- Future materializers must either return complete stamps or explicitly choose the scan-based writer.
- Plan 003 should use this report type for parallel CoW worker results.

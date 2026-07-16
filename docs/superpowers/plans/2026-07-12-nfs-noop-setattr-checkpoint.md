# NFS No-op `SETATTR` Checkpoint Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent metadata-only NFS `SETATTR` requests from creating false source-file deletions in Trail checkpoints.

**Architecture:** Enforce the no-op invariant in `ViewCore::setattr`, the shared mutation boundary used by filesystem adapters. A regression test exercises the real lower-root, upper-layer, journal, and checkpoint-candidate flow so the bug cannot recur without failing the test.

**Tech Stack:** Rust, Trail workspace-view core, Cargo test harness.

## Global Constraints

- Calls with neither size nor mode must return visible attributes without copying up or journaling the path.
- Calls with size, mode, or both must retain existing copy-up and journaling behavior.
- Invalid or stale inode errors must remain observable.
- Timestamp, ownership, ACL, and extended-attribute support remain out of scope.
- Existing corrupted lanes are not modified by this implementation.

---

## File Structure

- Modify and test: `trail/src/db/lane/workdir/view_core.rs`
  - `ViewCore::setattr` owns the shared no-op guard.
  - The colocated Unix test module owns the lower-file checkpoint regression test.

### Task 1: Make empty supported-attribute updates side-effect free

**Files:**
- Modify: `trail/src/db/lane/workdir/view_core.rs:996`
- Test: `trail/src/db/lane/workdir/view_core.rs:2199`

**Interfaces:**
- Consumes: `ViewCore::setattr(&mut self, ino: u64, size: Option<u64>, mode: Option<u32>) -> Result<ViewNodeAttr, i32>`
- Produces: The invariant that `size == None && mode == None` returns current attributes without adding a checkpoint candidate.

- [x] **Step 1: Write the failing regression test**

Add this test immediately before `view_core_conformance_truncate_mode_and_symlink_escape`:

```rust
#[test]
fn view_core_noop_setattr_does_not_create_checkpoint_candidate() {
    let (_temp, db, root, upper) = fixture();
    let mut view = lazy_core(&db, &root, upper.clone());
    let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();

    let attr = view.setattr(readme, None, None).unwrap();

    assert_eq!(attr.ino, readme);
    assert!(!upper.join("README.md").exists());
    let candidates = view.checkpoint_candidates().unwrap();
    assert!(!candidates.paths.contains("README.md"));
}
```

- [x] **Step 2: Run the regression test and verify RED**

Run:

```sh
cargo test -p trail view_core_noop_setattr_does_not_create_checkpoint_candidate
```

Expected: FAIL at `assert!(!candidates.paths.contains("README.md"))`, proving the current no-op `setattr` incorrectly journals the lower file.

- [x] **Step 3: Add the minimal shared-core guard**

Change the start of `ViewCore::setattr` to resolve the inode before deciding whether the request mutates supported attributes:

```rust
pub(crate) fn setattr(
    &mut self,
    ino: u64,
    size: Option<u64>,
    mode: Option<u32>,
) -> std::result::Result<ViewNodeAttr, i32> {
    let path = self.path_for_ino(ino)?;
    if size.is_none() && mode.is_none() {
        return self.attr(&path);
    }
    let _barrier = self.begin_mutation()?;
```

Keep the existing size, mode, journal, and final-attribute logic unchanged after the guard.

- [x] **Step 4: Run the regression test and verify GREEN**

Run:

```sh
cargo test -p trail view_core_noop_setattr_does_not_create_checkpoint_candidate
```

Expected: PASS with one matching test and zero failures.

- [x] **Step 5: Run focused workspace-view and NFS tests**

Run:

```sh
cargo test -p trail view_core
cargo test -p trail nfs_adapter
```

Expected: Both commands exit 0 with zero failed tests. The NFS filter may match only macOS-gated tests.

- [x] **Step 6: Run formatting and broader Trail verification**

Run:

```sh
cargo fmt --check
cargo test -p trail --lib
```

Expected: Formatting exits 0 and all Trail library tests pass.

- [x] **Step 7: Review and commit the isolated fix**

Run:

```sh
git diff --check
git diff -- trail/src/db/lane/workdir/view_core.rs
git add trail/src/db/lane/workdir/view_core.rs docs/superpowers/plans/2026-07-12-nfs-noop-setattr-checkpoint.md
git commit -m "fix: ignore empty workspace setattr"
```

Expected: The commit contains only the implementation plan, regression test, and no-op guard.

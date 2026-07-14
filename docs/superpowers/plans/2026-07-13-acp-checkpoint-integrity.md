# ACP Checkpoint Integrity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure ACP never reports a materialized edit turn as successful and unchanged when Trail could not create its checkpoint, and prevent ACP startup against legacy live roots that cannot accept mutations.

**Architecture:** Add a read-only live-root path-index preflight to the Trail core and invoke it from ACP startup and ACP doctor. At prompt completion, convert workdir-record failures into durable turn evidence and a failed outcome while preserving the dirty workdir for recovery. Keep successful prompt-end reconciliation as the authoritative one-operation checkpoint for materialized edits.

**Tech Stack:** Rust 2024, Trail SQLite/object store, ACP JSON-RPC relay, Cargo unit and integration tests.

## Global Constraints

- Do not auto-rebuild legacy indexes in an ACP hot path; return `PATH_INDEX_REQUIRED` with `trail index rebuild` recovery guidance.
- Never discard a workdir checkpoint error.
- Never set `outcome.no_changes=true` when workdir reconciliation observed edits but checkpointing failed.
- Preserve existing unrelated worktree changes.
- Use test-first red-green cycles and verify the real subprocess relay path.

---

### Task 1: Live-root path-index preflight

**Files:**
- Modify: `trail/src/db/storage/files.rs`
- Modify: `trail/src/acp.rs`
- Modify: `trail/src/cli/command/handler/acp.rs`
- Test: `trail/src/acp.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Produces: `Trail::ensure_live_path_invariant_indexes(&self) -> Result<()>`.
- Consumes: immutable live branch/lane refs and `WorktreeRoot.case_fold_map_root`.

- [x] **Step 1: Write failing tests**

Add a coordinator test that converts a non-empty current root to a legacy root and expects `CaptureCoordinator::new` to return `Error::PathIndexRequired`. Add a CLI test expecting ACP doctor to include a failed `path_invariant_index` check and overall failed status.

- [x] **Step 2: Verify red**

Run:

```bash
cargo test -p trail acp_rejects_legacy_live_root_before_start -- --exact
cargo test -p trail --test e2e acp_doctor_reports_legacy_path_index -- --exact
```

Expected: the coordinator currently starts and doctor lacks the check.

- [x] **Step 3: Implement minimal preflight**

Add a read-only method that loads distinct live `refs/branches/*` and `refs/lanes/*` roots and returns:

```rust
Error::PathIndexRequired(
    "live ref `<ref>` has a legacy root with no case-fold index; run `trail index rebuild`"
        .to_string(),
)
```

Call it from `CaptureCoordinator::new` before the upstream process starts. Add the same result as ACP doctor's `path_invariant_index` check and mark the report failed on error.

- [x] **Step 4: Verify green**

Re-run both exact tests and the existing modern-workspace ACP doctor test.

### Task 2: Durable checkpoint-failure outcome

**Files:**
- Modify: `trail/src/acp.rs`
- Test: `trail/src/acp.rs`

**Interfaces:**
- Produces: an `acp_workdir_checkpoint_failed` turn event with stable Trail error code and message.
- Produces: failed turn envelope with `checkpoint=null`, `no_changes=false`, and a checkpoint error summary.

- [x] **Step 1: Write the failing regression test**

Start a materialized ACP session on a modern root, create a file in its lane workdir, replace the lane ref root with an equivalent legacy root after startup, and complete the prompt. Assert the turn is failed, does not claim `no_changes`, contains no checkpoint, retains the dirty file, and records `acp_workdir_checkpoint_failed` with `PATH_INDEX_REQUIRED`.

- [x] **Step 2: Verify red**

Run:

```bash
cargo test -p trail acp_checkpoint_failure_is_durable_and_not_no_changes -- --exact
```

Expected: current code discards the record error and emits completed/no-changes.

- [x] **Step 3: Implement minimal failure handling**

Replace both ignored `record_lane_workdir_for_turn` results with a shared helper. On failure, store the diagnostic event, emit a stderr warning, finalize the turn as failed, preserve `checkpoint=null`, force `no_changes=false`, and retain the original upstream stop reason. Apply the same helper during relay closeout.

- [x] **Step 4: Verify green**

Run the exact regression test plus existing checkpoint/no-change envelope tests.

### Task 3: End-to-end relay verification and Lore recovery

**Files:**
- Modify: `trail/tests/e2e.rs`
- Modify only through Trail maintenance: `/Users/haipingfu/Github/lore/.trail`

**Interfaces:**
- Consumes: compiled `trail` binary and fake ACP subprocess.
- Produces: one `LaneRecord` checkpoint tied to a completed prompt with changed paths and clean lane workdir.

- [x] **Step 1: Strengthen the subprocess success assertion**

Extend `acp_relay_captures_session_prompt_mcp_and_workdir_edits` to assert a non-null outcome checkpoint, `no_changes=false`, a `workdir_recorded` event, and clean materialized workdir state.

- [x] **Step 2: Run focused and broad verification**

Run:

```bash
cargo fmt --check
cargo test -p trail acp_
cargo test -p trail --test e2e acp_relay_captures_session_prompt_mcp_and_workdir_edits -- --exact
cargo clippy -p trail --all-targets -- -D warnings
cargo build -p trail --release
```

- [x] **Step 3: Back up and migrate Lore**

Create and verify a Trail backup, run `trail index rebuild`, and re-run the read-only lane-record preview for `agent-claude-code-af2b5ec2c3f5`.

- [x] **Step 4: Recover and verify the dirty task state**

Record the ten existing files into a new recoverable lane operation, then verify lane status, timeline, transcript diagnostics, checkpoint visibility, and rewind preview. Do not apply the lane to Git.

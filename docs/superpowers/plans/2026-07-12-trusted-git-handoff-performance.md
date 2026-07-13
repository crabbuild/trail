# Trusted Git Handoff Performance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make high-level Trail agent apply use only mapped changed-path Git export, fail quickly when baseline trust is missing, and prove the behavior at repository scale.

**Architecture:** Add stable Git handoff errors and an explicit `GitExportPolicy` that separates general full-snapshot export from high-level mapped-delta apply. Gather Git identity once, perform at most one tracked-worktree status query, pass the resulting state into export, and expose export-mode counters in `GitExportReport`. Extend the real CLI scale harness so high-level dry-run and actual apply exercise Git publication rather than only internal lane merge.

**Tech Stack:** Rust 2021, rusqlite, prolly maps, Git command-line plumbing, shell scale harness, GitHub Actions.

## Global Constraints

- High-level agent apply must never select full snapshot export.
- Missing HEAD/base-root trust must fail before loading all root files or writing Git objects.
- Mapped apply work must scale with changed paths `k`, not repository paths `N`.
- Dry-run and actual apply SHALL each ask Git for tracked-worktree cleanliness at most once.
- Dry-run must not insert or repair Git mappings.
- Delta commit construction must preserve Git-tracked paths Trail does not model, including symlinks.
- General `trail git export -m` remains the explicit O(N)-capable full-snapshot operation.
- Tests must run on macOS, Linux, and Windows where the existing Git test gates permit them.

---

### Task 1: Stable Git handoff failures

**Files:**
- Modify: `trail/src/error.rs`
- Modify: `trail/src/cli/command/handler/errors.rs`

**Interfaces:**
- Consumes: existing `Error::code()`, `Error::exit_code()`, and `diagnostic_for_error` conventions.
- Produces: `Error::GitMappingRequired`, `Error::GitHeadChanged`, `Error::GitWorktreeDirty`, and `Error::GitDeltaExportRequired` with stable machine codes.

- [ ] **Step 1: Add failing error-contract tests**

Add a `#[cfg(test)]` module to `trail/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_handoff_errors_have_stable_codes_and_exit_status() {
        let errors = [
            (Error::GitMappingRequired("missing mapping".into()), "GIT_MAPPING_REQUIRED"),
            (Error::GitHeadChanged("head changed".into()), "GIT_HEAD_CHANGED"),
            (Error::GitWorktreeDirty("tracked changes".into()), "GIT_WORKTREE_DIRTY"),
            (Error::GitDeltaExportRequired("mapped delta required".into()), "GIT_DELTA_EXPORT_REQUIRED"),
        ];
        for (error, code) in errors {
            assert_eq!(error.code(), code);
            assert_eq!(error.exit_code(), 10);
        }
    }
}
```

Extend the existing diagnostic tests in `trail/src/cli/command/handler/errors.rs`:

```rust
#[test]
fn missing_git_mapping_recommends_explicit_reconciliation() {
    let diagnostic = diagnostic_for_error(&Error::GitMappingRequired("missing".into()));
    assert_eq!(diagnostic.code, "GIT_MAPPING_REQUIRED");
    assert_eq!(diagnostic.recovery.unwrap().command, "trail git import-update");
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p trail git_handoff_errors_have_stable_codes_and_exit_status --lib
```

Expected: compilation fails because the four error variants do not exist.

- [ ] **Step 3: Implement the error variants and diagnostics**

Add to `Error`:

```rust
#[error("Git baseline mapping is required: {0}")]
GitMappingRequired(String),
#[error("Git HEAD changed during handoff: {0}")]
GitHeadChanged(String),
#[error("Git tracked worktree is dirty: {0}")]
GitWorktreeDirty(String),
#[error("mapped Git delta export is required: {0}")]
GitDeltaExportRequired(String),
```

Map them to the exact codes from Step 1 and exit code `10`. Add dedicated diagnostics; `GitMappingRequired` must recommend `trail git import-update`, `GitHeadChanged` must recommend `git status --short`, and `GitWorktreeDirty` must recommend `git status --short`.

- [ ] **Step 4: Run focused and diagnostic tests and verify GREEN**

Run:

```bash
cargo test -p trail git_handoff_errors_have_stable_codes_and_exit_status --lib
cargo test -p trail missing_git_mapping_recommends_explicit_reconciliation --bin trail
```

Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add trail/src/error.rs trail/src/cli/command/handler/errors.rs
git commit -m "feat: add stable git handoff errors"
```

---

### Task 2: Explicit mapped-delta export policy

**Files:**
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/merge/git_export.rs`
- Modify: `trail/src/model/reports/worktree.rs`
- Test: `trail/src/db/merge/git_export.rs`

**Interfaces:**
- Consumes: `git_clean_head_matches_root_mapping`, `diff_root_file_maps`, `git_write_tree_from_head_delta`, and `git_write_tree`.
- Produces: `GitExportPolicy::{RequireMappedDelta, AllowFullSnapshot}`, `Trail::git_export_commit_mapped`, and `GitHandoffMetricsReport` embedded in Git export and agent apply reports.

- [ ] **Step 1: Write a failing missing-mapping export test**

Add a module test to `trail/src/db/merge/git_export.rs` that initializes Trail from the working tree inside a clean committed Git repository, records one Trail change, restores the Git worktree, and requests mapped export:

```rust
#[cfg(test)]
mod tests {
use super::*;

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "git failed: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn mapped_git_export_requires_preexisting_clean_mapping() {
    if Command::new("git").arg("--version").output().is_err() { return; }
    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("README.md"), "one\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    let init = Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let record = db.record(Some("main"), Some("change".into()), Actor::human(), false).unwrap();
    run_git(temp.path(), &["checkout", "--", "README.md"]);
    let range = format!("{}..{}", init.operation.0, record.operation.unwrap().0);
    let err = db.git_export_commit_mapped(&range, "mapped", None).unwrap_err();
    assert!(matches!(err, Error::GitMappingRequired(_)));
    assert!(db.git_mappings(10).unwrap().is_empty());
}
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p trail mapped_git_export_requires_preexisting_clean_mapping --lib -- --exact
```

Expected: compilation fails because `git_export_commit_mapped` does not exist.

- [ ] **Step 3: Add policy and report types**

Add internal types in `trail/src/db/mod.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitExportPolicy {
    RequireMappedDelta,
    AllowFullSnapshot,
}
```

Add a serializable report and embed it in `GitExportReport`:

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GitHandoffMetricsReport {
    pub export_mode: String,
    pub changed_path_count: u64,
    pub blob_write_count: u64,
    pub tracked_status_count: u64,
    pub full_root_file_count: u64,
}

pub performance: GitHandoffMetricsReport,
```

Refactor `git_export_commit` into a shared implementation:

```rust
pub fn git_export_commit(&mut self, range: &str, message: &str) -> Result<GitExportReport> {
    let state = self.current_git_state()?.ok_or_else(|| Error::Git("not a Git worktree".into()))?;
    self.git_export_commit_with_state(range, message, state, GitExportPolicy::AllowFullSnapshot)
}

pub(crate) fn git_export_commit_mapped(
    &mut self,
    range: &str,
    message: &str,
    state: Option<GitState>,
) -> Result<GitExportReport> {
    let state = match state {
        Some(state) => state,
        None => self.current_git_state()?
            .ok_or_else(|| Error::Git("not a Git worktree".into()))?,
    };
    self.git_export_commit_with_state(range, message, state, GitExportPolicy::RequireMappedDelta)
}
```

Inside `git_export_commit_with_state`, query `git_clean_head_matches_root_mapping` directly. Under `RequireMappedDelta`, return `Error::GitMappingRequired` on a miss; do not call `ensure_git_clean_head_root_mapping`, `load_root_files`, or `git_write_tree`. Under `AllowFullSnapshot`, retain the existing full export fallback.

- [ ] **Step 4: Run mapped and general export tests and verify GREEN**

Run:

```bash
cargo test -p trail mapped_git_export_requires_preexisting_clean_mapping --lib -- --exact
cargo test -p trail --test e2e git_export_with_message_creates_commit_object_and_mapping -- --exact
cargo test -p trail --test e2e git_export_uses_clean_head_mapping_for_delta_commit -- --exact
```

Expected: all three tests pass; mapped export reports `performance.export_mode == "mapped_delta"`, while an unmapped general export reports `performance.export_mode == "full_snapshot"`.

- [ ] **Step 5: Commit**

```bash
git add trail/src/db/mod.rs trail/src/db/merge/git_export.rs trail/src/model/reports/worktree.rs
git commit -m "perf: require mapped delta git export"
```

---

### Task 3: One Git status query in high-level apply

**Files:**
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: `trail/src/db/storage/git.rs`
- Modify: `trail/src/db/agent.rs`
- Modify: `trail/src/model/lane/activity.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: `GitState`, `git_export_commit_mapped`, `git_fast_forward`, and existing agent apply reports.
- Produces: `GitIdentity { head, branch }`, `current_git_identity`, `tracked_git_state`, per-handoff structural metrics, and high-level apply that passes one state snapshot into export.

- [ ] **Step 1: Write a failing structural-metrics regression test**

Add this helper in `trail/tests/e2e.rs` and use `InitImportMode::GitTracked` for mapped tests or `InitImportMode::WorkingTree` for the missing-mapping test:

```rust
fn ready_agent_lane_with_mode(mode: InitImportMode) -> (tempfile::TempDir, Trail) {
    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    Trail::init(temp.path(), "main", mode, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("apply-bot", Some("main"), false, None, None).unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "one changed path",
        "edits": [{"op": "write", "path": "AGENT.md", "content": "agent change\n"}]
    })).unwrap();
    apply_lane_patch_at_head(&mut db, "apply-bot", patch).unwrap();
    db.agent_mark_reviewed("apply-bot", None).unwrap();
    (temp, db)
}
```

Use it in a new test:

```rust
#[test]
fn agent_apply_reports_one_tracked_status_query() {
    if !git_available() { return; }
    let (temp, mut db) = ready_agent_lane_with_mode(InitImportMode::GitTracked);
    let dry_run = db.agent_apply("apply-bot", true, None).unwrap();
    assert_eq!(dry_run.performance.tracked_status_count, 1);
    assert_eq!(dry_run.performance.full_root_file_count, 0);
    assert_eq!(dry_run.performance.export_mode, "mapped_delta");
    drop(temp);
}
```

Add a second test using a fresh `ready_agent_lane` fixture and actual apply; assert the same status/full-root counts, `changed_path_count == 1`, and `blob_write_count == 1`.

Add a third test using a fresh fixture initialized with `InitImportMode::WorkingTree`. `agent_apply("apply-bot", true, None)` must return `Error::GitMappingRequired`, `git_mappings(10)` must remain empty, and Git HEAD must remain unchanged.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p trail --test e2e agent_apply_reports_one_tracked_status_query -- --exact
```

Expected: compilation fails because `AgentApplyReport.performance` does not exist.

- [ ] **Step 3: Split cheap identity from tracked cleanliness**

Add:

```rust
#[derive(Debug, Clone)]
pub(crate) struct GitIdentity {
    head: String,
    branch: Option<String>,
}
```

Add a `Copy + Default` internal counter record and one `Cell<GitHandoffMetrics>` field to `Trail`; initialize it in `Trail::open_at`:

```rust
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GitHandoffMetrics {
    export_mode: GitExportMode,
    changed_path_count: u64,
    blob_write_count: u64,
    tracked_status_count: u64,
    full_root_file_count: u64,
}
```

Define `GitExportMode` with variants `Unknown`, `MappedDelta`, and `FullSnapshot`; implement `Default` as `Unknown` and convert the variants to the exact report strings `unknown`, `mapped_delta`, and `full_snapshot`.

Reset it once at the start of `agent_apply` and general Git export. Increment `tracked_status_count` only where Trail invokes tracked-only status, `blob_write_count` in blob hashing, and `full_root_file_count` only in full snapshot export. Convert it into `GitHandoffMetricsReport` for every `AgentApplyReport` and `GitExportReport` constructor.

Implement `current_git_identity` with `rev-parse --verify HEAD` and
`symbolic-ref --quiet --short HEAD`; it must not run `git status`. Implement
`tracked_git_state(identity)` with the single tracked-only status command.

Refactor `agent_apply`:

1. Resolve `GitIdentity` once.
2. Require the HEAD/base-root mapping with `git_clean_head_matches_root_mapping`.
3. For dry-run, obtain one `GitState` before returning the plan.
4. For actual apply, finish Trail recording/merge first, then obtain one
   `GitState`, verify its HEAD equals `GitIdentity.head`, and pass it to
   `git_export_commit_mapped`.
5. Remove `current_git_branch` and the full-root fallback in
   `ensure_git_head_matches_root`.

- [ ] **Step 4: Add HEAD-race and dirty-worktree tests**

Add focused tests that exercise the internal validation helper:

```rust
assert!(matches!(
    validate_git_publication_state("old", &GitState { head: Some("new".into()), dirty: false }),
    Err(Error::GitHeadChanged(_))
));
assert!(matches!(
    validate_git_publication_state("head", &GitState { head: Some("head".into()), dirty: true }),
    Err(Error::GitWorktreeDirty(_))
));
```

- [ ] **Step 5: Run the apply regression set and verify GREEN**

Run:

```bash
cargo test -p trail --test e2e agent_apply_reports_one_tracked_status_query -- --exact
cargo test -p trail --test e2e agent_start_custom_command_applies_task_to_git_with_guided_flow -- --exact
cargo test -p trail git_publication_state --lib
```

Expected: one Trail-issued status query per dry-run or actual apply; all apply safety behavior remains green.

- [ ] **Step 6: Commit**

```bash
git add trail/src/db/mod.rs trail/src/db/core/init.rs trail/src/db/storage/git.rs trail/src/db/agent.rs trail/src/model/lane/activity.rs trail/tests/e2e.rs
git commit -m "perf: reuse git state during agent apply"
```

---

### Task 4: Characterize Git-only path preservation and dry-run immutability

**Files:**
- Modify: `trail/tests/e2e.rs`
- Modify: `trail/src/db/merge/git_export.rs`
- Modify: `trail/src/db/agent.rs`

**Interfaces:**
- Consumes: mapped-delta export and high-level agent apply from Tasks 2-3.
- Produces: end-to-end evidence that symlinks/unmanaged Git paths survive and dry-run does not create mappings or Git objects.

- [ ] **Step 1: Add the symlink-preservation characterization test**

On Unix, create and commit `target.md` plus tracked symlink `link.md`, initialize Trail with `GitTracked`, create a ready no-materialize agent lane that adds `README.md`, and apply it. Assert:

```rust
assert_eq!(git_output(temp.path(), &["show", "HEAD:link.md"]), "target.md");
assert_eq!(git_output(temp.path(), &["show", "HEAD:target.md"]), "target\n");
assert_eq!(git_output(temp.path(), &["show", "HEAD:README.md"]), "agent change");
```

- [ ] **Step 2: Run the characterization test**

Run:

```bash
cargo test -p trail --test e2e agent_apply_preserves_git_only_symlinks -- --exact
```

Expected: PASS through mapped-delta export. A failure proves the mapped temporary-index implementation does not preserve the Git HEAD tree and must be corrected before continuing.

- [ ] **Step 3: Write the dry-run immutability test**

Capture before/after values for Git HEAD, `.git/index`, `git_mappings` count, and Git object count around `agent apply --dry-run`. Assert all are unchanged.

- [ ] **Step 4: Run both tests and verify GREEN**

Run:

```bash
cargo test -p trail --test e2e agent_apply_preserves_git_only_symlinks -- --exact
cargo test -p trail --test e2e agent_apply_dry_run_writes_no_git_or_mapping_state -- --exact
```

Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add trail/tests/e2e.rs trail/src/db/merge/git_export.rs trail/src/db/agent.rs
git commit -m "test: lock down mapped agent handoff"
```

---

### Task 5: High-level Git apply scale harness and structural gates

**Files:**
- Modify: `scripts/cli-scale-bench.sh`
- Modify: `scripts/check-cli-scale-thresholds.py`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/scale.yml`
- Modify: `docs/guides/performance-and-scale-benchmarks.md`

**Interfaces:**
- Consumes: JSON `GitExportReport.performance.export_mode`, `changed_path_count`, and `blob_write_count` from Task 2.
- Produces: result rows `agent_git_apply_dry_run`, `agent_git_apply`, and `agent_git_apply_missing_mapping`; metric keys for export mode and changed/blob counts.

- [ ] **Step 1: Extend the threshold checker test-first**

Add Python unit coverage (new file `scripts/test_check_cli_scale_thresholds.py`) proving string equality gates and integer ceilings can be checked together:

```python
import importlib.util
import pathlib
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).with_name("check-cli-scale-thresholds.py")
SPEC = importlib.util.spec_from_file_location("scale_thresholds", SCRIPT)
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)

class ThresholdMetricTests(unittest.TestCase):
    def test_reads_structural_string_metrics(self):
        with tempfile.TemporaryDirectory() as directory:
            metrics = pathlib.Path(directory) / "metrics.tsv"
            metrics.write_text(
                "agent_git_export_mode\tmapped_delta\n"
                "agent_git_changed_paths\t1\n"
            )
            parsed = module.read_metric_values(metrics)
            self.assertEqual(parsed["agent_git_export_mode"], "mapped_delta")
            self.assertEqual(parsed["agent_git_changed_paths"], 1.0)
```

Run `python3 -m unittest scripts/test_check_cli_scale_thresholds.py` and verify RED because only numeric metrics are supported.

- [ ] **Step 2: Implement structural metric parsing and gates**

Add `--metric-equals key=value` support while preserving existing numeric `key=max_value` behavior. Run the unit test and verify GREEN.

- [ ] **Step 3: Add a committed Git high-level agent scenario**

In `cli-scale-bench.sh`, create a no-materialize lane from the Git fixture's mapped `main`, apply a structured patch touching `k = max(1, min(100, files / 1000))` files, mark the task reviewed/ready using the same gates as existing agent fixtures, then time:

```bash
run_timed "$scale" agent_git_apply_dry_run "$BIN" --workspace "$GIT_REPO" --json agent apply latest --dry-run
run_timed "$scale" agent_git_apply "$BIN" --workspace "$GIT_REPO" --json agent apply latest
```

Parse the actual apply JSON into:

```text
agent_git_export_mode<TAB>mapped_delta
agent_git_changed_paths<TAB>${changed_paths}
agent_git_blob_writes<TAB>${blob_writes}
```

Create a second `InitImportMode::WorkingTree` Git fixture without a mapping and assert `agent apply --dry-run` exits with `GIT_MAPPING_REQUIRED` within the timed row `agent_git_apply_missing_mapping`.

- [ ] **Step 4: Add CI and scheduled gates**

CI 1k ceilings:

```text
agent_git_apply_dry_run=15
agent_git_apply=20
agent_git_apply_missing_mapping=5
```

Scheduled ceilings:

```text
100k: dry-run=2, apply=3, missing=1
1M: dry-run=5, apply=8, missing=1
```

For every scale, assert `agent_git_export_mode=mapped_delta`, `agent_git_changed_paths=k`, and `agent_git_blob_writes<=k`.

- [ ] **Step 5: Run the smoke harness**

Run:

```bash
TRAIL_SCALE_FILES=1000 TRAIL_SCALE_MATERIALIZED=0 TRAIL_SCALE_BACKUP=0 TRAIL_SCALE_DAEMON=0 TRAIL_SCALE_GIT_IMPORT=1 make bench-cli-scale
```

Expected: all new rows exit `0`; structural metric gates pass.

- [ ] **Step 6: Document the mapped hot path**

Update the performance guide with the new rows, the explicit reconciliation requirement, and the rule that missing mapping is a fast error rather than a correctness fallback.

- [ ] **Step 7: Commit**

```bash
git add scripts/cli-scale-bench.sh scripts/check-cli-scale-thresholds.py scripts/test_check_cli_scale_thresholds.py .github/workflows/ci.yml .github/workflows/scale.yml docs/guides/performance-and-scale-benchmarks.md
git commit -m "perf: gate high-level git apply at scale"
```

---

### Task 6: Full verification and release evidence

**Files:**
- Modify only if verification exposes defects in files already listed above.

**Interfaces:**
- Consumes: all production and test interfaces from Tasks 1-5.
- Produces: passing formatting, lint, focused E2E, workspace tests, smoke scale evidence, and a clean diff.

- [ ] **Step 1: Format and lint**

Run:

```bash
cargo fmt --all --check
cargo clippy -p trail --all-targets -- -D warnings
```

Expected: exit `0` with no warnings.

- [ ] **Step 2: Run focused Git and agent tests**

Run:

```bash
cargo test -p trail --test e2e git_export -- --nocapture
cargo test -p trail --test e2e agent_apply -- --nocapture
cargo test -p trail --test e2e agent_start_custom_command_applies_task_to_git_with_guided_flow -- --exact
```

Expected: all matching tests pass.

- [ ] **Step 3: Run the Trail package test suite**

Run:

```bash
cargo test -p trail
```

Expected: all tests pass.

- [ ] **Step 4: Run 100k mapped handoff acceptance**

Run:

```bash
TRAIL_SCALE_FILES=100000 TRAIL_SCALE_MATERIALIZED=0 TRAIL_SCALE_BACKUP=0 TRAIL_SCALE_DAEMON=1 TRAIL_SCALE_GIT_IMPORT=1 make bench-cli-scale
```

Expected: mapped dry-run <= 2 seconds, actual apply <= 3 seconds, missing mapping <= 1 second, and all structural gates pass.

- [ ] **Step 5: Verify diff integrity**

Run:

```bash
git diff --check
git status --short
```

Expected: no whitespace errors; status contains only intentional files or is clean after commits.

- [ ] **Step 6: Commit any verification fixes**

If verification required code changes, first add a failing regression test for each defect, then commit only the affected files:

```bash
git add trail/src/error.rs trail/src/cli/command/handler/errors.rs trail/src/db/mod.rs trail/src/db/core/init.rs trail/src/db/storage/git.rs trail/src/db/merge/git_export.rs trail/src/db/agent.rs trail/src/model/reports/worktree.rs trail/src/model/lane/activity.rs trail/tests/e2e.rs scripts/cli-scale-bench.sh scripts/check-cli-scale-thresholds.py scripts/test_check_cli_scale_thresholds.py .github/workflows/ci.yml .github/workflows/scale.yml docs/guides/performance-and-scale-benchmarks.md
git commit -m "fix: close git handoff verification gaps"
```

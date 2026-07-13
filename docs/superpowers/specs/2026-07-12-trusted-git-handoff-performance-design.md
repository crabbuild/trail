# Trusted Git Handoff Performance Design

**Status:** Approved for implementation

## Objective

Make high-level Trail agent apply predictable and safe for repositories with
10,000, 100,000, and 1,000,000 files. A mapped apply must scale with the number
of changed paths `k`, not the total Trail or Git path count `N`. Missing trust
must produce an explicit reconciliation requirement rather than an implicit
full-tree scan or export.

This is the first slice of the production performance program. Later slices
will specify watcher-backed changed-path recording and persistent path-invariant
indexes. Those slices build on the guarantees established here but are not
required to remove the catastrophic Git handoff fallback.

## Evidence

The current high-level apply path has four measurable problems:

1. `agent_apply` obtains Git state through `current_git_branch`, obtains it
   again directly, and obtains it a third time inside `git_export_commit`.
   Each state lookup runs tracked-only `git status`.
2. A missing Git-to-Trail mapping causes `ensure_git_head_matches_root` to load
   the complete Trail root and call `git_write_tree`.
3. `git_write_tree` launches one `git hash-object` process per Trail file.
4. The scale harness covers structured lane patching and internal lane merge,
   but not high-level agent apply to Git.

The local Linux fixture contains 94,837 Git-tracked paths, a 94,506-path Trail
root, and no Git mapping. It therefore selects the catastrophic fallback even
for a one-file agent change.

## Performance Contract

For a clean Git worktree whose current HEAD and Trail base root have a trusted
mapping:

- High-level dry-run and actual apply SHALL NOT load all Trail root paths or
  files.
- High-level dry-run and actual apply SHALL NOT run full-tree Git export.
- Trail-side merge and export work SHALL be proportional to changed paths and
  the affected prolly-tree nodes.
- Actual apply SHALL ask Git for tracked-worktree cleanliness at most once.
- Dry-run SHALL ask Git for tracked-worktree cleanliness at most once.
- Git commit construction SHALL preserve Git-tracked paths that Trail does not
  model, including symlinks, by starting from the mapped Git HEAD tree.
- Dry-run SHALL not insert mappings or otherwise reconcile state.

When the current Git HEAD and Trail base root do not have a trusted mapping:

- High-level apply SHALL fail before root materialization or Git object writes.
- The error SHALL distinguish missing baseline trust from a dirty worktree or
  a divergent HEAD.
- The error SHALL recommend `trail git import-update` as the explicit
  reconciliation workflow.
- Trail SHALL NOT silently scan or hash `N` files to manufacture trust.

Full snapshot export remains available only through an explicitly selected
general Git export mode. High-level agent apply always requires mapped delta
export.

## Architecture

### Git handoff context

Introduce one internal Git handoff context owned by the Git handoff module. It
contains the workspace identity, branch, expected HEAD, clean/dirty result,
Trail base change/root, and the matching mapping. Callers do not independently
rediscover those facts.

Planning resolves branch and HEAD without a worktree scan. Dry-run performs its
single clean-state check while creating the plan. Actual apply defers its single
clean-state check until immediately before publication and verifies that HEAD
still equals the expected parent.

### Export policy

Git commit export receives an explicit policy:

- `RequireMappedDelta`: require the expected HEAD/base-root mapping and update a
  temporary Git index from only the root diff.
- `AllowFullSnapshot`: permit a general-purpose full Trail snapshot export when
  the caller explicitly chose that operation.

`agent_apply` and `agent_finish` always use `RequireMappedDelta`. The existing
`trail git export -m` command uses `AllowFullSnapshot`; invoking that explicit
general export command is the user's selection of the potentially O(N) mode.

### Delta construction

Mapped delta export continues to:

1. Compute a structural root diff into changed-path maps.
2. Populate a temporary index from the expected Git HEAD.
3. Write or remove only changed entries.
4. Write the new tree and commit with the expected HEAD as parent.

Git plumbing for multiple changed paths is batched behind the handoff module.
The initial implementation may retain one blob-hash call per changed file, but
it must never issue calls for unchanged files. A later optimization may replace
the adapter with fast-import or an in-process Git implementation without
changing callers.

### Publication safety

Immediately before publication, actual apply verifies:

- Git still points at the expected HEAD.
- The tracked worktree is clean.
- The mapped base root still matches the Trail target base.

Publication uses Git's fast-forward behavior and records the resulting
Git-to-Trail mapping only after success. A race or mismatch fails without moving
the Git ref or recording a successful mapping.

## Error Model

Add stable errors for:

- `git_mapping_required`: the HEAD/base-root trust record is missing.
- `git_head_changed`: Git HEAD changed after planning.
- `git_worktree_dirty`: tracked worktree changes block publication.
- `git_delta_export_required`: a high-level caller attempted full snapshot
  export.

Human diagnostics explain the condition and give one safe next command. JSON,
HTTP, and MCP surfaces preserve the stable code and do not parse human text.

## Benchmark and Regression Design

Extend the CLI scale harness with a committed Git fixture and a high-level agent
task that changes `k` paths. Measure both dry-run and actual apply at:

- `N = 10,000`, `100,000`, and `1,000,000`
- `k = 1` and `100`

Add three scenarios:

1. Mapped, clean high-level dry-run.
2. Mapped, clean high-level actual apply.
3. Missing mapping, which must fail quickly with `git_mapping_required` and no
   Git object or Trail mapping writes.

The benchmark records wall time, RSS, changed-path count, Git command count,
tracked-status count, full-root load count, blob-hash count, and export mode.
Production gates assert structural invariants before wall-time ceilings:

- `full_root_load_count = 0`
- `tracked_status_count <= 1`
- `blob_hash_count <= k`
- `changed_path_count = k`
- `export_mode = mapped_delta`

Reference-machine latency targets are:

- 100,000 files, `k = 1`: dry-run <= 2 seconds; actual apply <= 3 seconds.
- 1,000,000 files, `k = 1`: dry-run <= 5 seconds; actual apply <= 8 seconds.
- Missing mapping at either scale: fail <= 1 second after Trail opens.

CI keeps the 1,000-file smoke scenario. Scheduled scale automation enforces the
100,000- and 1,000,000-file structural gates and publishes latency results.

## Test Strategy

### Unit tests

- Mapped-delta policy rejects a missing mapping without loading a root.
- High-level policy cannot select full snapshot export.
- Git handoff context reuses one state snapshot.
- Publication rejects a changed HEAD.
- Dry-run writes no mapping.

### Integration tests

- A mapped one-file task creates a commit preserving unrelated Git files and
  symlinks.
- Missing mapping returns the stable reconciliation error and writes no Git
  objects.
- Dirty tracked files block apply.
- Actual apply records exactly one successful mapping after fast-forward.
- Repeated apply is idempotent.

### Scale tests

- High-level agent dry-run and actual apply run at every `N`/`k` matrix point.
- Structural counters prove the hot path did not fall back even when a broad
  wall-time threshold would otherwise pass.

## Rollout

1. Land failing regression tests and structural instrumentation.
2. Introduce export policy and make high-level apply require mapped delta.
3. Consolidate Git state discovery and final validation.
4. Add high-level scale scenarios and scheduled gates.
5. Update Git interop and performance documentation with the explicit
   reconciliation requirement.

The behavior change is intentionally strict. Repositories initialized without a
valid Git mapping must reconcile once before high-level apply. This converts an
unbounded surprise into a visible, recoverable setup requirement.

## Subsequent Performance Slices

The full production objective continues after this slice:

1. **Changed-path recording ledger:** persist watcher, COW-upper, and structured
   patch evidence so normal recording reads only changed paths; fall back only
   on explicit overflow reconciliation.
2. **Persistent path-invariant index:** add a case-fold path index to Worktree
   roots so adds and renames validate in `O(k log N)` without loading all paths.
3. **Git plumbing batching and end-to-end SLOs:** remove per-changed-file process
   overhead where measurements justify it and enforce cross-platform production
   release gates.

Each slice receives its own implementation plan and verification evidence. The
program is complete only after all slices pass their 10,000-, 100,000-, and
1,000,000-file gates.

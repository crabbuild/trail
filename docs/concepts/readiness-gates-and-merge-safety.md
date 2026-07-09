# Readiness Gates and Merge Safety

Trail treats lane merges as reviewable operations with explicit blockers.

## Readiness Checks

`trail lane readiness <name>` returns:

- `ready`: boolean.
- `status`: `ready` or `blocked`.
- `blockers`: merge-stopping issues.
- `warnings`: non-blocking issues.
- changed paths.
- materialized workdir state and changed paths.
- queued merges.
- pending approvals.
- conflicts.
- latest test and eval summaries.

## Common Blockers

Readiness can be blocked by:

- Pending approvals.
- Conflicts against the target.
- Dirty materialized lane workdirs that need recording.
- Missing required test or eval gates.
- Failed required suites.

The e2e suite verifies that direct lane merge and merge queue runs refuse blocked lanes.

## Test and Eval Gates

Run gates in a lane workdir:

```sh
trail lane test doc-bot --suite unit -- cargo test -p trail
trail lane eval doc-bot --suite policy-smoke --score 1.0 --threshold 1.0 -- cargo test -p trail
```

Required gates are configured with:

```sh
trail config set lane.require_test_gate true
trail config set lane.required_test_suites unit
trail config set lane.require_eval_gate true
trail config set lane.required_eval_suites policy-smoke
```

## Merge Paths

Preview the lane merge first:

```sh
trail merge-lane doc-bot --into main --dry-run
```

Use the queue for shared targets:

```sh
trail merge-queue add doc-bot --into main --priority 10
trail merge-queue run
```

Direct non-dry-run merges into the default branch require `--direct`.

## Code Facts Used

- Readiness: `crates/trail/src/db/lane/readiness.rs`
- Gate runner: `crates/trail/src/db/lane/gates`
- Merge queue: `crates/trail/src/db/merge`
- Tests: `merge_lane_and_queue_enforce_readiness_blockers`, `required_gate_config_blocks_merge_until_test_and_eval_pass`, `dirty_lane_workdir_must_be_recorded_before_merge`

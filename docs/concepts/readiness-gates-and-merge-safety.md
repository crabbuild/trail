# Readiness Gates and Merge Safety

CrabDB treats agent merges as reviewable operations with explicit blockers.

## Readiness Checks

`crabdb agent readiness <name>` returns:

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
- Dirty materialized agent workdirs that need recording.
- Missing required test or eval gates.
- Failed required suites.

The e2e suite verifies that direct agent merge and merge queue runs refuse blocked agents.

## Test and Eval Gates

Run gates in an agent workdir:

```sh
crabdb agent test doc-bot --suite unit -- cargo test -p crabdb
crabdb agent eval doc-bot --suite policy-smoke --score 1.0 --threshold 1.0 -- cargo test -p crabdb
```

Required gates are configured with:

```sh
crabdb config set agent.require_test_gate true
crabdb config set agent.required_test_suites unit
crabdb config set agent.require_eval_gate true
crabdb config set agent.required_eval_suites policy-smoke
```

## Merge Paths

Use direct merge for a one-off merge:

```sh
crabdb merge-agent doc-bot --into main --dry-run
crabdb merge-agent doc-bot --into main
```

Use the queue for shared targets:

```sh
crabdb merge-queue add doc-bot --into main --priority 10
crabdb merge-queue run
```

## Code Facts Used

- Readiness: `crates/crabdb/src/db/agent/readiness.rs`
- Gate runner: `crates/crabdb/src/db/agent/gates`
- Merge queue: `crates/crabdb/src/db/merge`
- Tests: `merge_agent_and_queue_enforce_readiness_blockers`, `required_gate_config_blocks_merge_until_test_and_eval_pass`, `dirty_agent_workdir_must_be_recorded_before_merge`


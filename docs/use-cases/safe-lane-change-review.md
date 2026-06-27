# Use Case: Safe Lane Change Review

Use readiness, gates, guardrails, and handoff reports to review lane work before merge.

## Review Flow

```sh
crabdb lane review doc-bot
crabdb lane status doc-bot
crabdb lane contribution doc-bot
crabdb lane readiness doc-bot
crabdb lane handoff doc-bot
crabdb lane diff doc-bot --patch --show-line-ids
```

## Gate Flow

```sh
crabdb lane test doc-bot --suite unit -- cargo test -p crabdb
crabdb lane eval doc-bot --suite policy --score 1.0 --threshold 1.0 -- cargo test -p crabdb
crabdb lane gates doc-bot --limit 20
```

## Approval Flow

```sh
crabdb guardrails check --lane doc-bot --action shell.exec --summary "run release tests"
crabdb approvals request doc-bot --action shell.exec --summary "run release tests"
crabdb approvals decide <approval-id> --decision approved
```

## Merge Flow

```sh
crabdb merge-lane doc-bot --into main --dry-run
crabdb merge-queue add doc-bot --into main
crabdb merge-queue run
```

## Code Facts Used

- Readiness/handoff: `crates/crabdb/src/db/lane/readiness.rs`
- Guardrails/approvals: `crates/crabdb/src/db/core/workspace/guardrails.rs`, `crates/crabdb/src/db/lane/control/approvals.rs`
- Tests: `merge_lane_and_queue_enforce_readiness_blockers`, `local_api_and_mcp_manage_human_approval_gates`

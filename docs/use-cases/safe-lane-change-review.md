# Use Case: Safe Lane Change Review

Use readiness, gates, guardrails, and handoff reports to review lane work before merge.

## Review Flow

```sh
trail lane review doc-bot
trail lane status doc-bot
trail lane contribution doc-bot
trail lane readiness doc-bot
trail lane handoff doc-bot
trail lane diff doc-bot --patch --show-line-ids
```

## Gate Flow

```sh
trail lane test doc-bot --suite unit -- cargo test -p trail
trail lane eval doc-bot --suite policy --score 1.0 --threshold 1.0 -- cargo test -p trail
trail lane gates doc-bot --limit 20
```

## Approval Flow

```sh
trail guardrails check --lane doc-bot --action shell.exec --summary "run release tests"
trail approvals request doc-bot --action shell.exec --summary "run release tests"
trail approvals decide <approval-id> --decision approved
```

## Merge Flow

```sh
trail merge-lane doc-bot --into main --dry-run
trail merge-queue add doc-bot --into main
trail merge-queue run
```

## Code Facts Used

- Readiness/handoff: `trail/src/db/lane/readiness.rs`
- Guardrails/approvals: `trail/src/db/core/workspace/guardrails.rs`, `trail/src/db/lane/control/approvals.rs`
- Tests: `merge_lane_and_queue_enforce_readiness_blockers`, `local_api_and_mcp_manage_human_approval_gates`

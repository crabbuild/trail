# Use Case: Safe Agent Change Review

Use readiness, gates, guardrails, and handoff reports to review agent work before merge.

## Review Flow

```sh
crabdb agent review doc-bot
crabdb agent status doc-bot
crabdb agent contribution doc-bot
crabdb agent readiness doc-bot
crabdb agent handoff doc-bot
crabdb agent diff doc-bot --patch --show-line-ids
```

## Gate Flow

```sh
crabdb agent test doc-bot --suite unit -- cargo test -p crabdb
crabdb agent eval doc-bot --suite policy --score 1.0 --threshold 1.0 -- cargo test -p crabdb
crabdb agent gates doc-bot --limit 20
```

## Approval Flow

```sh
crabdb guardrails check --agent doc-bot --action shell.exec --summary "run release tests"
crabdb approvals request doc-bot --action shell.exec --summary "run release tests"
crabdb approvals decide <approval-id> --decision approved
```

## Merge Flow

```sh
crabdb merge-agent doc-bot --into main --dry-run
crabdb merge-queue add doc-bot --into main
crabdb merge-queue run
```

## Code Facts Used

- Readiness/handoff: `crates/crabdb/src/db/agent/readiness.rs`
- Guardrails/approvals: `crates/crabdb/src/db/core/workspace/guardrails.rs`, `crates/crabdb/src/db/agent/control/approvals.rs`
- Tests: `merge_agent_and_queue_enforce_readiness_blockers`, `local_api_and_mcp_manage_human_approval_gates`

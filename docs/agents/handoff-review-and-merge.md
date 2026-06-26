# Handoff, Review, and Merge

Use review, handoff, and contribution reports to inspect agent state before merge or transfer it to another host.

## Review Packet

```sh
crabdb agent review doc-bot --limit 50
```

The review packet combines:

- Readiness report.
- Evidence summary counts.
- Changed paths.
- Latest and recent gates.
- Recent operations, sessions, events, trace spans, approvals, and conflicts.
- Next steps.

## Handoff Packet

```sh
crabdb agent handoff doc-bot --limit 50
```

The handoff report includes:

- Agent details.
- Readiness report.
- Current session context.
- Recent sessions.
- Recent events.
- Recent trace spans.
- Recent operations.
- Next steps.

## Contribution Packet

```sh
crabdb agent contribution doc-bot --limit 50
```

The contribution report focuses on status, changed paths, operations, sessions, recent events, and approvals.

## Review Checklist

```sh
crabdb agent review doc-bot
crabdb agent status doc-bot
crabdb agent readiness doc-bot
crabdb agent gates doc-bot
crabdb agent diff doc-bot --patch --show-line-ids
crabdb approvals list --agent doc-bot
```

Stop if readiness reports blockers.

## Merge

```sh
crabdb merge-agent doc-bot --into main --dry-run
crabdb merge-queue add doc-bot --into main
crabdb merge-queue run
```

Use the queue when multiple branches may target the same branch.

## Code Facts Used

- Handoff/readiness: `crates/crabdb/src/db/agent/readiness.rs`
- Contribution: `crates/crabdb/src/db/agent/identity.rs`
- Merge queue: `crates/crabdb/src/db/merge/queue.rs`
- Tests: `merge_queue_runs_agent_branch_into_main`, `merge_agent_and_queue_enforce_readiness_blockers`

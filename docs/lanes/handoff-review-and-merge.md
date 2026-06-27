# Handoff, Review, and Merge

Use review, handoff, and contribution reports to inspect lane state before merge or transfer it to another host.

## Review Packet

```sh
crabdb lane review doc-bot --limit 50
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
crabdb lane handoff doc-bot --limit 50
```

The handoff report includes:

- Lane details.
- Readiness report.
- Current session context.
- Recent sessions.
- Recent events.
- Recent trace spans.
- Recent operations.
- Next steps.

## Contribution Packet

```sh
crabdb lane contribution doc-bot --limit 50
```

The contribution report focuses on status, changed paths, operations, sessions, recent events, and approvals.

## Review Checklist

```sh
crabdb lane review doc-bot
crabdb lane status doc-bot
crabdb lane readiness doc-bot
crabdb lane gates doc-bot
crabdb lane diff doc-bot --patch --show-line-ids
crabdb approvals list --lane doc-bot
```

Stop if readiness reports blockers.

## Rewind

Use rewind when a lane branch has gone in the wrong direction and a known-good
change or root should become the new lane head:

```sh
crabdb lane rewind doc-bot --to <change-or-root> --record-current --sync-workdir
```

`--record-current` preserves the previous head under a `rewind/...` branch and
records dirty materialized workdir edits before the rewind when possible.
`--sync-workdir` refreshes a clean materialized workdir to the rewound head.

## Merge

```sh
crabdb merge-lane doc-bot --into main --dry-run
crabdb merge-queue add doc-bot --into main
crabdb merge-queue run
```

Use the queue when multiple branches may target the same branch.

## Code Facts Used

- Handoff/readiness: `crates/crabdb/src/db/lane/readiness.rs`
- Rewind: `crates/crabdb/src/db/lane/rewind.rs`
- Contribution: `crates/crabdb/src/db/lane/identity.rs`
- Merge queue: `crates/crabdb/src/db/merge/queue.rs`
- Tests: `merge_queue_runs_lane_branch_into_main`, `merge_lane_and_queue_enforce_readiness_blockers`

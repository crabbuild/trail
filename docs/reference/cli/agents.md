# CLI Reference: Agents

## Lifecycle and Review

```text
crabdb agent spawn <NAME> [--from <REF>] [--materialize[=true|false]] [--no-materialize] [--workdir <PATH>] [--paths <PATH>...] [--include-neighbors] [--provider <PROVIDER>] [--model <MODEL>]
crabdb agent list
crabdb agent show <NAME>
crabdb agent status <NAME>
crabdb agent review <NAME> [--limit <N>]
crabdb agent contribution <NAME> [--limit <N>]
crabdb agent readiness <NAME>
crabdb agent handoff <NAME> [--limit <N>]
crabdb agent rewind <NAME> --to <CHANGE|ROOT|REF> [--record-current] [--sync-workdir]
crabdb agent rm <NAME> [--force]
```

Default limits are 50 for review, contribution, and handoff.

`agent rewind` records an `AgentRewind` operation on the agent ref. With
`--record-current`, CrabDB first records dirty materialized workdir edits when
possible and preserves the pre-rewind head as `rewind/<agent>/<change>`. With
`--sync-workdir`, a clean materialized workdir is refreshed to the new head.

## Coordination and Messages

```text
crabdb agent claim <NAME> <PATH> [--ttl-secs <SECONDS>]
crabdb agent message <NAME> --role <ROLE> --text <TEXT> [--session <SESSION>]
```

Default claim TTL is 600 seconds.

Claims are advisory coordination signals. They help agents avoid stepping on the
same paths, but readiness, conflict detection, and merge review remain the hard
safety checks.

## Workdir Commands

```text
crabdb agent record <NAME> [-m <MESSAGE>]
crabdb agent watch <NAME> [-m <MESSAGE>] [--interval-secs <SECONDS>] [--debounce-ms <MS>] [--include-untracked] [--once]
crabdb agent read <NAME> <PATH> [--hydrate] [--no-hydrate] [--force] [--include-neighbors]
crabdb agent workdir <NAME>
crabdb agent sync-workdir <NAME> [--force] [--paths <PATH>...] [--include-neighbors]
crabdb agent checkout <NAME> [--force] [--dry-run] [--workdir <PATH>]
```

## Patches and Diffs

```text
crabdb agent apply-patch <NAME> --patch <FILE> [--allow-ignored]
crabdb agent diff <NAME> [--patch] [--show-line-ids]
crabdb agent timeline <NAME> [--limit <N>]
```

Agent timeline default limit is 30.

## Tests and Evals

```text
crabdb agent test <NAME> [--turn <TURN>] [--timeout-secs <SECONDS>] [--suite <SUITE>] [--score <SCORE>] [--threshold <THRESHOLD>] -- <COMMAND>...
crabdb agent eval <NAME> [--turn <TURN>] [--timeout-secs <SECONDS>] [--suite <SUITE>] [--score <SCORE>] [--threshold <THRESHOLD>] -- <COMMAND>...
crabdb agent gates <NAME> [--kind <KIND>] [--limit <N>]
```

Default timeout is 600 seconds. Gate history default limit is 50.

## Events, Turns, Runs, Traces

See [Sessions, approvals, anchors, and leases](sessions-approvals-anchors-and-leases.md) for session and approval commands. See agent workflow pages for detailed turn/run/trace examples.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/agent_args.rs`
- Agent handlers: `crates/crabdb/src/cli/command/handler/agent.rs`
- Reports: `crates/crabdb/src/model/reports/agent.rs`, `crates/crabdb/src/model/agent`

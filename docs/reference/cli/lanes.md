# CLI Reference: Lanes

## Lifecycle and Review

```text
crabdb lane spawn <NAME> [--from <REF>] [--materialize[=true|false]] [--no-materialize] [--workdir <PATH>] [--paths <PATH>...] [--include-neighbors] [--provider <PROVIDER>] [--model <MODEL>]
crabdb lane list
crabdb lane show <NAME>
crabdb lane status <NAME>
crabdb lane review <NAME> [--limit <N>]
crabdb lane contribution <NAME> [--limit <N>]
crabdb lane readiness <NAME>
crabdb lane handoff <NAME> [--limit <N>]
crabdb lane rewind <NAME> --to <CHANGE|ROOT|REF> [--record-current] [--sync-workdir]
crabdb lane rm <NAME> [--force]
```

Default limits are 50 for review, contribution, and handoff.

`lane rewind` records a `LaneRewind` operation on the lane ref. With
`--record-current`, CrabDB first records dirty materialized workdir edits when
possible and preserves the pre-rewind head as `rewind/<lane>/<change>`. With
`--sync-workdir`, a clean materialized workdir is refreshed to the new head.

## Coordination and Messages

```text
crabdb lane claim <NAME> <PATH> [--ttl-secs <SECONDS>]
crabdb lane message <NAME> --role <ROLE> --text <TEXT> [--session <SESSION>]
```

Default claim TTL is 600 seconds.

Claims are advisory coordination signals. They help lanes avoid stepping on the
same paths, but readiness, conflict detection, and merge review remain the hard
safety checks.

## Workdir Commands

```text
crabdb lane record <NAME> [-m <MESSAGE>]
crabdb lane watch <NAME> [-m <MESSAGE>] [--interval-secs <SECONDS>] [--debounce-ms <MS>] [--include-untracked] [--once]
crabdb lane read <NAME> <PATH> [--hydrate] [--no-hydrate] [--force] [--include-neighbors]
crabdb lane workdir <NAME>
crabdb lane sync-workdir <NAME> [--force] [--paths <PATH>...] [--include-neighbors]
crabdb lane checkout <NAME> [--force] [--dry-run] [--workdir <PATH>]
```

`lane sync-workdir --force` refuses no longer-dirty content as before, but when
it overwrites dirty materialized workdir files it prints `Rescue workdir:` with
the `.crabdb/lane-workdir-rescue/...` directory that contains copied dirty files
and a manifest.

## Patches and Diffs

```text
crabdb lane apply-patch <NAME> --patch <FILE> [--allow-ignored]
crabdb lane diff <NAME> [--patch] [--show-line-ids]
crabdb lane timeline <NAME> [--limit <N>]
```

Lane timeline default limit is 30.

## Tests and Evals

```text
crabdb lane test <NAME> [--turn <TURN>] [--timeout-secs <SECONDS>] [--suite <SUITE>] [--score <SCORE>] [--threshold <THRESHOLD>] -- <COMMAND>...
crabdb lane eval <NAME> [--turn <TURN>] [--timeout-secs <SECONDS>] [--suite <SUITE>] [--score <SCORE>] [--threshold <THRESHOLD>] -- <COMMAND>...
crabdb lane gates <NAME> [--kind <KIND>] [--limit <N>]
```

Default timeout is 600 seconds. Gate history default limit is 50.

## Events, Turns, Runs, Traces

See [Sessions, approvals, anchors, and leases](sessions-approvals-anchors-and-leases.md) for session and approval commands. See lane workflow pages for detailed turn/run/trace examples.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/lane_args.rs`
- Lane handlers: `crates/crabdb/src/cli/command/handler/lane.rs`
- Reports: `crates/crabdb/src/model/reports/lane.rs`, `crates/crabdb/src/model/lane`

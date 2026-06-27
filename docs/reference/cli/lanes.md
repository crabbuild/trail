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
crabdb lane refresh-preview <NAME> [--target <BRANCH>]
crabdb lane handoff <NAME> [--limit <N>]
crabdb lane rewind <NAME> --to <CHANGE|ROOT|REF> [--record-current] [--sync-workdir]
crabdb lane rm <NAME> [--force]
```

Default limits are 50 for review, contribution, and handoff.

`lane spawn --paths <PATH>...` creates a sparse materialized workdir scoped to
the selected paths. When `lane.enforce_sparse_paths=true`, lane patches and
workdir records become a hard write boundary: writes, deletes, and both sides
of renames must stay inside the selected paths. The sparse boundary is also
persisted with the lane, so enforcement survives a missing workdir sparse
manifest and can restore that manifest on the next valid sparse update.

`lane status` reports branch changes, dirty materialized workdir state, queued
merges, and `base_status`. When the lane's saved base is behind the workspace
default branch, text output also prints how many first-parent operations the
lane started behind that branch.

`lane refresh-preview` shows what would happen if the lane were refreshed onto
the target branch before merge. It reports how many first-parent operations the
lane base is behind the target, changed paths that would be brought in, and
conflicts that would need resolution.

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

Claims and explicit write leases are advisory by default. When
`lane.claim_enforcement` is set to `warn` or `reject`, patches and records
outside active write claims/leases emit policy warnings or fail before
recording. Read leases do not grant write permission under this policy.

## Workdir Commands

```text
crabdb lane record <NAME> [-m <MESSAGE>] [--preview]
crabdb lane watch <NAME> [-m <MESSAGE>] [--interval-secs <SECONDS>] [--debounce-ms <MS>] [--include-untracked] [--once]
crabdb lane read <NAME> <PATH> [--hydrate] [--no-hydrate] [--force] [--include-neighbors]
crabdb lane workdir <NAME>
crabdb lane sync-workdir <NAME> [--force] [--paths <PATH>...] [--include-neighbors]
crabdb lane checkout <NAME> [--force] [--dry-run] [--workdir <PATH>]
```

`lane sync-workdir --force` refuses no longer-dirty content as before, but when
it overwrites dirty materialized workdir files or replaces a non-directory file
at the lane workdir path it prints `Rescue workdir:` with the
`.crabdb/lane-workdir-rescue/...` directory that contains copied recoverable
files and a manifest.
Full workdir refreshes are staged in a hidden sibling directory and manifest
verified before replacing the visible workdir.

`lane record --preview` does not advance the lane. It reports changed paths,
ignored paths, risky workdir entries such as nested `.git`, nested `.crabdb`,
symlinks, hardlinks, or external mounts,
oversized changed files, and whether current lane policy would allow the record.

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

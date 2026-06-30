# CLI Reference: Lanes

Lanes are isolated CrabDB workspaces. Use them when you want to try work,
review changes, run validation, or coordinate multiple agents without changing
the main workspace immediately.

Use this page when you want to:

- Create or inspect a lane.
- Materialize a lane into a filesystem workdir.
- Record, review, diff, rewind, or remove lane work.
- Coordinate path ownership between agents.
- Run tests and evaluations in a lane.
- Inspect advanced turn, run, event, and trace records.

For code-agent tasks, prefer the higher-level `crabdb agent ...` workflow. Use
`crabdb lane ...` when you need lower-level lane control.

## Quick Start

### Create a virtual lane

```sh
crabdb lane spawn feature-docs
crabdb lane status feature-docs
```

A virtual lane stores branch state in CrabDB and does not create a filesystem
workdir.

### Create a lane with a workdir

```sh
crabdb lane spawn feature-docs --workdir-mode full-cow
crabdb lane workdir feature-docs
```

Use a materialized workdir when an editor, test command, or terminal agent needs
real files on disk.

### Create a sparse lane

```sh
crabdb lane spawn docs-only --paths docs README.md
crabdb lane hydrate docs-only docs/reference/cli/lanes.md
```

Sparse lanes materialize only selected paths. They are useful for bounded agent
or editor work.

### Review and apply later

```sh
crabdb lane record feature-docs -m "Draft lane docs"
crabdb lane review feature-docs
crabdb lane diff feature-docs --stat
crabdb lane diff feature-docs --patch
crabdb lane readiness feature-docs
```

`lane` commands do not replace the safe apply workflow. Use higher-level agent
commands, Git export, or lane-specific merge tooling when you are ready to land
work.

## Mental Model

| Term | Meaning |
| --- | --- |
| Lane | A branch-like workspace tracked by CrabDB |
| Base | The change or ref the lane started from |
| Head | The current latest change on the lane |
| Workdir | Optional materialized filesystem copy of the lane |
| Sparse lane | Lane workdir scoped to selected paths |
| Gate | A recorded test or eval result |
| Turn | Durable prompt/tool cycle linked to a lane |
| Trace | Structured span data for deeper debugging |

The common lifecycle is:

1. Spawn a lane from a ref.
2. Edit virtually, through patches, or in a workdir.
3. Record dirty workdir changes.
4. Review the lane diff and timeline.
5. Run tests or evals.
6. Rewind, refresh, hand off, or remove the lane.

## Command Families

| Goal | Commands |
| --- | --- |
| Create and inspect | `spawn`, `list`, `show`, `status`, `rm` |
| Review readiness | `review`, `contribution`, `readiness`, `refresh-preview`, `handoff` |
| Coordinate work | `claim`, `message` |
| Workdir editing | `record`, `watch`, `read`, `hydrate`, `workdir`, `sync-workdir`, `checkout` |
| Patches and history | `apply-patch`, `diff`, `timeline`, `rewind` |
| Validation | `test`, `eval`, `gates` |
| Advanced records | `turn`, `run`, `events`, `trace` |

## Create and Inspect Lanes

```text
crabdb lane spawn <NAME> [--from <REF>] \
  [--workdir-mode virtual|sparse|full-cow|overlay-cow] \
  [--materialize[=true|false]] [--no-materialize] \
  [--workdir <PATH>] [--paths <PATH>...] [--include-neighbors] \
  [--provider <PROVIDER>] [--model <MODEL>]

crabdb lane list
crabdb lane show <NAME>
crabdb lane status <NAME>
crabdb lane rm <NAME> [--force]
```

### Workdir modes

| Mode | What it does |
| --- | --- |
| `virtual` | Creates no filesystem workdir. This is the high-scale default. |
| `sparse` | Materializes only selected paths. |
| `full-cow` | Materializes the full root and tries filesystem clone COW. |
| `overlay-cow` | Creates an empty FUSE mountpoint for a transparent write-time COW view. Reads come from CrabDB objects; writes land in the lane upper layer. |

Compatibility flags:

- `--materialize` creates a materialized workdir.
- `--no-materialize` keeps the lane virtual.
- `--paths <PATH>...` implies a sparse materialized workdir.

`overlay-cow` requires FUSE support when mounted by a runtime such as
`crabdb agent start --workdir-mode overlay-cow`: macFUSE on macOS, or `/dev/fuse`
access on Linux. If the mount fails, CrabDB reports the setup error instead of
copying the full workdir.

### Sparse path boundaries

`lane spawn --paths <PATH>...` creates a sparse workdir scoped to those paths.

When `lane.enforce_sparse_paths=true`, sparse paths become a hard write
boundary:

- Writes must stay inside the selected paths.
- Deletes must stay inside the selected paths.
- Both sides of a rename must stay inside the selected paths.

The boundary is stored with the lane. If a sparse manifest is missing, CrabDB
can restore it during the next valid sparse update.

### Status output

`lane status` reports:

- Branch changes.
- Dirty materialized workdir state.
- Queued merges.
- `base_status`.
- How far the lane base is behind the default branch.

## Review, Readiness, and Handoff

```text
crabdb lane review <NAME> [--limit <N>]
crabdb lane contribution <NAME> [--limit <N>]
crabdb lane readiness <NAME>
crabdb lane refresh-preview <NAME> [--target <BRANCH>]
crabdb lane handoff <NAME> [--limit <N>]
```

Default limit: 50 for `review`, `contribution`, and `handoff`.

| Command | Use it when |
| --- | --- |
| `review` | You want a compact review packet for the lane. |
| `contribution` | You want a change review bundle with operation history. |
| `readiness` | You want blockers, warnings, and merge-readiness signals. |
| `refresh-preview` | You want to see what a refresh onto a target branch would do. |
| `handoff` | You need a transfer packet for another person or agent. |

`refresh-preview` reports:

- How many first-parent operations the lane base is behind the target.
- Changed paths that would come in from the target.
- Conflicts that would need resolution.

## Coordinate Work

```text
crabdb lane claim <NAME> <PATH> [--ttl-secs <SECONDS>]
crabdb lane message <NAME> --role <ROLE> --text <TEXT> [--session <SESSION>]
```

Default claim TTL: 600 seconds.

Claims and write leases are advisory by default. If `lane.claim_enforcement` is
set to `warn` or `reject`, CrabDB checks writes against active claims and
leases:

- `warn` emits policy warnings.
- `reject` fails before recording.
- Read leases do not grant write permission.

Use `lane message` for timeline notes such as reviewer comments, agent status,
or handoff context.

## Workdir Editing

```text
crabdb lane record <NAME> [-m <MESSAGE>] [--preview]

crabdb lane watch <NAME> [-m <MESSAGE>] [--interval-secs <SECONDS>] \
  [--debounce-ms <MS>] [--include-untracked] [--once]

crabdb lane read <NAME> <PATH> [--hydrate] [--no-hydrate] [--force] \
  [--include-neighbors]

crabdb lane hydrate <NAME> <PATH>... [--force] [--include-neighbors]
crabdb lane workdir <NAME>

crabdb lane sync-workdir <NAME> [--force] [--paths <PATH>...] \
  [--include-neighbors]

crabdb lane checkout <NAME> [--force] [--dry-run] [--workdir <PATH>]
```

### Record changes

`lane record` records current workdir changes as one operation.

Use `--preview` before recording risky work. Preview reports:

- Changed paths.
- Ignored paths.
- Nested `.git` or `.crabdb` directories.
- Symlinks, hardlinks, and external mounts.
- Oversized changed files.
- Whether current lane policy would allow the record.

### Watch changes

`lane watch` records workdir changes repeatedly.

Defaults:

| Setting | Default |
| --- | --- |
| Interval | 2 seconds |
| Debounce | None unless set |
| Untracked files | Excluded unless `--include-untracked` is set |

Use `--once` when you want one watch pass instead of a loop.

### Read and hydrate files

`lane read` reads one file from a lane. In sparse workdirs it can hydrate the
path before reading unless `--no-hydrate` is set.

`lane hydrate` is the path-scoped convenience form of:

```sh
crabdb lane sync-workdir <NAME> --paths <PATH>...
```

It uses the same dirty-workdir and safety checks as `sync-workdir --paths`.

### Sync and rescue

`lane sync-workdir` refreshes a materialized workdir from the lane head.

Without `--force`, it refuses to overwrite dirty content. With `--force`, it can
overwrite dirty files, but it copies recoverable files into a rescue directory
first.

Human output prints:

```text
Rescue workdir: .crabdb/lane-workdir-rescue/...
```

Full workdir refreshes are staged in a hidden sibling directory and verified
against a manifest before replacing the visible workdir.

## Patches, Diffs, and History

```text
crabdb lane apply-patch <NAME> --patch <FILE> \
  [--allow-ignored] [--allow-stale]

crabdb lane diff <NAME> [--stat] [--patch] [--show-line-ids]
crabdb lane timeline <NAME> [--limit <N>]
crabdb lane rewind <NAME> --to <CHANGE|ROOT|REF> \
  [--record-current] [--sync-workdir]
```

Timeline default limit: 30.

| Command | Use it when |
| --- | --- |
| `apply-patch` | Apply a structured patch directly to the lane branch. |
| `diff` | Compare the lane head against its base. Use `--stat` for a copyable total and `--patch` for unified patches. |
| `timeline` | Inspect operations recorded on the lane. |
| `rewind` | Move the lane back to a known change, root, or ref. |

`lane diff --patch` prints a Git-style unified diff. In an interactive
terminal, CrabDB colorizes patch headers, hunks, additions, and deletions by
default. Pass the global `--no-color` flag, or set `NO_COLOR=1`, for plain text.

`lane rewind` records a `LaneRewind` operation. With `--record-current`, CrabDB
first records dirty materialized workdir edits when possible and preserves the
pre-rewind head as:

```text
rewind/<lane>/<change>
```

With `--sync-workdir`, a clean materialized workdir is refreshed to the new
head.

## Tests, Evals, and Gates

```text
crabdb lane test <NAME> [--turn <TURN>] [--timeout-secs <SECONDS>] \
  [--suite <SUITE>] [--score <SCORE>] [--threshold <THRESHOLD>] \
  -- <COMMAND>...

crabdb lane eval <NAME> [--turn <TURN>] [--timeout-secs <SECONDS>] \
  [--suite <SUITE>] [--score <SCORE>] [--threshold <THRESHOLD>] \
  -- <COMMAND>...

crabdb lane gates <NAME> [--kind <KIND>] [--limit <N>]
```

Defaults:

| Setting | Default |
| --- | --- |
| Test/eval timeout | 600 seconds |
| Gate history limit | 50 |

Use `test` for executable validation, such as `cargo test`. Use `eval` for
scored or model-based checks. Both record gate metadata on the lane.

## Turns

Turn commands are lower-level records for prompt and tool workflows. Most users
will see turns through `crabdb agent ...`, but these commands are useful for
custom integrations.

```text
crabdb lane turn start <NAME> [--from <REF>] [--title <TITLE>] \
  [--base-change <CHANGE>]

crabdb lane turn show <TURN_ID>
crabdb lane turn message <TURN_ID> --role <ROLE> --text <TEXT>

crabdb lane turn event <TURN_ID> --event-type <TYPE> \
  [--payload-json <JSON>] [--change <CHANGE>] [--message <TEXT>]

crabdb lane turn apply-patch <TURN_ID> --patch <FILE> \
  [--allow-ignored] [--allow-stale]

crabdb lane turn end <TURN_ID> [--status <STATUS>]
```

Default turn end status: `completed`.

## Runs

Runs track paused and resumed lane workflows.

```text
crabdb lane run pause <NAME> --reason <REASON> --summary <SUMMARY> \
  [--state-json <JSON>] [--interruption-json <JSON>] \
  [--session <SESSION>] [--turn <TURN>]

crabdb lane run list [--lane <LANE>] [--status <STATUS>]
crabdb lane run show <RUN_ID>
crabdb lane run resume <RUN_ID> [--reviewer <NAME>] [--note <TEXT>]
```

Use runs when an automation needs to pause for approval, persist state, and
resume later with reviewer context.

## Events and Traces

```text
crabdb lane events [--lane <LANE>] [--session <SESSION>] [--turn <TURN>] \
  [--type <TYPE>] [--limit <N>]

crabdb lane trace start <TURN_ID> --type <TYPE> --name <NAME> \
  [--parent <SPAN>] [--trace-id <TRACE>] [--attributes-json <JSON>]

crabdb lane trace end <SPAN_ID> [--status <STATUS>] [--result-json <JSON>]

crabdb lane trace list [--lane <LANE>] [--session <SESSION>] \
  [--turn <TURN>] [--trace-id <TRACE>] [--limit <N>]

crabdb lane trace summary [--lane <LANE>] [--session <SESSION>] \
  [--turn <TURN>] [--trace-id <TRACE>] [--slowest <N>]

crabdb lane trace show <SPAN_ID>
```

Defaults:

| Setting | Default |
| --- | --- |
| Event limit | 50 |
| Trace list limit | 50 |
| Trace summary slowest spans | 5 |
| Trace end status | `completed` |

See [Sessions, approvals, anchors, and leases](sessions-approvals-anchors-and-leases.md)
for related session, approval, anchor, and lease commands.

## Safety Notes

Lanes are designed to keep risky work reviewable:

- Virtual lanes avoid filesystem side effects.
- Sparse lanes can enforce path boundaries.
- Record preview shows risky filesystem entries before recording.
- Sync rescue directories preserve overwritten dirty files.
- Rewind records a durable operation instead of silently moving state.
- Gates keep validation history attached to the lane.

## Implementation References

Use these files when you need to verify the CLI surface from code:

- `crates/crabdb/src/cli/command/lane_args.rs`
- `crates/crabdb/src/cli/command/lane_args/turn.rs`
- `crates/crabdb/src/cli/command/lane_args/run.rs`
- `crates/crabdb/src/cli/command/lane_args/trace.rs`
- `crates/crabdb/src/cli/command/handler/lane.rs`
- `crates/crabdb/src/model/reports/lane.rs`
- `crates/crabdb/src/model/lane`

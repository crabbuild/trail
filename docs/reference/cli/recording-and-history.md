# CLI Reference: Recording and History

## `status`

```text
crabdb status [--branch <BRANCH>]
```

Shows branch state, root object, worktree cleanliness, and changed paths.

## `record`

```text
crabdb record [-m <MESSAGE>] [--paths <PATH>...] [--kind <KIND>] [--session <SESSION>] [--allow-ignored]
```

Allowed record kinds:

- `file-edit`
- `multi-file-edit`
- `format`
- `manual-checkpoint`
- `manual-record`

## `watch`

```text
crabdb watch [-m <MESSAGE>] [--session <SESSION>] [--interval-secs <SECONDS>] [--debounce-ms <MS>] [--include-untracked] [--once]
```

Default interval is 2 seconds. `--debounce` is an alias for `--debounce-ms`.

## `timeline`

```text
crabdb timeline [--limit <N>] [--branch <BRANCH>] [--session <SESSION>] [--agent <AGENT>]
```

Default limit is 30.

## `show`

```text
crabdb show <SELECTOR>
```

Shows operations, messages, refs, or objects resolved from a selector.

## `why`

```text
crabdb why [<PATH:LINE>] [--at <REF>] [--line-id <LINE_ID>]
```

Explains path-line or stable line provenance.

## `history`

```text
crabdb history [<SELECTOR>] [--file-id <FILE_ID>] [--line-id <LINE_ID>]
```

Uses derived file and line history indexes.

## `code-from`

```text
crabdb code-from <SELECTOR>
```

Finds operations and changed paths from a message, session, or agent.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/worktree_args.rs`, `crates/crabdb/src/cli/command/inspect_args.rs`
- Handler parsing: `crates/crabdb/src/cli/command/handler/parsing.rs`
- Reports: `crates/crabdb/src/model/reports/worktree.rs`, `crates/crabdb/src/model/inspect`


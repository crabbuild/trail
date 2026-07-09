# CLI Reference: Recording and History

## `status`

```text
trail status [--branch <BRANCH>]
```

Shows branch state, root object, worktree cleanliness, and changed paths.

## `record`

```text
trail record [-m <MESSAGE>] [--paths <PATH>...] [--kind <KIND>] [--session <SESSION>] [--allow-ignored]
```

Allowed record kinds:

- `file-edit`
- `multi-file-edit`
- `format`
- `manual-checkpoint`
- `manual-record`

## `watch`

```text
trail watch [-m <MESSAGE>] [--session <SESSION>] [--interval-secs <SECONDS>] [--debounce-ms <MS>] [--include-untracked] [--once]
```

Default interval is 2 seconds. `--debounce` is an alias for `--debounce-ms`.

## `timeline`

```text
trail timeline [--limit <N>] [--branch <BRANCH>] [--session <SESSION>] [--lane <LANE>]
```

Default limit is 30.

## `show`

```text
trail show <SELECTOR>
```

Shows operations, messages, refs, or objects resolved from a selector.

## `why`

```text
trail why [<PATH:LINE>] [--at <REF>] [--line-id <LINE_ID>]
```

Explains path-line or stable line provenance.

## `history`

```text
trail history [<SELECTOR>] [--file-id <FILE_ID>] [--line-id <LINE_ID>]
```

Uses derived file and line history indexes.

## `code-from`

```text
trail code-from <SELECTOR>
```

Finds operations and changed paths from a message, session, or lane.

## Code Facts Used

- Args: `crates/trail/src/cli/command/worktree_args.rs`, `crates/trail/src/cli/command/inspect_args.rs`
- Handler parsing: `crates/trail/src/cli/command/handler/parsing.rs`
- Reports: `crates/trail/src/model/reports/worktree.rs`, `crates/trail/src/model/inspect`

# Inspect History and Provenance

Trail records operations, messages, file changes, and line changes so reviewers and agents can answer where code came from.

## Timeline

```sh
trail timeline --limit 30
trail timeline --branch main
trail timeline --session session-docs
trail timeline --lane doc-bot
```

Timeline returns recent operations scoped by branch, session, or lane.

## Show

```sh
trail show <selector>
```

`show` resolves operations, messages, refs, and object IDs.

## Why

```sh
trail why README.md:2
trail why --line-id <change-id>:<local-seq>
```

`why` explains the current text and history for a path-line selector or stable line ID.

## History

```sh
trail history README.md
trail history --file-id <file-id>
trail history --line-id <line-id>
```

History uses derived indexes. If the indexes are missing or stale after manual database surgery, rebuild them with `trail index rebuild`.

## Code From

```sh
trail code-from <message-or-session-or-lane>
```

This finds source operations and changed paths from a message, session, or lane branch.

## Low-Level Inspection

Advanced/internal commands:

```sh
trail object show <object-id>
trail root show <root-id>
trail text show <text-id> --limit 50
trail map range <map-id> --map-type path --limit 50
```

Use these when debugging storage or implementing integrations. Most users should prefer `status`, `diff`, `why`, `history`, and `show`.

## Code Facts Used

- Inspect args: `trail/src/cli/command/inspect_args.rs`
- Inspection models: `trail/src/model/inspect`
- Tests: `show_history_and_code_from_use_recorded_indexes`, `inspection_apis_decode_objects_roots_and_texts`, `map_debug_commands_decode_known_prolly_maps`

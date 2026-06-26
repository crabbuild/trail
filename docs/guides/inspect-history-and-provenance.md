# Inspect History and Provenance

CrabDB records operations, messages, file changes, and line changes so reviewers and agents can answer where code came from.

## Timeline

```sh
crabdb timeline --limit 30
crabdb timeline --branch main
crabdb timeline --session session-docs
crabdb timeline --agent doc-bot
```

Timeline returns recent operations scoped by branch, session, or agent.

## Show

```sh
crabdb show <selector>
```

`show` resolves operations, messages, refs, and object IDs.

## Why

```sh
crabdb why README.md:2
crabdb why --line-id <change-id>:<local-seq>
```

`why` explains the current text and history for a path-line selector or stable line ID.

## History

```sh
crabdb history README.md
crabdb history --file-id <file-id>
crabdb history --line-id <line-id>
```

History uses derived indexes. If the indexes are missing or stale after manual database surgery, rebuild them with `crabdb index rebuild`.

## Code From

```sh
crabdb code-from <message-or-session-or-agent>
```

This finds source operations and changed paths from a message, session, or agent branch.

## Low-Level Inspection

Advanced/internal commands:

```sh
crabdb object show <object-id>
crabdb root show <root-id>
crabdb text show <text-id> --limit 50
crabdb map range <map-id> --map-type path --limit 50
```

Use these when debugging storage or implementing integrations. Most users should prefer `status`, `diff`, `why`, `history`, and `show`.

## Code Facts Used

- Inspect args: `crates/crabdb/src/cli/command/inspect_args.rs`
- Inspection models: `crates/crabdb/src/model/inspect`
- Tests: `show_history_and_code_from_use_recorded_indexes`, `inspection_apis_decode_objects_roots_and_texts`, `map_debug_commands_decode_known_prolly_maps`


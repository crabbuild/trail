# Lanes, Sessions, Turns, and Traces

CrabDB models active work as durable branch, conversation, and activity records.
A lane is the branch-backed container. External coding agents, humans, and
automation can all work inside lanes.

## Lanes

A lane has:

- A `LaneRecord`: name, provider, model, creation time, and metadata.
- A `LaneBranch`: ref name, base and head changes, base and head roots,
  current session, optional workdir, status, and timestamps.

Use:

```sh
crabdb lane spawn doc-bot --from main
crabdb lane list
crabdb lane show doc-bot
```

## Sessions

Sessions group turns, messages, events, and operations.

```sh
crabdb session start doc-bot --title "Documentation pass" --id session-docs
crabdb session context session-docs --limit 20
crabdb session end session-docs --status completed
```

## Turns

A turn is durable work within a lane session. It has a base change, before
change, optional after change, and status.

```sh
crabdb lane turn start doc-bot --title "Update docs"
crabdb lane turn message <turn-id> --role user --text "Write docs"
crabdb lane turn apply-patch <turn-id> --patch patch.json
crabdb lane turn end <turn-id> --status completed
```

## Events and Spans

Events are structured records linked to lanes, sessions, turns, changes, and
messages. Trace spans are parentable event pairs with duration and status.

```sh
crabdb lane events --lane doc-bot --limit 20
crabdb lane trace start <turn-id> --type tool --name "render docs"
crabdb lane trace end <span-id> --status completed
crabdb lane trace summary --lane doc-bot
```

Sensitive values in trace metadata are redacted by the storage layer.

## Code Facts Used

- Lane models: `crates/crabdb/src/model/lane`
- Lane/session/turn/trace CLI args: `crates/crabdb/src/cli/command/lane_args`
- Trace storage/query: `crates/crabdb/src/db/lane/control/traces`
- Tests: `lane_sessions_track_messages_patches_and_turns`, `lane_turn_cli_tracks_events_and_closeout`, `lane_trace_spans_are_parentable_redacted_and_available_across_surfaces`

# Lanes, Sessions, Turns, and Traces

Trail models active work as durable branch, conversation, and activity records.
A lane is the branch-backed container. External coding agents, humans, and
automation can all work inside lanes.

## Lanes

A lane has:

- A `LaneRecord`: name, provider, model, creation time, and metadata.
- A `LaneBranch`: ref name, base and head changes, base and head roots,
  current session, optional workdir, status, and timestamps.

Use:

```sh
trail lane spawn doc-bot --from main
trail lane list
trail lane show doc-bot
```

## Sessions

Sessions group turns, messages, events, and operations.

```sh
trail session start doc-bot --title "Documentation pass" --id session-docs
trail session context session-docs --limit 20
trail session end session-docs --status completed
```

## Turns

A turn is durable work within a lane session. It has a base change, before
change, optional after change, and status.

```sh
trail lane turn start doc-bot --title "Update docs"
trail lane turn message <turn-id> --role user --text "Write docs"
trail lane turn apply-patch <turn-id> --patch patch.json
trail lane turn end <turn-id> --status completed
```

Agent-hosted turns can also carry a typed `TurnEnvelope` in turn metadata. The
envelope is a compact receipt for one prompt/tool/checkpoint cycle: provider,
model, Trail and upstream session IDs, prompt hash and summary, workspace
context, usage, capture counters, and outcome. Full assistant text remains in
message/transcript records; the envelope stores only compact metadata.

Completed agent turns resolve to either an outcome checkpoint or an explicit
`no_changes` outcome. This lets review, handoff, and rewind views distinguish a
real checkpoint from a prompt that inspected or reasoned without changing lane
state.

## Events and Spans

Events are structured records linked to lanes, sessions, turns, changes, and
messages. Trace spans are parentable event pairs with duration and status.

```sh
trail lane events --lane doc-bot --limit 20
trail lane trace start <turn-id> --type tool --name "render docs"
trail lane trace end <span-id> --status completed
trail lane trace summary --lane doc-bot
```

Sensitive values in trace metadata are redacted by the storage layer.

## Code Facts Used

- Lane models: `trail/src/model/lane`
- Lane/session/turn/trace CLI args: `trail/src/cli/command/lane_args`
- Trace storage/query: `trail/src/db/lane/control/traces`
- Tests: `lane_sessions_track_messages_patches_and_turns`, `lane_turn_cli_tracks_events_and_closeout`, `lane_trace_spans_are_parentable_redacted_and_available_across_surfaces`

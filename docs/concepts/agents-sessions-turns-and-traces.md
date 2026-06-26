# Agents, Sessions, Turns, and Traces

CrabDB models coding-agent work as durable branch, conversation, and activity records.

## Agents

An agent has:

- An `AgentRecord`: name, provider, model, creation time, and metadata.
- An `AgentBranch`: ref name, base and head changes, base and head roots, current session, optional workdir, status, and timestamps.

Use:

```sh
crabdb agent spawn doc-bot --from main
crabdb agent list
crabdb agent show doc-bot
```

## Sessions

Sessions group turns, messages, events, and operations.

```sh
crabdb session start doc-bot --title "Documentation pass" --id session-docs
crabdb session context session-docs --limit 20
crabdb session end session-docs --status completed
```

## Turns

A turn is durable work within an agent session. It has a base change, before change, optional after change, and status.

```sh
crabdb agent turn start doc-bot --title "Update docs"
crabdb agent turn message <turn-id> --role user --text "Write docs"
crabdb agent turn apply-patch <turn-id> --patch patch.json
crabdb agent turn end <turn-id> --status completed
```

## Events and Spans

Events are structured records linked to agents, sessions, turns, changes, and messages. Trace spans are parentable event pairs with duration and status.

```sh
crabdb agent events --agent doc-bot --limit 20
crabdb agent trace start <turn-id> --type tool --name "render docs"
crabdb agent trace end <span-id> --status completed
crabdb agent trace summary --agent doc-bot
```

Sensitive values in trace metadata are redacted by the storage layer.

## Code Facts Used

- Agent models: `crates/crabdb/src/model/agent`
- Agent/session/turn/trace CLI args: `crates/crabdb/src/cli/command/agent_args`
- Trace storage/query: `crates/crabdb/src/db/agent/control/traces`
- Tests: `agent_sessions_track_messages_patches_and_turns`, `agent_turn_cli_tracks_events_and_closeout`, `agent_trace_spans_are_parentable_redacted_and_available_across_surfaces`


# Sessions, Turns, Messages, and Runs

Use sessions and turns when agent work needs durable context. Use run checkpoints when a host must pause and later resume work.

## Sessions

```sh
crabdb session start doc-bot --title "Docs update" --id session-docs
crabdb session current doc-bot
crabdb session list --agent doc-bot
crabdb session show session-docs
crabdb session context session-docs --limit 50
crabdb session end session-docs --status completed
```

Sessions contain turns, messages, events, and operations.

## Agent Messages

```sh
crabdb agent message doc-bot --role assistant --text "Plan completed" --session session-docs
```

## Turns

```sh
crabdb agent turn start doc-bot --title "Apply docs patch"
crabdb agent turn show <turn-id>
crabdb agent turn message <turn-id> --role user --text "Update the guide"
crabdb agent turn event <turn-id> --event-type note --message "started"
crabdb agent turn apply-patch <turn-id> --patch patch.json
crabdb agent turn end <turn-id> --status completed
```

## Runs

```sh
crabdb agent run pause doc-bot \
  --reason approval_required \
  --summary "Waiting for shell approval" \
  --state-json '{"step":"test"}'

crabdb agent run list --agent doc-bot --status paused
crabdb agent run show <run-id>
crabdb agent run resume <run-id> --reviewer alice --note "approved"
```

Run status filters accept paused, resumed, blocked, cancelled, or all.

## Code Facts Used

- Session args: `crates/crabdb/src/cli/command/collaboration_args/sessions.rs`
- Turn/run args: `crates/crabdb/src/cli/command/agent_args/turn.rs`, `crates/crabdb/src/cli/command/agent_args/run.rs`
- Models: `crates/crabdb/src/model/agent/activity.rs`, `crates/crabdb/src/model/agent/coordination.rs`
- Tests: `local_api_and_mcp_manage_agent_sessions`, `agent_turn_cli_tracks_events_and_closeout`


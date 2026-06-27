# Sessions, Turns, Messages, and Runs

Use sessions and turns when lane work needs durable context. Use run checkpoints when a host must pause and later resume work.

## Sessions

```sh
crabdb session start doc-bot --title "Docs update" --id session-docs
crabdb session current doc-bot
crabdb session list --lane doc-bot
crabdb session show session-docs
crabdb session context session-docs --limit 50
crabdb session end session-docs --status completed
```

Sessions contain turns, messages, events, and operations.

## Lane Messages

```sh
crabdb lane message doc-bot --role assistant --text "Plan completed" --session session-docs
```

## Turns

```sh
crabdb lane turn start doc-bot --title "Apply docs patch"
crabdb lane turn show <turn-id>
crabdb lane turn message <turn-id> --role user --text "Update the guide"
crabdb lane turn event <turn-id> --event-type note --message "started"
crabdb lane turn apply-patch <turn-id> --patch patch.json
crabdb lane turn end <turn-id> --status completed
```

## Runs

```sh
crabdb lane run pause doc-bot \
  --reason approval_required \
  --summary "Waiting for shell approval" \
  --state-json '{"step":"test"}'

crabdb lane run list --lane doc-bot --status paused
crabdb lane run show <run-id>
crabdb lane run resume <run-id> --reviewer alice --note "approved"
```

Run status filters accept paused, resumed, blocked, cancelled, or all.

## Code Facts Used

- Session args: `crates/crabdb/src/cli/command/collaboration_args/sessions.rs`
- Turn/run args: `crates/crabdb/src/cli/command/lane_args/turn.rs`, `crates/crabdb/src/cli/command/lane_args/run.rs`
- Models: `crates/crabdb/src/model/lane/activity.rs`, `crates/crabdb/src/model/lane/coordination.rs`
- Tests: `local_api_and_mcp_manage_lane_sessions`, `lane_turn_cli_tracks_events_and_closeout`

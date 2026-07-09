# Sessions, Turns, Messages, and Runs

Use sessions and turns when lane work needs durable context. Use run checkpoints when a host must pause and later resume work.

## Sessions

```sh
trail session start doc-bot --title "Docs update" --id session-docs
trail session current doc-bot
trail session list --lane doc-bot
trail session show session-docs
trail session context session-docs --limit 50
trail session end session-docs --status completed
```

Sessions contain turns, messages, events, and operations.

## Lane Messages

```sh
trail lane message doc-bot --role assistant --text "Plan completed" --session session-docs
```

## Turns

```sh
trail lane turn start doc-bot --title "Apply docs patch"
trail lane turn show <turn-id>
trail lane turn message <turn-id> --role user --text "Update the guide"
trail lane turn event <turn-id> --event-type note --message "started"
trail lane turn apply-patch <turn-id> --patch patch.json
trail lane turn end <turn-id> --status completed
```

## Runs

```sh
trail lane run pause doc-bot \
  --reason approval_required \
  --summary "Waiting for shell approval" \
  --state-json '{"step":"test"}'

trail lane run list --lane doc-bot --status paused
trail lane run show <run-id>
trail lane run resume <run-id> --reviewer alice --note "approved"
```

Run status filters accept paused, resumed, blocked, cancelled, or all.

## Code Facts Used

- Session args: `crates/trail/src/cli/command/collaboration_args/sessions.rs`
- Turn/run args: `crates/trail/src/cli/command/lane_args/turn.rs`, `crates/trail/src/cli/command/lane_args/run.rs`
- Models: `crates/trail/src/model/lane/activity.rs`, `crates/trail/src/model/lane/coordination.rs`
- Tests: `local_api_and_mcp_manage_lane_sessions`, `lane_turn_cli_tracks_events_and_closeout`

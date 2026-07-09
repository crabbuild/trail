# Use Case: Provenance for Review

Use Trail provenance before editing or approving a change.

## Questions to Ask

```sh
trail why src/lib.rs:42
trail history src/lib.rs
trail code-from session-docs
trail show <change-id>
```

For a lane branch:

```sh
trail lane review doc-bot
trail lane contribution doc-bot
trail lane diff doc-bot --patch --show-line-ids
```

## Review Signals

Look for:

- Operation message and actor.
- Session and turn linkage.
- Changed paths and line changes.
- Messages and trace events.
- Latest test/eval gates.
- Pending approvals.

## Code Facts Used

- Provenance commands: `trail/src/cli/command/inspect_args.rs`
- Lane review reports: `trail/src/model/lane/core.rs`
- Tests: `show_history_and_code_from_use_recorded_indexes`, `lane_management_commands_have_backing_apis`

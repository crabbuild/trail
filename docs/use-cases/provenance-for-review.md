# Use Case: Provenance for Review

Use CrabDB provenance before editing or approving a change.

## Questions to Ask

```sh
crabdb why src/lib.rs:42
crabdb history src/lib.rs
crabdb code-from session-docs
crabdb show <change-id>
```

For a lane branch:

```sh
crabdb lane review doc-bot
crabdb lane contribution doc-bot
crabdb lane diff doc-bot --patch --show-line-ids
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

- Provenance commands: `crates/crabdb/src/cli/command/inspect_args.rs`
- Lane review reports: `crates/crabdb/src/model/lane/core.rs`
- Tests: `show_history_and_code_from_use_recorded_indexes`, `lane_management_commands_have_backing_apis`

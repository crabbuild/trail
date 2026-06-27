# Events, Traces, and Spans

Events and spans make lane activity queryable across CLI, HTTP, and MCP.

## Events

List events:

```sh
crabdb lane events --lane doc-bot --limit 50
crabdb lane events --session session-docs
crabdb lane events --turn <turn-id>
crabdb lane events --type patch_applied
```

Add a turn event:

```sh
crabdb lane turn event <turn-id> \
  --event-type tool_call \
  --payload-json '{"tool":"cargo test"}'
```

Events can link to a change or message.

## Spans

Start a span:

```sh
crabdb lane trace start <turn-id> --type tool --name "cargo test"
```

Start a child span:

```sh
crabdb lane trace start <turn-id> --type command --name "unit tests" --parent <span-id>
```

End a span:

```sh
crabdb lane trace end <span-id> --status completed --result-json '{"ok":true}'
```

Query spans:

```sh
crabdb lane trace list --lane doc-bot --limit 50
crabdb lane trace summary --lane doc-bot --slowest 5
crabdb lane trace show <span-id>
```

## Redaction

Trace metadata and event payloads pass through redaction helpers. Common secret-looking fields and private-key-like content are redacted before storage.

## Code Facts Used

- Trace args: `crates/crabdb/src/cli/command/lane_args/trace.rs`
- Trace models: `crates/crabdb/src/model/lane/activity.rs`
- Redaction: `crates/crabdb/src/db/util/redaction.rs`
- Tests: `lane_trace_metadata_redacts_common_secrets`, `lane_trace_events_are_queryable_across_cli_api_and_mcp`

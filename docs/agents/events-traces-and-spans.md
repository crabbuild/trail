# Events, Traces, and Spans

Events and spans make agent activity queryable across CLI, HTTP, and MCP.

## Events

List events:

```sh
crabdb agent events --agent doc-bot --limit 50
crabdb agent events --session session-docs
crabdb agent events --turn <turn-id>
crabdb agent events --type patch_applied
```

Add a turn event:

```sh
crabdb agent turn event <turn-id> \
  --event-type tool_call \
  --payload-json '{"tool":"cargo test"}'
```

Events can link to a change or message.

## Spans

Start a span:

```sh
crabdb agent trace start <turn-id> --type tool --name "cargo test"
```

Start a child span:

```sh
crabdb agent trace start <turn-id> --type command --name "unit tests" --parent <span-id>
```

End a span:

```sh
crabdb agent trace end <span-id> --status completed --result-json '{"ok":true}'
```

Query spans:

```sh
crabdb agent trace list --agent doc-bot --limit 50
crabdb agent trace summary --agent doc-bot --slowest 5
crabdb agent trace show <span-id>
```

## Redaction

Trace metadata and event payloads pass through redaction helpers. Common secret-looking fields and private-key-like content are redacted before storage.

## Code Facts Used

- Trace args: `crates/crabdb/src/cli/command/agent_args/trace.rs`
- Trace models: `crates/crabdb/src/model/agent/activity.rs`
- Redaction: `crates/crabdb/src/db/util/redaction.rs`
- Tests: `agent_trace_metadata_redacts_common_secrets`, `agent_trace_events_are_queryable_across_cli_api_and_mcp`


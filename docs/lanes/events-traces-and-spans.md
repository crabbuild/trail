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

Event types are storage metadata, so CrabDB validates them before insertion:
they must be 1-128 bytes and use only ASCII letters, digits, `_`, `-`, and
`.`. Leading/trailing whitespace, paths, credentials, and secret-looking values
are rejected before any event row is stored.

`lane.max_event_payload_bytes` rejects oversized event payloads before storage.
The limit applies to both the incoming JSON payload and the redacted stored JSON
payload, so redaction cannot shrink an oversized input below the configured
limit. `0` disables it.

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

`lane.max_trace_payload_bytes` rejects oversized `span_started` and
`span_ended` payloads before storage or trace-span indexing. The generic
`lane.max_event_payload_bytes` limit still applies to span events too. Trace
limits are also checked before and after redaction; `0` disables either limit.

## Redaction

Trace metadata and event payloads pass through redaction helpers. Common secret-looking fields and private-key-like content are redacted before storage.

## Code Facts Used

- Trace args: `crates/crabdb/src/cli/command/lane_args/trace.rs`
- Trace models: `crates/crabdb/src/model/lane/activity.rs`
- Redaction: `crates/crabdb/src/db/util/redaction.rs`
- Tests: `lane_trace_metadata_redacts_common_secrets`, `lane_trace_events_are_queryable_across_cli_api_and_mcp`, `lane_event_and_trace_payload_limits_are_enforced`

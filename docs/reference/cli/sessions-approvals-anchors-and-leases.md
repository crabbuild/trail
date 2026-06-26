# CLI Reference: Sessions, Approvals, Anchors, and Leases

## `session`

```text
crabdb session start <AGENT> [--title <TITLE>] [--id <ID>]
crabdb session current [AGENT]
crabdb session list [--agent <AGENT>]
crabdb session show <SESSION_ID>
crabdb session context <SESSION_ID> [--limit <N>]
crabdb session end <SESSION_ID> [--status <STATUS>]
```

Session context default limit is 50. End status defaults to `completed`.

## `approvals`

```text
crabdb approvals request <AGENT> --action <ACTION> --summary <SUMMARY> [--payload-json <JSON>] [--session <SESSION>] [--turn <TURN>]
crabdb approvals list [--agent <AGENT>] [--status <STATUS>]
crabdb approvals show <APPROVAL_ID>
crabdb approvals decide <APPROVAL_ID> --decision <approved|rejected|cancelled> [--reviewer <REVIEWER>] [--note <NOTE>]
```

## `agent run`

```text
crabdb agent run pause <AGENT> --reason <REASON> --summary <SUMMARY> [--state-json <JSON>] [--interruption-json <JSON>] [--session <SESSION>] [--turn <TURN>]
crabdb agent run list [--agent <AGENT>] [--status <STATUS>]
crabdb agent run show <RUN_ID>
crabdb agent run resume <RUN_ID> [--reviewer <REVIEWER>] [--note <NOTE>]
```

## `agent turn`

```text
crabdb agent turn start <AGENT> [--from <REF>] [--title <TITLE>] [--base-change <CHANGE>]
crabdb agent turn show <TURN_ID>
crabdb agent turn message <TURN_ID> --role <ROLE> --text <TEXT>
crabdb agent turn event <TURN_ID> --event-type <TYPE> [--payload-json <JSON>] [--change <CHANGE>] [--message <MESSAGE>]
crabdb agent turn apply-patch <TURN_ID> --patch <FILE> [--allow-ignored]
crabdb agent turn end <TURN_ID> [--status <STATUS>]
```

Turn end status defaults to `completed`.

## `agent trace`

```text
crabdb agent trace start <TURN_ID> --type <TYPE> --name <NAME> [--parent <SPAN>] [--trace-id <TRACE>] [--attributes-json <JSON>]
crabdb agent trace end <SPAN_ID> [--status <STATUS>] [--result-json <JSON>]
crabdb agent trace list [--agent <AGENT>] [--session <SESSION>] [--turn <TURN>] [--trace-id <TRACE>] [--limit <N>]
crabdb agent trace summary [--agent <AGENT>] [--session <SESSION>] [--turn <TURN>] [--trace-id <TRACE>] [--slowest <N>]
crabdb agent trace show <SPAN_ID>
```

Trace list default limit is 50; summary slowest default is 5. Trace end status defaults to `completed`.

## `agent events`

```text
crabdb agent events [--agent <AGENT>] [--session <SESSION>] [--turn <TURN>] [--type <TYPE>] [--limit <N>]
```

Default limit is 50.

## `anchor`

```text
crabdb anchor create <PATH:LINE> --label <LABEL>
crabdb anchor resolve <ANCHOR_ID>
crabdb anchor list
crabdb anchor delete <ANCHOR_ID>
```

## `lease`

```text
crabdb lease acquire <AGENT> --path <PATH> [--mode <read|write>] [--ttl-secs <SECONDS>]
crabdb lease list [--all]
crabdb lease release <LEASE_ID>
```

Lease mode defaults to `write`; TTL defaults to 3600 seconds.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/collaboration_args`, `crates/crabdb/src/cli/command/agent_args`
- Models: `crates/crabdb/src/model/agent/activity.rs`, `crates/crabdb/src/model/agent/coordination.rs`
- Tests: `anchors_follow_stable_line_identity`, `local_api_and_mcp_expose_advisory_leases`


# CLI Reference: Sessions, Approvals, Anchors, and Leases

## `session`

```text
trail session start <LANE> [--title <TITLE>] [--id <ID>]
trail session current [LANE]
trail session list [--lane <LANE>]
trail session show <SESSION_ID>
trail session context <SESSION_ID> [--limit <N>]
trail session end <SESSION_ID> [--status <STATUS>]
```

Session context default limit is 50. End status defaults to `completed`.

## `approvals`

```text
trail approvals request <LANE> --action <ACTION> --summary <SUMMARY> [--payload-json <JSON>] [--session <SESSION>] [--turn <TURN>]
trail approvals list [--lane <LANE>] [--status <STATUS>]
trail approvals show <APPROVAL_ID>
trail approvals decide <APPROVAL_ID> --decision <approved|rejected|cancelled> [--reviewer <REVIEWER>] [--note <NOTE>]
```

## `lane run`

```text
trail lane run pause <LANE> --reason <REASON> --summary <SUMMARY> [--state-json <JSON>] [--interruption-json <JSON>] [--session <SESSION>] [--turn <TURN>]
trail lane run list [--lane <LANE>] [--status <STATUS>]
trail lane run show <RUN_ID>
trail lane run resume <RUN_ID> [--reviewer <REVIEWER>] [--note <NOTE>]
```

## `lane turn`

```text
trail lane turn start <LANE> [--from <REF>] [--title <TITLE>] [--base-change <CHANGE>]
trail lane turn show <TURN_ID>
trail lane turn message <TURN_ID> --role <ROLE> --text <TEXT>
trail lane turn event <TURN_ID> --event-type <TYPE> [--payload-json <JSON>] [--change <CHANGE>] [--message <MESSAGE>]
trail lane turn apply-patch <TURN_ID> --patch <FILE> [--allow-ignored]
trail lane turn end <TURN_ID> [--status <STATUS>]
```

Turn end status defaults to `completed`.

## `lane trace`

```text
trail lane trace start <TURN_ID> --type <TYPE> --name <NAME> [--parent <SPAN>] [--trace-id <TRACE>] [--attributes-json <JSON>]
trail lane trace end <SPAN_ID> [--status <STATUS>] [--result-json <JSON>]
trail lane trace list [--lane <LANE>] [--session <SESSION>] [--turn <TURN>] [--trace-id <TRACE>] [--limit <N>]
trail lane trace summary [--lane <LANE>] [--session <SESSION>] [--turn <TURN>] [--trace-id <TRACE>] [--slowest <N>]
trail lane trace show <SPAN_ID>
```

Trace list default limit is 50; summary slowest default is 5. Trace end status defaults to `completed`.

## `lane events`

```text
trail lane events [--lane <LANE>] [--session <SESSION>] [--turn <TURN>] [--type <TYPE>] [--limit <N>]
```

Default limit is 50.

## `anchor`

```text
trail anchor create <PATH:LINE> --label <LABEL>
trail anchor resolve <ANCHOR_ID>
trail anchor list
trail anchor delete <ANCHOR_ID>
```

## `lease`

```text
trail lease acquire <LANE> --path <PATH> [--mode <read|write>] [--ttl-secs <SECONDS>]
trail lease list [--all]
trail lease release <LEASE_ID>
```

Lease mode defaults to `write`; TTL defaults to 3600 seconds.

## Code Facts Used

- Args: `crates/trail/src/cli/command/collaboration_args`, `crates/trail/src/cli/command/lane_args`
- Models: `crates/trail/src/model/lane/activity.rs`, `crates/trail/src/model/lane/coordination.rs`
- Tests: `anchors_follow_stable_line_identity`, `local_api_and_mcp_expose_advisory_leases`

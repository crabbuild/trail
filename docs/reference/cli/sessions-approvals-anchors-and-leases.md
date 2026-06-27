# CLI Reference: Sessions, Approvals, Anchors, and Leases

## `session`

```text
crabdb session start <LANE> [--title <TITLE>] [--id <ID>]
crabdb session current [LANE]
crabdb session list [--lane <LANE>]
crabdb session show <SESSION_ID>
crabdb session context <SESSION_ID> [--limit <N>]
crabdb session end <SESSION_ID> [--status <STATUS>]
```

Session context default limit is 50. End status defaults to `completed`.

## `approvals`

```text
crabdb approvals request <LANE> --action <ACTION> --summary <SUMMARY> [--payload-json <JSON>] [--session <SESSION>] [--turn <TURN>]
crabdb approvals list [--lane <LANE>] [--status <STATUS>]
crabdb approvals show <APPROVAL_ID>
crabdb approvals decide <APPROVAL_ID> --decision <approved|rejected|cancelled> [--reviewer <REVIEWER>] [--note <NOTE>]
```

## `lane run`

```text
crabdb lane run pause <LANE> --reason <REASON> --summary <SUMMARY> [--state-json <JSON>] [--interruption-json <JSON>] [--session <SESSION>] [--turn <TURN>]
crabdb lane run list [--lane <LANE>] [--status <STATUS>]
crabdb lane run show <RUN_ID>
crabdb lane run resume <RUN_ID> [--reviewer <REVIEWER>] [--note <NOTE>]
```

## `lane turn`

```text
crabdb lane turn start <LANE> [--from <REF>] [--title <TITLE>] [--base-change <CHANGE>]
crabdb lane turn show <TURN_ID>
crabdb lane turn message <TURN_ID> --role <ROLE> --text <TEXT>
crabdb lane turn event <TURN_ID> --event-type <TYPE> [--payload-json <JSON>] [--change <CHANGE>] [--message <MESSAGE>]
crabdb lane turn apply-patch <TURN_ID> --patch <FILE> [--allow-ignored]
crabdb lane turn end <TURN_ID> [--status <STATUS>]
```

Turn end status defaults to `completed`.

## `lane trace`

```text
crabdb lane trace start <TURN_ID> --type <TYPE> --name <NAME> [--parent <SPAN>] [--trace-id <TRACE>] [--attributes-json <JSON>]
crabdb lane trace end <SPAN_ID> [--status <STATUS>] [--result-json <JSON>]
crabdb lane trace list [--lane <LANE>] [--session <SESSION>] [--turn <TURN>] [--trace-id <TRACE>] [--limit <N>]
crabdb lane trace summary [--lane <LANE>] [--session <SESSION>] [--turn <TURN>] [--trace-id <TRACE>] [--slowest <N>]
crabdb lane trace show <SPAN_ID>
```

Trace list default limit is 50; summary slowest default is 5. Trace end status defaults to `completed`.

## `lane events`

```text
crabdb lane events [--lane <LANE>] [--session <SESSION>] [--turn <TURN>] [--type <TYPE>] [--limit <N>]
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
crabdb lease acquire <LANE> --path <PATH> [--mode <read|write>] [--ttl-secs <SECONDS>]
crabdb lease list [--all]
crabdb lease release <LEASE_ID>
```

Lease mode defaults to `write`; TTL defaults to 3600 seconds.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/collaboration_args`, `crates/crabdb/src/cli/command/lane_args`
- Models: `crates/crabdb/src/model/lane/activity.rs`, `crates/crabdb/src/model/lane/coordination.rs`
- Tests: `anchors_follow_stable_line_identity`, `local_api_and_mcp_expose_advisory_leases`

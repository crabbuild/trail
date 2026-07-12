# Lane Merge Queue Hard Cutover Design

## Goal

Make Trail's serialized merge queue an explicitly lane-only capability across
the CLI, HTTP API, MCP surface, Rust reports and methods, and SQLite schema.
Generic branch and ref merges remain available through `trail merge` and are
not accepted by the lane merge queue.

This is an intentional breaking change. The old top-level command, HTTP routes,
MCP tools, resource URI, report types, methods, and database table are removed
without compatibility aliases.

## Domain Boundaries

Trail exposes three distinct merge workflows:

```text
trail merge <source> --into <target>
trail lane merge <lane> --into <target>
trail lane merge-queue <command>
```

- `trail merge` merges a generic branch or ref.
- `trail lane merge` previews or explicitly direct-merges one lane.
- `trail lane merge-queue` schedules lane merges for serialized execution.

The lane merge queue never accepts ordinary branch refs. Callers that need to
merge a branch or generic source use `trail merge` directly.

## CLI Contract

The canonical commands are:

```sh
trail lane merge-queue add <lane> --into main --priority 10
trail lane merge-queue list
trail lane merge-queue explain <queue-id-or-lane>
trail lane merge-queue run --limit 1
trail lane merge-queue remove <queue-id-or-lane>
```

`add` resolves `<lane>` by lane name or stable lane id and rejects selectors
that resolve only to a branch or generic ref. `list` and `run` are
workspace-wide operations even though they live under the lane command group.

The top-level `trail merge-queue` command is removed. Clap must reject it as an
unknown command. No hidden alias or deprecation period is provided.

`trail lane merge <lane> --direct` remains the explicit queue bypass. A
non-dry-run merge into the configured shared/default branch without `--direct`
continues to fail safely, but its next-step message points only to:

```sh
trail lane merge-queue add <lane> --into <target>
trail lane merge-queue run
```

All help output, human rendering, documentation, examples, scripts, and
benchmarks use the new spelling.

## HTTP API Contract

The lane merge queue uses the existing plural lane namespace:

```text
POST   /v1/lanes/merges/queue
GET    /v1/lanes/merges/queue
POST   /v1/lanes/merges/queue/run
GET    /v1/lanes/merges/queue/{selector}/explain
DELETE /v1/lanes/merges/queue/{selector}
```

This complements the existing single-lane merge operation:

```text
POST /v1/lanes/{lane}/merge
```

The enqueue request is strict:

```json
{
  "lane": "doc-bot",
  "into": "main",
  "priority": 10
}
```

`lane` and `into` are required. `priority` defaults to zero. Unknown fields are
rejected. Legacy request keys such as `source`, `target`, and `target_branch`
are not accepted.

The old `/v1/merge-queue` routes, including the query-form
`/v1/merge-queue/explain?selector=...`, are removed and follow normal
unknown-route behavior. OpenAPI contains only the new routes and lane-specific
schemas.

## Public Models and Storage

Public queue types are renamed around the lane domain:

- `LaneMergeQueueEntry`
- `LaneMergeQueueAddReport`
- `LaneMergeQueueRemoveReport`
- `LaneMergeQueueExplainReport`
- `LaneMergeQueueRunReport`
- `LaneMergeQueueRunItem`

An entry exposes `queue_id`, `lane_id`, `lane`, `target_ref`, `status`,
`priority`, `created_at`, and `updated_at`. The stable `lane_id` is the stored
identity. `lane` is the resolved human-facing lane name. Generic `source_ref`
is not part of queue requests, entries, or reports.

The SQLite queue table becomes:

```sql
CREATE TABLE lane_merge_queue (
    queue_id   TEXT PRIMARY KEY,
    lane_id    TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    status     TEXT NOT NULL,
    priority   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

Indexes support active-entry lookup by lane and target and queue execution by
status, descending priority, and ascending creation time. New queue ids use the
`lmq_` prefix.

Database and internal service methods use lane-specific names, including
`enqueue_lane_merge`, `list_lane_merge_queue`, `explain_lane_merge_queue`,
`run_lane_merge_queue`, and `remove_lane_merge_queue`. Normalization of generic
queue source refs is removed.

`merge_results` and `conflict_sets` retain generic source and target ref fields
because they also represent ordinary `trail merge` operations. The nullable
`merge_results.queue_id` column becomes `lane_queue_id` for new lane-queue
results.

## Schema Migration

The schema version advances from v15 to v16. Migration runs within Trail's
existing schema savepoint and is all-or-nothing.

The v16 migration:

1. Drops `merge_queue` without copying queued, running, completed, or cancelled
   entries.
2. Creates `lane_merge_queue` and its indexes.
3. Renames `merge_results.queue_id` to `lane_queue_id` and clears legacy
   non-null values because their generic queue records no longer exist.
4. Records schema version 16 only after every schema operation succeeds.

Failure rolls the entire migration back, including the version marker. Opening
a successfully migrated workspace never recreates the old table.

This destructive queue reset is deliberate. Backups remain the user's recovery
mechanism for pre-cutover queue data; Trail does not attempt to infer which
legacy generic entries represented lanes.

## Queue Behavior and Safety

Enqueue resolves the supplied lane selector to a current `LaneRecord` and
`LaneBranch`, then stores the stable lane id and normalized target branch ref.
If the same lane and target already have a queued or running entry, enqueue
returns that entry rather than creating a duplicate.

Execution resolves the stored lane id back to its current lane branch and then
reuses the existing lane merge safety path. Before each item it rechecks:

- lane existence and branch state;
- dirty materialized workdir state;
- approvals and required gates;
- lane readiness;
- target existence and merge conflicts.

Entries run by priority descending and creation time ascending. The runner
stops at the first blocker, error, or conflict, preserving the existing
serialized safety property. Completed and cancelled v16 entries remain visible
as audit history.

`explain` and `remove` accept a new `lmq_` queue id, lane id, or lane name.
Ambiguous or missing selectors return an invalid-input error without mutation.
Branch refs and old `mq_` ids do not resolve through the new API.

## MCP and Resource Contract

MCP tools become:

```text
trail.lane_merge_queue_add
trail.lane_merge_queue_list
trail.lane_merge_queue_explain
trail.lane_merge_queue_run
trail.lane_merge_queue_remove
```

The workspace resource becomes:

```text
trail://workspace/lane-merge-queue
```

The old `trail.merge_queue_*` tools and
`trail://workspace/merge-queue` resource are removed. Risk annotations remain
equivalent: list and explain are read-only; add is a write; run and remove are
consequential writes.

## Error Handling

- Unknown lane selectors fail before an entry is written.
- Branch and generic ref selectors are rejected as non-lane inputs.
- Invalid targets fail before an entry is written.
- Direct shared-target lane merges continue to explain how to enqueue or use
  the explicit `--direct` bypass.
- A lane removed or corrupted after enqueue blocks execution and leaves an
  auditable failed/blocked result rather than falling back to a generic ref
  merge.
- Queue conflicts use the existing structured conflict-set mechanism and stop
  subsequent queue processing.
- Removed CLI, HTTP, and MCP names receive their surfaces' standard
  unknown-command, unknown-route, or unknown-tool response.

## Verification

Automated coverage must prove:

1. `trail lane merge-queue` parses every subcommand and
   `trail merge-queue` does not parse.
2. Local and daemon-backed CLI commands produce matching lane-specific JSON and
   human output.
3. OpenAPI exposes only `/v1/lanes/merges/queue` routes and strict request and
   response schemas.
4. Removed HTTP routes return unknown-route responses.
5. MCP discovery exposes only the lane-specific tools and resource; each tool
   drives the corresponding database operation.
6. A v15 database with legacy queue rows migrates transactionally to v16,
   discards those rows, clears legacy merge-result queue links, and contains no
   `merge_queue` table.
7. Failed v16 migration leaves the v15 schema and version marker intact.
8. Branch/ref inputs are rejected while lane name and lane-id inputs enqueue
   successfully.
9. Duplicate enqueue, priority ordering, item limits, explanation,
   cancellation, readiness blockers, dirty workdirs, approvals, test/eval
   gates, and conflict pausing retain their safety behavior.
10. `trail merge` continues to merge generic branches independently of the lane
    queue.
11. Documentation, skills, rendered next steps, benchmarks, and repository
    source contain no callable use of the removed public names.

## Deliberate Non-Goals

- Migrating or preserving legacy queue entries.
- Queueing generic branches or arbitrary refs.
- Renaming generic merge-result or conflict source/target fields.
- Changing lane readiness, approval, gate, merge strategy, or conflict
  semantics.
- Adding a compatibility alias, warning period, or legacy HTTP shim.

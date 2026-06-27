# MCP Tools Reference

CrabDB MCP tool names are stable strings under the `crabdb.` prefix.

## Core

- `crabdb.doctor`
- `crabdb.status`
- `crabdb.diff`
- `crabdb.timeline`
- `crabdb.why`
- `crabdb.history`
- `crabdb.code_from`
- `crabdb.config_list`
- `crabdb.config_get`
- `crabdb.config_set`
- `crabdb.ignore_list`
- `crabdb.ignore_add`
- `crabdb.ignore_remove`
- `crabdb.ignore_check`
- `crabdb.guardrail_check`

## Lanes

- `crabdb.lane_spawn`
- `crabdb.lane_claim`
- `crabdb.lane_list`
- `crabdb.lane_show`
- `crabdb.lane_status`
- `crabdb.lane_review`
- `crabdb.lane_contribution`
- `crabdb.gate_history`
- `crabdb.lane_readiness`
- `crabdb.lane_handoff`
- `crabdb.lane_rewind`
- `crabdb.lane_remove`

## Sessions, Approvals, Runs, Leases, Anchors

- `crabdb.session_start`
- `crabdb.session_list`
- `crabdb.session_current`
- `crabdb.session_show`
- `crabdb.session_context`
- `crabdb.session_end`
- `crabdb.approval_request`
- `crabdb.approval_list`
- `crabdb.approval_show`
- `crabdb.approval_decide`
- `crabdb.run_pause`
- `crabdb.run_list`
- `crabdb.run_show`
- `crabdb.run_resume`
- `crabdb.lease_acquire`
- `crabdb.lease_list`
- `crabdb.lease_release`
- `crabdb.anchor_create`
- `crabdb.anchor_list`
- `crabdb.anchor_resolve`
- `crabdb.anchor_delete`

## Merge and Conflicts

- `crabdb.merge_queue_add`
- `crabdb.merge_queue_list`
- `crabdb.merge_queue_run`
- `crabdb.merge_queue_remove`
- `crabdb.conflict_list`
- `crabdb.conflict_show` returns conflict details plus deterministic explanation evidence and conservative next steps.
- `crabdb.conflict_resolve`

## Turns, Events, Spans, Patches, Gates, Workdirs

- `crabdb.begin_turn`
- `crabdb.add_message`
- `crabdb.add_event`
- `crabdb.event_list`
- `crabdb.span_start`
- `crabdb.span_end`
- `crabdb.span_list`
- `crabdb.span_summary`
- `crabdb.span_show`
- `crabdb.apply_patch`
- `crabdb.end_turn`
- `crabdb.show_turn`
- `crabdb.diff_lane`
- `crabdb.run_test`
- `crabdb.run_eval`
- `crabdb.sync_workdir`
- `crabdb.read_file`

`crabdb.sync_workdir` returns `rescue_workdir` when a forced sync overwrites
dirty materialized workdir files. That directory contains copied dirty regular
files and `manifest.json`.

## Tool Risk Annotations

The MCP layer annotates tools as read-only, workspace write, destructive write, or open-world write. Read-only tool calls also run under CrabDB's read-only guard, so they must not persist database or index changes while serving the request. The `crabdb://workspace/status` resource uses the same non-mutating status path. Read-only examples include status, diff, timeline, why, history, lane status, review, readiness, handoff, sessions, approvals, runs, leases, anchors, merge queue list, conflict show, event/span queries, and guardrail check. Destructive-write examples include `crabdb.lane_rewind` because it intentionally moves a lane ref and may refresh a materialized workdir.

Open-world write examples are `crabdb.run_test` and `crabdb.run_eval` because they execute commands in lane workdirs.

Mutating tool calls use strict JSON arguments. Unknown argument fields are
returned as MCP tool errors instead of being ignored. Mutating tool attempts are
recorded in `external_mutation_audit` with the tool name, success/error status,
inferred lane/ref/change ids, and a small redacted summary. Read-only tools are
guarded and are not written to this audit table.

## Prompts

- `crabdb.lane_task`
- `crabdb.review_lane`
- `crabdb.resolve_conflict`

## Resources

Static resources include workspace status, doctor, lanes, merge queue, conflicts, OpenAPI, and compatibility guide resources. Resource templates cover individual lanes, lane review packets, sessions, turns, conflicts, approvals, run states, and trace spans.

## Code Facts Used

- Tools: `crates/crabdb/src/mcp/tools`
- Tool calls: `crates/crabdb/src/mcp/tool_call`
- Risk annotations: `crates/crabdb/src/mcp/tools/annotations.rs`
- Prompts/resources: `crates/crabdb/src/mcp/capabilities`

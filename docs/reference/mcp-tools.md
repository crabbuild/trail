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

## Agents

- `crabdb.agent_spawn`
- `crabdb.agent_claim`
- `crabdb.agent_list`
- `crabdb.agent_show`
- `crabdb.agent_status`
- `crabdb.agent_contribution`
- `crabdb.gate_history`
- `crabdb.agent_readiness`
- `crabdb.agent_handoff`
- `crabdb.agent_remove`

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
- `crabdb.conflict_show`
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
- `crabdb.diff_agent`
- `crabdb.run_test`
- `crabdb.run_eval`
- `crabdb.sync_workdir`
- `crabdb.read_file`

## Tool Risk Annotations

The MCP layer annotates tools as read-only, workspace write, destructive write, or open-world write. Read-only examples include status, diff, timeline, why, history, agent status, readiness, handoff, sessions, approvals, runs, leases, anchors, merge queue list, conflict show, event/span queries, and guardrail check.

Open-world write examples are `crabdb.run_test` and `crabdb.run_eval` because they execute commands in agent workdirs.

## Prompts

- `crabdb.agent_task`
- `crabdb.review_agent`
- `crabdb.resolve_conflict`

## Resources

Static resources include workspace status, doctor, agents, merge queue, conflicts, OpenAPI, and compatibility guide resources. Resource templates cover individual agents, sessions, turns, conflicts, approvals, run states, and trace spans.

## Code Facts Used

- Tools: `crates/crabdb/src/mcp/tools`
- Tool calls: `crates/crabdb/src/mcp/tool_call`
- Risk annotations: `crates/crabdb/src/mcp/tools/annotations.rs`
- Prompts/resources: `crates/crabdb/src/mcp/capabilities`


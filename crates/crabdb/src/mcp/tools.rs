use serde_json::{json, Value};

use super::response::object_schema;

pub(crate) fn tools() -> Value {
    let mut tools = json!([
        {
            "name": "crabdb.doctor",
            "title": "CrabDB Doctor",
            "description": "Run read-only operational diagnostics for workspace health, locks, fsck, approvals, leases, merge queue, conflicts, and agent workdirs.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.status",
            "title": "CrabDB Status",
            "description": "Read the current CrabDB branch status and changed paths.",
            "inputSchema": object_schema(json!({
                "branch": { "type": "string", "description": "Optional CrabDB branch name." }
            }), vec![])
        },
        {
            "name": "crabdb.diff",
            "title": "CrabDB Diff",
            "description": "Show a ref range, root range, or dirty worktree diff with optional patches and stable line ids.",
            "inputSchema": object_schema(json!({
                "range": { "type": "string", "description": "Ref range such as main..feature or ch_a..ch_b." },
                "root": { "type": "string", "description": "Root id range such as obj_a..obj_b." },
                "dirty": { "type": "boolean", "description": "Diff the current branch head against the materialized worktree." },
                "patch": { "type": "boolean" },
                "show_line_ids": { "type": "boolean" },
                "show-line-ids": { "type": "boolean" }
            }), vec![])
        },
        {
            "name": "crabdb.timeline",
            "title": "CrabDB Timeline",
            "description": "Read recent operations, optionally scoped to one branch, session, or agent.",
            "inputSchema": object_schema(json!({
                "branch": { "type": "string" },
                "session": { "type": "string" },
                "agent": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec![])
        },
        {
            "name": "crabdb.why",
            "title": "Explain Line Provenance",
            "description": "Explain the stable file and line identity plus recorded history for a path:line selector or line id.",
            "inputSchema": object_schema(json!({
                "path_line": { "type": "string" },
                "line_id": { "type": "string" },
                "branch": { "type": "string" },
                "at": { "type": "string" }
            }), vec![])
        },
        {
            "name": "crabdb.history",
            "title": "Read File Or Line History",
            "description": "Read file history by path/file id or line history by line id.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string" },
                "path": { "type": "string" },
                "file_id": { "type": "string" },
                "line_id": { "type": "string" }
            }), vec![])
        },
        {
            "name": "crabdb.code_from",
            "title": "Trace Code From Source",
            "description": "Find operations and changed paths produced by a change, message, session, or agent branch.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string" }
            }), vec!["selector"])
        },
        {
            "name": "crabdb.agent_spawn",
            "title": "Spawn Agent Branch",
            "description": "Create or reuse an isolated agent branch, optionally materializing its workdir.",
            "inputSchema": object_schema(json!({
                "name": { "type": "string" },
                "from_ref": { "type": "string" },
                "materialize": { "type": "boolean" },
                "workdir": { "type": "string" },
                "workdir_path": { "type": "string" },
                "provider": { "type": "string" },
                "model": { "type": "string" }
            }), vec!["name"])
        },
        {
            "name": "crabdb.agent_claim",
            "title": "Claim Agent Path",
            "description": "Create a soft advisory write claim for an agent path, returning conflicts as warnings instead of hard failures.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "path": { "type": "string" },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }), vec!["agent", "path"])
        },
        {
            "name": "crabdb.agent_list",
            "title": "List Agents",
            "description": "List agent metadata and branch state for coordinator discovery.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.agent_show",
            "title": "Show Agent",
            "description": "Show one agent's metadata and branch state by name or agent id.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_status",
            "title": "Agent Status",
            "description": "Show one agent branch status, including workdir and latest test state.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_contribution",
            "title": "Agent Contribution",
            "description": "Summarize one agent branch for review with status, changed paths, operations, sessions, events, approvals, and latest gates.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.gate_history",
            "title": "Agent Gate History",
            "description": "List recent durable test/eval gate results for one agent branch, optionally filtered by kind.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "kind": { "type": "string", "enum": ["all", "test", "tests", "eval", "evals"] },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_readiness",
            "title": "Agent Readiness",
            "description": "Assess whether one agent branch is ready to merge by checking conflicts, approvals, workdir state, tests, and evals.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_handoff",
            "title": "Agent Handoff",
            "description": "Package one agent branch for transfer with readiness, current session context, recent events, spans, operations, and next steps.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_remove",
            "title": "Remove Agent",
            "description": "Remove an agent branch and materialized workdir. Requires force when the branch has unmerged changes.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "force": { "type": "boolean" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.config_list",
            "title": "List CrabDB Config",
            "description": "List validated CrabDB workspace configuration entries.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.config_get",
            "title": "Get CrabDB Config",
            "description": "Read one validated CrabDB workspace configuration entry.",
            "inputSchema": object_schema(json!({
                "key": { "type": "string" }
            }), vec!["key"])
        },
        {
            "name": "crabdb.config_set",
            "title": "Set CrabDB Config",
            "description": "Set one CrabDB workspace configuration entry using the same validation as the CLI.",
            "inputSchema": object_schema(json!({
                "key": { "type": "string" },
                "value": { "type": "string" }
            }), vec!["key", "value"])
        },
        {
            "name": "crabdb.session_start",
            "title": "Start Agent Session",
            "description": "Start an explicit durable session and attach it to an agent branch.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "title": { "type": "string" },
                "id": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.session_list",
            "title": "List Agent Sessions",
            "description": "List durable agent sessions, optionally scoped to one agent.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec![])
        },
        {
            "name": "crabdb.session_current",
            "title": "Current Agent Session",
            "description": "Read current agent branch session attachments, optionally for one agent.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec![])
        },
        {
            "name": "crabdb.session_show",
            "title": "Show Agent Session",
            "description": "Return a session with turns, messages, events, and operations.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" }
            }), vec!["session_id"])
        },
        {
            "name": "crabdb.session_context",
            "title": "Session Context",
            "description": "Return a bounded session context packet with total counts and recent messages, events, turns, and operations.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec!["session_id"])
        },
        {
            "name": "crabdb.session_end",
            "title": "End Agent Session",
            "description": "End a durable agent session with completed, failed, cancelled, or archived status.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" },
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }), vec!["session_id"])
        },
        {
            "name": "crabdb.approval_request",
            "title": "Request Human Approval",
            "description": "Create a durable pending approval for a sensitive agent action.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object" },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" }
            }), vec!["agent", "action", "summary"])
        },
        {
            "name": "crabdb.approval_list",
            "title": "List Human Approvals",
            "description": "List durable approval gates, optionally scoped by agent and status.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "status": { "type": "string", "enum": ["pending", "approved", "rejected", "cancelled", "all"] }
            }), vec![])
        },
        {
            "name": "crabdb.approval_show",
            "title": "Show Human Approval",
            "description": "Show one durable approval gate by id.",
            "inputSchema": object_schema(json!({
                "approval_id": { "type": "string" }
            }), vec!["approval_id"])
        },
        {
            "name": "crabdb.approval_decide",
            "title": "Decide Human Approval",
            "description": "Approve, reject, or cancel a pending approval gate.",
            "inputSchema": object_schema(json!({
                "approval_id": { "type": "string" },
                "decision": { "type": "string", "enum": ["approved", "rejected", "cancelled"] },
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }), vec!["approval_id", "decision"])
        },
        {
            "name": "crabdb.run_pause",
            "title": "Pause Agent Run",
            "description": "Persist a serialized paused agent run checkpoint for later resume.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "reason": { "type": "string" },
                "summary": { "type": "string" },
                "state": { "type": "object" },
                "interruption": { "type": "object" },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" }
            }), vec!["agent", "reason", "summary"])
        },
        {
            "name": "crabdb.run_list",
            "title": "List Agent Run States",
            "description": "List durable paused/resumed agent checkpoints, optionally scoped by agent and status.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "status": { "type": "string", "enum": ["paused", "resumed", "blocked", "cancelled", "all"] }
            }), vec![])
        },
        {
            "name": "crabdb.run_show",
            "title": "Show Agent Run State",
            "description": "Show one durable agent run checkpoint by id.",
            "inputSchema": object_schema(json!({
                "run_id": { "type": "string" }
            }), vec!["run_id"])
        },
        {
            "name": "crabdb.run_resume",
            "title": "Resume Agent Run",
            "description": "Mark a paused checkpoint resumed after any linked approval is approved.",
            "inputSchema": object_schema(json!({
                "run_id": { "type": "string" },
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }), vec!["run_id"])
        },
        {
            "name": "crabdb.lease_acquire",
            "title": "Acquire Path Lease",
            "description": "Acquire an advisory read or write lease for an agent path before editing.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "path": { "type": "string" },
                "mode": { "type": "string", "enum": ["read", "write"] },
                "ttl_secs": { "type": "integer", "minimum": 1 }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.lease_list",
            "title": "List Path Leases",
            "description": "List active advisory leases, or all leases when all is true.",
            "inputSchema": object_schema(json!({
                "all": { "type": "boolean" }
            }), vec![])
        },
        {
            "name": "crabdb.lease_release",
            "title": "Release Path Lease",
            "description": "Release an advisory path lease by lease id.",
            "inputSchema": object_schema(json!({
                "lease_id": { "type": "string" }
            }), vec!["lease_id"])
        },
        {
            "name": "crabdb.anchor_create",
            "title": "Create Line Anchor",
            "description": "Create a durable review anchor for a path:line selector on an optional branch.",
            "inputSchema": object_schema(json!({
                "path_line": { "type": "string" },
                "label": { "type": "string" },
                "branch": { "type": "string" }
            }), vec!["path_line", "label"])
        },
        {
            "name": "crabdb.anchor_list",
            "title": "List Line Anchors",
            "description": "List durable review anchors.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.anchor_resolve",
            "title": "Resolve Line Anchor",
            "description": "Resolve a durable review anchor on an optional branch.",
            "inputSchema": object_schema(json!({
                "anchor_id": { "type": "string" },
                "branch": { "type": "string" }
            }), vec!["anchor_id"])
        },
        {
            "name": "crabdb.anchor_delete",
            "title": "Delete Line Anchor",
            "description": "Delete a durable review anchor by id.",
            "inputSchema": object_schema(json!({
                "anchor_id": { "type": "string" }
            }), vec!["anchor_id"])
        },
        {
            "name": "crabdb.merge_queue_add",
            "title": "Queue Merge",
            "description": "Queue an agent or branch ref for serialized merge into a target branch.",
            "inputSchema": object_schema(json!({
                "source": { "type": "string" },
                "target": { "type": "string" },
                "priority": { "type": "integer" }
            }), vec!["source", "target"])
        },
        {
            "name": "crabdb.merge_queue_list",
            "title": "List Merge Queue",
            "description": "List queued, running, merged, cancelled, failed, and conflicted merge queue entries.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.merge_queue_run",
            "title": "Run Merge Queue",
            "description": "Run queued merges serially, pausing on the first conflict or failure.",
            "inputSchema": object_schema(json!({
                "limit": { "type": "integer", "minimum": 1 }
            }), vec![])
        },
        {
            "name": "crabdb.merge_queue_remove",
            "title": "Remove Merge Queue Entry",
            "description": "Cancel a queued or conflicted merge queue entry by queue id, agent, branch, or ref.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string" }
            }), vec!["selector"])
        },
        {
            "name": "crabdb.conflict_list",
            "title": "List Merge Conflicts",
            "description": "List structured conflict sets opened by merge queue runs.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.conflict_show",
            "title": "Show Merge Conflict",
            "description": "Show one structured conflict set with source, target, status, and details.",
            "inputSchema": object_schema(json!({
                "conflict_set_id": { "type": "string" }
            }), vec!["conflict_set_id"])
        },
        {
            "name": "crabdb.conflict_resolve",
            "title": "Resolve Merge Conflict",
            "description": "Resolve a conflict set by taking source, taking target, or providing manual content for every conflicted path.",
            "inputSchema": object_schema(json!({
                "conflict_set_id": { "type": "string" },
                "take": { "type": "string", "enum": ["source", "target"] },
                "manual": {
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "object",
                            "additionalProperties": {
                                "oneOf": [
                                    { "type": "string" },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "content": { "type": "string" },
                                            "delete": { "type": "boolean" },
                                            "executable": { "type": "boolean" }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }), vec!["conflict_set_id"])
        },
        {
            "name": "crabdb.begin_turn",
            "title": "Begin Agent Turn",
            "description": "Create or reuse an agent branch and start a durable agent turn.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "branch": { "type": "string" },
                "session_title": { "type": "string" },
                "base_change": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.add_message",
            "title": "Add Turn Message",
            "description": "Attach a user, assistant, tool, reviewer, or system message to a turn.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "role": { "type": "string" },
                "content": { "type": "string" },
                "text": { "type": "string" }
            }), vec!["turn_id", "role"])
        },
        {
            "name": "crabdb.add_event",
            "title": "Add Turn Trace Event",
            "description": "Attach a tool call, tool result, guardrail, handoff, evaluation, or custom event to a turn.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "event_type": { "type": "string" },
                "payload": { "type": "object" },
                "change_id": { "type": "string" },
                "message_id": { "type": "string" }
            }), vec!["turn_id", "event_type"])
        },
        {
            "name": "crabdb.event_list",
            "title": "List Trace Events",
            "description": "List recent agent trace events across agents, sessions, turns, and event types.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "event_type": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec![])
        },
        {
            "name": "crabdb.span_start",
            "title": "Start Trace Span",
            "description": "Start a parentable trace span for an agent, tool call, guardrail, handoff, or evaluation within a turn.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "span_type": { "type": "string" },
                "name": { "type": "string" },
                "parent": { "type": "string" },
                "parent_span_id": { "type": "string" },
                "trace": { "type": "string" },
                "trace_id": { "type": "string" },
                "attributes": { "type": "object" }
            }), vec!["turn_id", "span_type", "name"])
        },
        {
            "name": "crabdb.span_end",
            "title": "End Trace Span",
            "description": "End a trace span with a status and optional result payload.",
            "inputSchema": object_schema(json!({
                "span_id": { "type": "string" },
                "status": { "type": "string" },
                "result": { "type": "object" }
            }), vec!["span_id"])
        },
        {
            "name": "crabdb.span_list",
            "title": "List Trace Spans",
            "description": "List derived trace spans across agents, sessions, turns, and trace ids.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "trace_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec![])
        },
        {
            "name": "crabdb.span_summary",
            "title": "Summarize Trace Spans",
            "description": "Summarize derived trace spans with status/type counts, open spans, failed spans, and slowest completed spans.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "trace_id": { "type": "string" },
                "slowest": { "type": "integer", "minimum": 1, "maximum": 50 }
            }), vec![])
        },
        {
            "name": "crabdb.span_show",
            "title": "Show Trace Span",
            "description": "Show a single derived trace span.",
            "inputSchema": object_schema(json!({
                "span_id": { "type": "string" }
            }), vec!["span_id"])
        },
        {
            "name": "crabdb.apply_patch",
            "title": "Apply Agent Patch",
            "description": "Apply a native CrabDB patch or design-style files patch to a turn's agent branch.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "message": { "type": "string" },
                "base_change": { "type": "string" },
                "session_id": { "type": "string" },
                "allow_ignored": { "type": "boolean" },
                "edits": { "type": "array", "items": { "type": "object" } },
                "files": { "type": "array", "items": { "type": "object" } }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.end_turn",
            "title": "End Agent Turn",
            "description": "Close a durable agent turn with completed, failed, cancelled, or archived status.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.show_turn",
            "title": "Show Agent Turn",
            "description": "Return a turn with its session, messages, trace events, and operations.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.diff_agent",
            "title": "Diff Agent Branch",
            "description": "Show the changes from an agent branch base to its current head.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "patch": { "type": "boolean" },
                "show_line_ids": { "type": "boolean" },
                "show-line-ids": { "type": "boolean" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.run_test",
            "title": "Run Agent Test",
            "description": "Run a command in an agent workdir and record durable test_started/test_finished events plus stdout/stderr Blob output.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["agent", "command"])
        },
        {
            "name": "crabdb.run_eval",
            "title": "Run Agent Eval",
            "description": "Run an evaluation command in an agent workdir and record durable eval_started/eval_finished events plus stdout/stderr Blob output.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["agent", "command"])
        },
        {
            "name": "crabdb.sync_workdir",
            "title": "Sync Agent Workdir",
            "description": "Refresh an agent materialized workdir from its branch head, refusing dirty edits unless force is true.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "force": { "type": "boolean" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.ignore_list",
            "title": "List Ignore Rules",
            "description": "List workspace .crabignore patterns visible to CrabDB.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.ignore_add",
            "title": "Add Ignore Rule",
            "description": "Add a workspace .crabignore pattern under CrabDB's write lock.",
            "inputSchema": object_schema(json!({
                "pattern": { "type": "string" }
            }), vec!["pattern"])
        },
        {
            "name": "crabdb.ignore_remove",
            "title": "Remove Ignore Rule",
            "description": "Remove a workspace .crabignore pattern under CrabDB's write lock.",
            "inputSchema": object_schema(json!({
                "pattern": { "type": "string" }
            }), vec!["pattern"])
        },
        {
            "name": "crabdb.ignore_check",
            "title": "Check Ignored Path",
            "description": "Check whether a relative path is ignored by the hardcoded denylist or workspace ignore files.",
            "inputSchema": object_schema(json!({
                "path": { "type": "string" }
            }), vec!["path"])
        },
        {
            "name": "crabdb.guardrail_check",
            "title": "Guardrail Check",
            "description": "Preflight an agent action against CrabDB path policy, risky tool categories, and pending approvals. Returns allowed, approval_required, or blocked.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object" },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }), vec!["action"]),
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }
    ]);
    annotate_tools(&mut tools);
    tools
}

fn annotate_tools(tools: &mut Value) {
    let Some(tools) = tools.as_array_mut() else {
        return;
    };
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(object) = tool.as_object_mut() {
            object.insert("annotations".to_string(), tool_annotations(&name));
        }
    }
}

fn tool_annotations(name: &str) -> Value {
    match tool_risk_class(name) {
        ToolRiskClass::ReadOnly => json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }),
        ToolRiskClass::Write => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": false
        }),
        ToolRiskClass::IdempotentWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }),
        ToolRiskClass::DestructiveWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": false
        }),
        ToolRiskClass::OpenWorldWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": true
        }),
    }
}

#[derive(Clone, Copy)]
enum ToolRiskClass {
    ReadOnly,
    Write,
    IdempotentWrite,
    DestructiveWrite,
    OpenWorldWrite,
}

fn tool_risk_class(name: &str) -> ToolRiskClass {
    match name {
        "crabdb.doctor"
        | "crabdb.status"
        | "crabdb.diff"
        | "crabdb.timeline"
        | "crabdb.why"
        | "crabdb.history"
        | "crabdb.code_from"
        | "crabdb.agent_list"
        | "crabdb.agent_show"
        | "crabdb.agent_status"
        | "crabdb.agent_contribution"
        | "crabdb.gate_history"
        | "crabdb.agent_readiness"
        | "crabdb.agent_handoff"
        | "crabdb.config_list"
        | "crabdb.config_get"
        | "crabdb.session_list"
        | "crabdb.session_current"
        | "crabdb.session_show"
        | "crabdb.session_context"
        | "crabdb.approval_list"
        | "crabdb.approval_show"
        | "crabdb.run_list"
        | "crabdb.run_show"
        | "crabdb.lease_list"
        | "crabdb.anchor_list"
        | "crabdb.anchor_resolve"
        | "crabdb.merge_queue_list"
        | "crabdb.conflict_list"
        | "crabdb.conflict_show"
        | "crabdb.event_list"
        | "crabdb.span_list"
        | "crabdb.span_summary"
        | "crabdb.span_show"
        | "crabdb.show_turn"
        | "crabdb.diff_agent"
        | "crabdb.ignore_list"
        | "crabdb.ignore_check"
        | "crabdb.guardrail_check" => ToolRiskClass::ReadOnly,
        "crabdb.config_set" | "crabdb.ignore_add" | "crabdb.ignore_remove" => {
            ToolRiskClass::IdempotentWrite
        }
        "crabdb.agent_remove"
        | "crabdb.anchor_delete"
        | "crabdb.merge_queue_remove"
        | "crabdb.conflict_resolve"
        | "crabdb.apply_patch"
        | "crabdb.sync_workdir" => ToolRiskClass::DestructiveWrite,
        "crabdb.run_test" | "crabdb.run_eval" => ToolRiskClass::OpenWorldWrite,
        _ => ToolRiskClass::Write,
    }
}

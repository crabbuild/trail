use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "trail.doctor",
            "title": "Trail Doctor",
            "description": "Run read-only operational diagnostics for workspace health, locks, fsck, approvals, leases, merge queue, conflicts, and lane workdirs.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.status",
            "title": "Trail Status",
            "description": "Read the current Trail branch status and changed paths.",
            "inputSchema": object_schema(json!({
                "branch": { "type": "string", "description": "Optional Trail branch name." }
            }), vec![])
        },
        {
            "name": "trail.diff",
            "title": "Trail Diff",
            "description": "Show a ref range, root range, or dirty worktree diff with optional patches and stable line ids.",
            "inputSchema": object_schema(json!({
                "range": { "type": "string", "description": "Ref range such as main..feature or change_a..change_b." },
                "root": { "type": "string", "description": "Root id range such as object_a..object_b." },
                "dirty": { "type": "boolean", "description": "Diff the current branch head against the materialized worktree." },
                "patch": { "type": "boolean" },
                "show_line_ids": { "type": "boolean" },
                "show-line-ids": { "type": "boolean" }
            }), vec![])
        },
        {
            "name": "trail.timeline",
            "title": "Trail Timeline",
            "description": "Read recent operations, optionally scoped to one branch, session, or lane.",
            "inputSchema": object_schema(json!({
                "branch": { "type": "string" },
                "session": { "type": "string" },
                "lane": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec![])
        },
        {
            "name": "trail.why",
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
            "name": "trail.history",
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
            "name": "trail.code_from",
            "title": "Trace Code From Source",
            "description": "Find operations and changed paths produced by a change, message, session, or lane branch.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string" }
            }), vec!["selector"])
        },
        {
            "name": "trail.agent_status",
            "title": "Agent Task Status",
            "description": "Show the latest high-level agent task and the next useful action without exposing lane internals.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.agent_inbox",
            "title": "Agent Task Inbox",
            "description": "Show all agent tasks grouped by what needs attention, with one primary next command.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.agent_board",
            "title": "Agent Task Board",
            "description": "Show a low-noise multi-agent board with task columns, attention counts, and one primary next command.",
            "inputSchema": object_schema(json!({
                "all": { "type": "boolean", "description": "Include archived tasks." }
            }), vec![])
        },
        {
            "name": "trail.agent_stack",
            "title": "Agent Apply Stack",
            "description": "Show overlap and a safe apply order across multiple agent tasks, with one next command.",
            "inputSchema": object_schema(json!({
                "all": { "type": "boolean", "description": "Include archived tasks." }
            }), vec![])
        },
        {
            "name": "trail.agent_next",
            "title": "Next Agent Action",
            "description": "Return one primary next command for an agent task, plus a few alternatives, so users do not need to choose between lane, transcript, diff, and apply commands manually.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_guide",
            "title": "Agent Task Guide",
            "description": "Return a compact state-aware guide for using Trail with agent tasks: current state, one next command, workflow steps, and the small mental model.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_dashboard",
            "title": "Agent Task Dashboard",
            "description": "Return one compact dashboard with next action, focus file, validation status, changed files, risk, and apply readiness.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_review_data",
            "title": "Agent Review Data",
            "description": "Return one editor-friendly review packet with next action, file review progress, focus file, changes by file, confidence, validation, risk, and apply readiness.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_review_flow",
            "title": "Agent Review Flow",
            "description": "Return a guided checklist that walks one task through inspect, mark reviewed, validate, and finish without requiring users to connect separate Trail commands manually.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_ask",
            "title": "Ask Trail About Agent Work",
            "description": "Route a plain-language agent-task question to the right read-only Trail agent report, returning the routed tool name and report payload.",
            "inputSchema": object_schema(json!({
                "question": { "type": "string", "description": "Plain-language question such as 'what changed?', 'is it safe to land?', 'recover', or 'explain README.md'." },
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec!["question"])
        },
        {
            "name": "trail.agent_view",
            "title": "View Agent Task",
            "description": "Inspect one agent task transcript, tools, changed files, checkpoint, and review packet.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_brief",
            "title": "Agent Task Brief",
            "description": "Return one compact agent task brief with next action, readiness, changed files, turn summaries, latest diff stats, and tools.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_summary",
            "title": "Agent Task Summary",
            "description": "Return the one-page post-run cockpit for an agent task: readiness, risk, validation, receipt Markdown, PR draft, and next commands.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_validate",
            "title": "Agent Validation Guidance",
            "description": "Read latest test/eval gates and suggested validation commands for an agent task without running anything.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_test_plan",
            "title": "Agent Test Plan",
            "description": "Return a prioritized test/eval checklist with exact commands based on changed areas, impact, and existing validation gates.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_report",
            "title": "Agent Task Report",
            "description": "Return a shareable review bundle for an agent task, including story, risk, changes, transcript, readiness, suggestions, and Markdown.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_handoff",
            "title": "Agent Task Handoff",
            "description": "Return a copyable handoff packet for another human or agent, with state, risks, validation, changed files, turns, tools, and exact receiver commands.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_receipt",
            "title": "Agent Task Receipt",
            "description": "Return a copyable post-run receipt for an agent task: summary, validation gates, changed files, turns, tools, risk, checkpoint, and next command.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_pr",
            "title": "Agent PR Draft",
            "description": "Return a read-only pull request draft title and body for an agent task without creating a remote PR.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_story",
            "title": "Agent Task Story",
            "description": "Return a plain-language story of what happened in an agent task, including turns, changed files, tools, notes, and next action.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_tools",
            "title": "Agent Tool Activity",
            "description": "Return captured tool calls, available ACP commands, and the turns/checkpoints/changed files around each tool.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_risk",
            "title": "Agent Task Risk",
            "description": "Return a deterministic risk level, reasons, and recommendations before applying an agent task.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_impact",
            "title": "Agent Impact",
            "description": "Return changed impact areas, blast radius, validation state, risk, and recommended review/test checks for an agent task.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_review_map",
            "title": "Agent Review Map",
            "description": "Return a file-by-file review checklist grouped by changed area, with review, why, diff, and open commands.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_confidence",
            "title": "Agent Confidence",
            "description": "Return one go/no-go confidence verdict across review freshness, validation, risk, and Git apply preflight.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_ready",
            "title": "Agent Apply Readiness",
            "description": "Run a read-only apply readiness preflight that combines task readiness, risk, Git dry-run status, blockers, and the next safe command.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_diagnose",
            "title": "Diagnose Agent Task",
            "description": "Diagnose a stuck or sideways agent task and return likely issue, evidence, recovery targets, and safe next commands before undo/rewind.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_workdir",
            "title": "Agent Task Workdir",
            "description": "Return the materialized workdir path and shell-safe cd command for an agent task.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_changes",
            "title": "Agent Changes",
            "description": "Show high-level agent change cards plus prompt/turn or operation checkpoint groups so users do not chase checkpoint ids manually.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "by_operation": { "type": "boolean" },
                "by-turn": { "type": "boolean" },
                "by-file": { "type": "boolean", "description": "Show one review card per changed file." }
            }), vec![])
        },
        {
            "name": "trail.agent_delta",
            "title": "Agent Delta",
            "description": "Show the newest completed agent turn or operation with changed files, provenance, next commands, and optional patch output.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "by_operation": { "type": "boolean", "description": "Use the newest recorded Trail operation instead of the newest turn." },
                "by-turn": { "type": "boolean" },
                "file": { "type": "string", "description": "Limit the newest delta to one workspace-relative path." },
                "patch": { "type": "boolean", "description": "Include unified patches." }
            }), vec![])
        },
        {
            "name": "trail.agent_new",
            "title": "New Agent Changes",
            "description": "Show what changed since the agent task was last marked reviewed, with optional file filter and patch output.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "file": { "type": "string", "description": "Limit new changes to one workspace-relative path." },
                "patch": { "type": "boolean", "description": "Include unified patches." }
            }), vec![])
        },
        {
            "name": "trail.agent_mark_reviewed",
            "title": "Mark Agent Reviewed",
            "description": "Mark the current agent task checkpoint as reviewed so later reads can show only new changes.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "note": { "type": "string", "description": "Optional review note." }
            }), vec![])
        },
        {
            "name": "trail.agent_mark_file_reviewed",
            "title": "Mark Agent File Reviewed",
            "description": "Mark one changed file as reviewed at the current agent task checkpoint so review-map can track file-level progress.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "path": { "type": "string", "description": "Changed file path to mark reviewed." },
                "note": { "type": "string", "description": "Optional review note." }
            }), vec!["path"])
        },
        {
            "name": "trail.agent_archive",
            "title": "Archive Agent Task",
            "description": "Hide an agent task from default agent inbox/list/latest views without deleting its lane, transcript, checkpoints, or provenance.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "note": { "type": "string", "description": "Optional archive note." }
            }), vec![])
        },
        {
            "name": "trail.agent_unarchive",
            "title": "Unarchive Agent Task",
            "description": "Restore an archived agent task to default agent inbox/list/latest views.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "note": { "type": "string", "description": "Optional unarchive note." }
            }), vec![])
        },
        {
            "name": "trail.agent_change",
            "title": "Agent Change Set",
            "description": "Inspect one high-level change card as a focused change set with files, provenance, tools, commands, and optional patches.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "card": { "type": "string", "description": "Change card rank, key, or title. Defaults to 1." },
                "patch": { "type": "boolean", "description": "Include focused patches for files in the change set." }
            }), vec![])
        },
        {
            "name": "trail.agent_timeline",
            "title": "Agent Timeline",
            "description": "Show the chronological prompt/operation timeline with checkpoints, tools, changed files, and follow-up commands.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "by_operation": { "type": "boolean" },
                "by-turn": { "type": "boolean" }
            }), vec![])
        },
        {
            "name": "trail.agent_files",
            "title": "Agent Files",
            "description": "Show changed files with the turns, prompts, checkpoints, and commands behind each file.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_file",
            "title": "Agent File",
            "description": "Inspect one file in an agent task with its change set, provenance, tools, commands, and optional focused patch.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "path": { "type": "string", "description": "Workspace-relative path, path:line selector, or absolute path inside the task workdir." },
                "patch": { "type": "boolean", "description": "Include the focused patch for this file." }
            }), vec!["path"])
        },
        {
            "name": "trail.agent_checkpoints",
            "title": "Agent Checkpoints",
            "description": "List friendly rewind targets and exact checkpoint ids for an agent task before running a destructive rewind.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_why",
            "title": "Explain Agent File Change",
            "description": "Explain why a file changed in an agent task by linking it to prompt/turn, checkpoint, tools, and a focused diff command.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "path": { "type": "string", "description": "Workspace-relative path, path:line selector, or absolute path inside the task workdir." }
            }), vec!["path"])
        },
        {
            "name": "trail.agent_compare",
            "title": "Compare Agent Tasks",
            "description": "Compare two agent tasks, highlighting shared changed files, one-sided changes, risk, and a recommended next action.",
            "inputSchema": object_schema(json!({
                "left": { "type": "string", "description": "Left agent task, lane, session, or ACP session." },
                "right": { "type": "string", "description": "Right agent task, lane, session, or ACP session." }
            }), vec!["left", "right"])
        },
        {
            "name": "trail.agent_test",
            "title": "Run Agent Task Test",
            "description": "Run a command in an agent task workdir and record a durable test gate without requiring the user to know the lane name.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["command"])
        },
        {
            "name": "trail.agent_eval",
            "title": "Run Agent Task Eval",
            "description": "Run an evaluation command in an agent task workdir and record a durable eval gate without requiring the user to know the lane name.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["command"])
        },
        {
            "name": "trail.agent_turn",
            "title": "Agent Turn",
            "description": "Inspect one agent turn with prompt, assistant summary, tools, checkpoint, changed files, and optional focused patch.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "turn": { "type": "string", "description": "1-based turn index, turn id, last, latest, or omitted for the latest completed turn." },
                "file": { "type": "string", "description": "Limit embedded diff output to one changed file path." },
                "patch": { "type": "boolean", "description": "Include unified patch text in the embedded diff." }
            }), vec![])
        },
        {
            "name": "trail.agent_diff",
            "title": "Agent Diff",
            "description": "Show the whole task diff or a single turn/checkpoint/operation diff for an agent task.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "turn": { "type": "string", "description": "1-based turn index or turn id." },
                "operation": { "type": "string", "description": "Operation/change id to diff from its first parent." },
                "checkpoint": { "type": "string", "description": "Checkpoint/change id to diff from its turn start or first parent." },
                "last_turn": { "type": "boolean" },
                "last-turn": { "type": "boolean" },
                "file": { "type": "string", "description": "Limit the diff output to one changed file path." },
                "patch": { "type": "boolean" }
            }), vec![])
        },
        {
            "name": "trail.agent_review",
            "title": "Review Agent Task",
            "description": "Show the agent task review dashboard: readiness, risk, blockers, warnings, prioritized files, and exact next commands.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." }
            }), vec![])
        },
        {
            "name": "trail.agent_focus",
            "title": "Focus Agent Review",
            "description": "Bundle the next file to inspect with its review priority, prompt/tool explanation, and focused diff so users do not manually chain review, why, and diff.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "file": { "type": "string", "description": "Optional changed file path to focus instead of the highest-priority review file." },
                "patch": { "type": "boolean", "description": "Include unified patch text in the focused diff." }
            }), vec![])
        },
        {
            "name": "trail.agent_apply",
            "title": "Apply Agent Task",
            "description": "Preview or apply an agent task back to Git. Hosts should call with dry_run first and require explicit confirmation before non-dry-run apply.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "dry_run": { "type": "boolean", "description": "Preview the apply plan without mutating Git." },
                "message": { "type": "string", "description": "Optional Git commit message for non-dry-run apply." }
            }), vec![])
        },
        {
            "name": "trail.agent_finish",
            "title": "Finish Agent Task",
            "description": "Preview or apply an agent task back to Git, then archive it after success. Hosts should call with dry_run first and require explicit confirmation before non-dry-run finish.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "dry_run": { "type": "boolean", "description": "Preview the finish plan without mutating Git or archiving the task." },
                "message": { "type": "string", "description": "Optional Git commit message for non-dry-run finish." },
                "note": { "type": "string", "description": "Optional note to store with the archive marker after successful finish." }
            }), vec![])
        },
        {
            "name": "trail.agent_rewind",
            "title": "Rewind Agent Task",
            "description": "Rewind an agent task to a checkpoint or friendly label such as before-last-turn, turn:2, before-turn:2, prompt:<text>, before-prompt:<text>, or before-last-operation.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "to": { "type": "string", "description": "Checkpoint/change id/root/ref or friendly agent rewind target." },
                "target": { "type": "string", "description": "Alias for to." }
            }), vec!["to"])
        },
        {
            "name": "trail.agent_undo",
            "title": "Undo Agent Turn",
            "description": "Undo the latest agent turn by default, or undo a selected turn, prompt, or latest operation without requiring checkpoint ids.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string", "description": "Agent task, lane, session, ACP session, or latest." },
                "last_turn": { "type": "boolean", "description": "Undo the latest completed turn; this is the default." },
                "last-turn": { "type": "boolean", "description": "Alias for last_turn." },
                "turn": { "type": "string", "description": "1-based turn index or turn id to undo." },
                "prompt": { "type": "string", "description": "Undo the latest prompt containing this text." },
                "last_operation": { "type": "boolean", "description": "Undo the latest recorded operation when no turn transcript exists." },
                "last-operation": { "type": "boolean", "description": "Alias for last_operation." }
            }), vec![])
        },
        {
            "name": "trail.config_list",
            "title": "List Trail Config",
            "description": "List validated Trail workspace configuration entries.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.config_get",
            "title": "Get Trail Config",
            "description": "Read one validated Trail workspace configuration entry.",
            "inputSchema": object_schema(json!({
                "key": { "type": "string" }
            }), vec!["key"])
        },
        {
            "name": "trail.config_set",
            "title": "Set Trail Config",
            "description": "Set one Trail workspace configuration entry using the same validation as the CLI.",
            "inputSchema": object_schema(json!({
                "key": { "type": "string" },
                "value": { "type": "string" }
            }), vec!["key", "value"])
        },
        {
            "name": "trail.ignore_list",
            "title": "List Ignore Rules",
            "description": "List workspace .trailignore patterns visible to Trail.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.ignore_add",
            "title": "Add Ignore Rule",
            "description": "Add a workspace .trailignore pattern under Trail's write lock.",
            "inputSchema": object_schema(json!({
                "pattern": { "type": "string" }
            }), vec!["pattern"])
        },
        {
            "name": "trail.ignore_remove",
            "title": "Remove Ignore Rule",
            "description": "Remove a workspace .trailignore pattern under Trail's write lock.",
            "inputSchema": object_schema(json!({
                "pattern": { "type": "string" }
            }), vec!["pattern"])
        },
        {
            "name": "trail.ignore_check",
            "title": "Check Ignored Path",
            "description": "Check whether a relative path is ignored by the hardcoded denylist or workspace ignore files.",
            "inputSchema": object_schema(json!({
                "path": { "type": "string" }
            }), vec!["path"])
        },
        {
            "name": "trail.guardrail_check",
            "title": "Guardrail Check",
            "description": "Preflight a lane action against Trail path policy, risky tool categories, and pending approvals. Returns allowed, approval_required, or blocked.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
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
    ])
}

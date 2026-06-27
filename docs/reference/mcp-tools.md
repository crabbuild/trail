# MCP Tools Reference

CrabDB MCP tool names are stable strings under the `crabdb.` prefix.
The stdio transport expects one UTF-8 JSON-RPC object per line. Each input line
is limited to 16 MiB; oversized or non-UTF-8 lines return JSON-RPC parse errors,
and the server continues reading subsequent requests.

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

## Agent Tasks

- `crabdb.agent_status`
- `crabdb.agent_inbox`
- `crabdb.agent_next`
- `crabdb.agent_ask`
- `crabdb.agent_view`
- `crabdb.agent_brief`
- `crabdb.agent_summary`
- `crabdb.agent_validate`
- `crabdb.agent_report`
- `crabdb.agent_receipt`
- `crabdb.agent_pr`
- `crabdb.agent_story`
- `crabdb.agent_risk`
- `crabdb.agent_ready`
- `crabdb.agent_diagnose`
- `crabdb.agent_test`
- `crabdb.agent_eval`
- `crabdb.agent_workdir`
- `crabdb.agent_changes`
- `crabdb.agent_delta`
- `crabdb.agent_new`
- `crabdb.agent_mark_reviewed`
- `crabdb.agent_change`
- `crabdb.agent_timeline`
- `crabdb.agent_files`
- `crabdb.agent_file`
- `crabdb.agent_checkpoints`
- `crabdb.agent_why`
- `crabdb.agent_turn`
- `crabdb.agent_compare`
- `crabdb.agent_diff`
- `crabdb.agent_review`
- `crabdb.agent_focus`
- `crabdb.agent_apply`
- `crabdb.agent_rewind`
- `crabdb.agent_undo`

These are the high-level tools an editor agent should prefer when the user asks
questions like "what should I do next?", "what did the agent do?", "what should
I review first?", "what tools were used?", "what changed?", "show the last turn
diff", "what needs attention?", "where is the workdir?", "where did the agent edit?", "which prompt changed README.md?", "last prompt?", "what changed in the last prompt?", "what changed in README.md in the last prompt?", "show transcript?", "open review?", "review this task?", "what tests should I run?", "can I merge?", or "is this ready to apply?". `crabdb.agent_ask` is the
lowest-burden front door: pass a plain-language question and it deterministically
routes to the right read-only report, returning the routed tool name and payload.
It will route apply/merge/land questions to readiness and rewind/undo questions
to checkpoint or diagnosis views rather than mutating state. Patch and diff
questions such as "show last patch", "show turn diff", or "show patch for
README.md" route to focused patch reports.
`crabdb.agent_inbox` groups all
tasks by the action they need and returns one primary next command. Its
structured `items` include each task's attention state, new files/lines since
the last review, and an optional `review_first` target for editor dashboards.
`crabdb.agent_next` returns one primary next command plus a few alternatives.
`crabdb.agent_status` and
`crabdb.agent_brief` include embedded risk reports so an editor can show the
safety signal without making a second tool call. `crabdb.agent_summary` returns
the one-page post-run cockpit with readiness, risk, validation, receipt
Markdown, PR draft, Git preflight, and next commands. `crabdb.agent_validate`
returns a read-only validation guide with latest test/eval gates and suggested
`agent_test`/`agent_eval` commands; use it before running open-world commands.
`crabdb.agent_story` returns one
plain-language account of what happened, with turn summaries, changed files,
tools, notes, and next action. `crabdb.agent_risk` returns a deterministic
low/medium/high/blocking risk level with reasons and mitigation commands before
apply. `crabdb.agent_ready` returns a read-only apply preflight that combines
task readiness, risk, Git dry-run status, blockers, warnings, and one next
command. `crabdb.agent_diagnose` explains a likely issue, supporting evidence,
friendly recovery targets, and safe inspection/recovery commands before an
editor suggests destructive undo or rewind. `crabdb.agent_test` and
`crabdb.agent_eval` run commands in the task
workdir and record durable gates without requiring the caller to know the lane
name. `crabdb.agent_brief` returns a compact task review packet with next
action, readiness, changed files, turn summaries, latest diff stats, and tools.
`crabdb.agent_report` returns a shareable review bundle with story, risk,
changes, transcript, readiness, suggestions, and a Markdown handoff string.
`crabdb.agent_receipt` returns the easier post-run artifact: summary,
validation gates, changed files, turns, tools, risk, checkpoint, next command,
and a Markdown receipt string.
`crabdb.agent_pr` returns a read-only pull request draft title and body generated
from the same recorded task state. It does not create a remote PR.
`crabdb.agent_changes` returns one primary `next` command, deterministic change
cards, then raw turn/operation groups, so editor panels can show intent-level
review chunks without asking the user to connect checkpoints manually. Each card
includes `review_command`, `focus_command`, `why_command`, and `diff_command`
fields when available.
`crabdb.agent_review` returns the review dashboard for an agent task: readiness,
risk, blockers, warnings, prioritized files to inspect first, and exact next
commands.
`crabdb.agent_focus` bundles the next file to inspect with its review priority,
prompt/tool explanation, and focused diff summary.
`crabdb.agent_workdir` returns the exact materialized task directory plus a
shell-safe `cd` command. `crabdb.agent_change` expands one change card by rank,
key, or title into files, provenance, tools, commands, and optional focused
patches. `crabdb.agent_delta` returns the newest completed turn or operation as
one card, with changed files, provenance, next commands, and optional focused
patches. `crabdb.agent_new` returns the changes since the latest reviewed marker
or the whole task when no marker exists. `crabdb.agent_mark_reviewed` writes that
marker at the current task checkpoint. `crabdb.agent_timeline` returns the chronological
prompt/operation timeline with checkpoints, tools, changed files, and per-item
follow-up commands. `crabdb.agent_files` returns a file-centric review
view with the turns, prompts, checkpoints, and commands behind each changed
file. `crabdb.agent_file` inspects one path, which is useful for editor panels
that know the currently open file. `crabdb.agent_checkpoints` lists friendly rewind
targets and exact checkpoint ids before an editor calls destructive recovery
tools. `crabdb.agent_why` answers "why did this file change?"
with the related prompt/turn, checkpoint, tools, and a focused diff command.
`crabdb.agent_turn` returns one prompt-sized receipt with prompt and assistant
previews, messages, tools, checkpoint, changed files, and optional focused
patch.
`crabdb.agent_diff` accepts `file` to keep a task, turn, operation, or
checkpoint diff scoped to one changed path.
`crabdb.agent_compare` compares two tasks, highlights shared changed files and
one-sided changes, returns both risk reports, and recommends a next command. The
tools resolve `latest`, agent task names, lane
names, CrabDB session ids, and ACP session ids so users do not need to manually
connect prompts, checkpoints, and operation ranges. Agent task reports include a
human `title` for display and a stable `name`/`lane` for exact follow-up
commands. Materialized task reports include `workdir`, the exact directory where
the agent edited files.
`crabdb.agent_undo` is the easy recovery tool for "undo the last prompt" or
"undo the prompt containing this text." `crabdb.agent_rewind` accepts exact
checkpoints and friendly targets such as `before-last-turn`, `turn:2`,
`before-turn:2`, `prompt:<text>`, `before-prompt:<text>`, and
`before-last-operation`. Both undo and rewind are marked destructive so agent
hosts can ask for explicit confirmation before moving task state or refreshing
materialized workdirs.
`crabdb.agent_apply` is also marked destructive because non-dry-run apply can
record a task workdir, create a Git commit, and fast-forward the current Git
branch. Hosts should call `crabdb.agent_ready` first and require explicit
confirmation before non-dry-run apply.

## Agent Prompts

- `crabdb.review_agent`
- `crabdb.recover_agent`
- `crabdb.apply_agent`

These prompts guide editor hosts through the common agent-task workflows using
the high-level `crabdb.agent_*` tools. They accept an optional `selector`
argument that defaults to `latest`. The review prompt starts with
`agent_summary` and file-focused inspection. The recovery prompt starts with
`agent_diagnose` before destructive undo/rewind. The apply prompt starts with
`agent_summary` and `agent_ready`, then requires explicit user confirmation
before non-dry-run apply.

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
- `crabdb.lane_refresh_preview`
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
- `crabdb.merge_queue_explain`
- `crabdb.merge_queue_remove`
- `crabdb.conflict_list`
- `crabdb.conflict_show` returns conflict details plus deterministic explanation evidence and conservative next steps.
- `crabdb.conflict_resolve`

Conflict explanations include the stored `base_root`, `target_root`, and
`source_root` snapshots used to reproduce the conflict, plus per-path
`conflict_class` values such as `modify/modify`, `delete/modify`,
`rename/modify`, `binary`, `mode`, and `same_insertion_gap`.
They can also include `known_resolutions` when a path/content conflict
signature matches a previously resolved conflict.
`crabdb.conflict_resolve` requires exactly one of `take` or `manual`. Manual
file values can be plain strings or objects with only `content`, `delete`, and
`executable`; unknown keys are rejected.

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

`crabdb.apply_patch` accepts either native `edits` or compatibility `files`;
provide exactly one non-empty array. Native edit objects and compatibility file
objects reject unknown keys, and line-id edits require `expected_text`.

`crabdb.sync_workdir` returns `rescue_workdir` when a forced sync overwrites
dirty materialized workdir files or replaces a non-directory file at the lane
workdir path. That directory contains copied recoverable regular files and
`manifest.json`.

## Tool Risk Annotations

The MCP layer annotates tools as read-only, workspace write, destructive write, or open-world write. Read-only tool calls also run under CrabDB's read-only guard, so they must not persist SQLite database changes or mutate CrabDB sidecars such as `config.toml`, `HEAD`, ref files, daemon endpoint/token/cache files, or lane workdir metadata manifests while serving the request. The `crabdb://workspace/status` resource uses the same non-mutating status path. Read-only examples include status, diff, timeline, why, history, agent status/next/ask/view/brief/validate/report/story/risk/ready/workdir/changes/files/checkpoints/why/turn/compare/diff/review/focus, lane status, review, readiness, handoff, sessions, approvals, runs, leases, anchors, merge queue list, conflict show, event/span queries, and guardrail check. Destructive-write examples include `crabdb.agent_apply`, `crabdb.agent_undo`, `crabdb.agent_rewind`, `crabdb.lane_rewind`, and `crabdb.merge_queue_run` because they intentionally move lane or shared branch refs and may refresh materialized workdirs.

Open-world write examples are `crabdb.agent_test`, `crabdb.agent_eval`,
`crabdb.run_test`, and `crabdb.run_eval` because they execute commands in lane
or task workdirs.

MCP requests use strict JSON-RPC envelopes and strict JSON params/arguments.
Unknown top-level request fields are rejected except the reserved `_meta` object.
Unknown tool-call param fields are rejected except the reserved `_meta` object,
and unknown argument fields are returned as MCP tool errors instead of being
ignored. Resource, prompt, and completion params use the same strict field
handling. Mutating tool attempts are recorded in `external_mutation_audit` with
actor `mcp:stdio`, the tool name, success/error status, inferred
lane/ref/change ids, and a small redacted summary. Turn-scoped mutation failures
can still be attributed through the
`turn_id` argument. Read-only tools are guarded and are not written to this audit
table.

## Prompts

- `crabdb.lane_task`
- `crabdb.review_lane`
- `crabdb.resolve_conflict`

## Resources

Static resources include workspace status, doctor, lanes, merge queue,
conflicts, OpenAPI, documentation, and agent task dashboard resources:
`crabdb://workspace/agent-tasks`,
`crabdb://workspace/agent-tasks/latest/review`,
`crabdb://workspace/agent-tasks/latest/changes`,
`crabdb://workspace/agent-tasks/latest/files`, and
`crabdb://workspace/agent-tasks/latest/focus`. Resource templates cover
individual agent task review/changes/file/report/focus dashboards, lanes, lane
review packets, sessions, turns, conflicts, approvals, run states, and trace
spans.

## Code Facts Used

- Tools: `crates/crabdb/src/mcp/tools`
- Tool calls: `crates/crabdb/src/mcp/tool_call`
- Risk annotations: `crates/crabdb/src/mcp/tools/annotations.rs`
- Prompts/resources: `crates/crabdb/src/mcp/capabilities`

# MCP Tools Reference

Trail MCP tool names are stable strings under the `trail.` prefix.
The stdio transport expects one UTF-8 JSON-RPC object per line. Each input line
is limited to 16 MiB; oversized or non-UTF-8 lines return JSON-RPC parse errors,
and the server continues reading subsequent requests.

## Core

- `trail.doctor`
- `trail.status`
- `trail.diff`
- `trail.timeline`
- `trail.why`
- `trail.history`
- `trail.code_from`
- `trail.config_list`
- `trail.config_get`
- `trail.config_set`
- `trail.ignore_list`
- `trail.ignore_add`
- `trail.ignore_remove`
- `trail.ignore_check`
- `trail.guardrail_check`

## Agent Tasks

- `trail.agent_status`
- `trail.agent_inbox`
- `trail.agent_board`
- `trail.agent_stack`
- `trail.agent_next`
- `trail.agent_guide`
- `trail.agent_dashboard`
- `trail.agent_review_data`
- `trail.agent_review_flow`
- `trail.agent_ask`
- `trail.agent_view`
- `trail.agent_brief`
- `trail.agent_summary`
- `trail.agent_validate`
- `trail.agent_test_plan`
- `trail.agent_report`
- `trail.agent_handoff`
- `trail.agent_receipt`
- `trail.agent_pr`
- `trail.agent_story`
- `trail.agent_tools`
- `trail.agent_impact`
- `trail.agent_review_map`
- `trail.agent_risk`
- `trail.agent_confidence`
- `trail.agent_ready`
- `trail.agent_diagnose`
- `trail.agent_test`
- `trail.agent_eval`
- `trail.agent_workdir`
- `trail.agent_changes`
- `trail.agent_delta`
- `trail.agent_new`
- `trail.agent_mark_reviewed`
- `trail.agent_mark_file_reviewed`
- `trail.agent_archive`
- `trail.agent_unarchive`
- `trail.agent_change`
- `trail.agent_timeline`
- `trail.agent_files`
- `trail.agent_file`
- `trail.agent_checkpoints`
- `trail.agent_why`
- `trail.agent_turn`
- `trail.agent_compare`
- `trail.agent_diff`
- `trail.agent_review`
- `trail.agent_focus`
- `trail.agent_apply`
- `trail.agent_finish`
- `trail.agent_rewind`
- `trail.agent_undo`

These are the high-level tools an editor agent should prefer when the user asks
questions like "help me use Trail?", "show agent board?", "which task first?", "show dashboard?", "walk me through review?", "go/no-go?", "am I good?", "what should I do next?", "what did the agent do?", "what did
the agent change?", "what files did it touch?", "what should I review first?",
"what should I put in the PR?", "handoff this to another agent?", "give me a summary to share?", "what commit
message should I use?", "is it tested?", "how should I test this?", "what tools were used?", "what is the blast radius?", "review map?", "review files?", "what changed?", "show the diff?", "show the last turn
diff", "what needs attention?", "where is the workdir?", "where did the agent edit?", "which prompt changed README.md?", "last prompt?", "what changed in the last prompt?", "what changed in README.md in the last prompt?", "show transcript?", "what file should I review first?", "what file should I open?", "where should I look first?", "open review?", "review this task?", "what tests should I run?", "validation plan?", "can I merge?", "why can't I apply?", "what is blocking this task?", "why did it fail?", "what went wrong?", "any red flags?", "what should I worry about?", "which files are risky?", or "is this ready to apply?". `trail.agent_ask` is the
lowest-burden front door: pass a plain-language question and it deterministically
routes to the right read-only report, returning the routed tool name and payload.
It will route apply/merge/land questions to readiness and rewind/undo questions
to checkpoint or diagnosis views rather than mutating state. Patch and diff
questions such as "show the diff", "show last patch", "show turn diff", or
"show changes by file", or "show patch for README.md" route to whole-task or
focused patch reports.
`trail.agent_inbox` groups all
tasks by the action they need and returns one primary next command. Its
structured `items` include each task's attention state, new files/lines since
the last review, and an optional `review_first` target for editor dashboards.
`trail.agent_board` presents the same multi-task evidence as low-noise columns
for needs-record, conflicted, blocked, needs-review, ready, running, applied,
and archived tasks. Use it for editor sidebars or "show all agents" requests
where the user wants orientation before details.
`trail.agent_stack` is the apply-order view: it finds files changed by more
than one task, ranks non-overlapping apply candidates by risk and change size,
and returns one next command. Use it for "which task first?" or "what can I
apply safely?" requests.
`trail.agent_next` returns one primary next command plus a few alternatives.
`trail.agent_guide` returns the compact "how do I use Trail from here?"
workflow: current state, one next command, setup/review/apply or recovery steps,
and the small mental model to show in editor panels.
`trail.agent_ask` routes "show actions", "what can I do?", "show buttons", and
similar action-palette questions to structured action data so hosts do not need
the user to remember `agent action`.
`trail.agent_dashboard` returns one compact task board with next action, focus
file, optional open command, validation status, changed files, risk, and apply
readiness. `trail.agent_review_data` returns one editor-friendly structured
packet with file review progress, focus file, review map, changes by file,
confidence, validation, risk, readiness, and typed `actions` so hosts do not
need to stitch multiple reports together or parse suggestions into buttons.
Each action includes stable ids, enabled state, disabled reason, safety class,
exact command, optional `mcp_tool`, and optional `mcp_arguments` for direct MCP
execution. CLI/editor hosts can also show the same command palette with
`trail agent action`, run a published id through `trail agent action <id>` for
the latest task, or use `trail agent action <task> <id>` for a specific task;
confirmation-required actions still need `--confirm`.
`trail.agent_review_flow` returns the procedural checklist for one
task: inspect new changes, mark the checkpoint reviewed, validate, and preview
finish/apply. Use it for editor "review checklist" or "walk me through review"
requests. `trail.agent_status` and
`trail.agent_brief` include embedded risk reports so an editor can show the
safety signal without making a second tool call. `trail.agent_summary` returns
the one-page post-run cockpit with readiness, risk, validation, receipt
Markdown, PR draft, Git preflight, and next commands. `trail.agent_validate`
returns a read-only validation guide with latest test/eval gates and suggested
`agent_test`/`agent_eval` commands; use it before running open-world commands.
`trail.agent_test_plan` returns the actionable validation checklist: ranked
test/eval steps, exact commands, affected paths, and reasons derived from
changed areas, impact, risk, and existing gates. Use it for "what tests should I
run?" and keep `trail.agent_validate` for gate status questions.
`trail.agent_story` returns one
plain-language account of what happened, with turn summaries, changed files,
tools, notes, and next action. `trail.agent_tools` returns a focused tool
activity report with available ACP commands, grouped tool calls, statuses,
turns, checkpoints, and changed files around tool-heavy turns.
`trail.agent_impact` returns the blast-radius view for a task: changed impact
areas, highest impact, validation state, risk, and recommended review/test
checks. Use it for "what areas did this touch?" or "what should I test because
of these changes?" requests.
`trail.agent_review_map` returns the file-by-file review checklist grouped by
changed area, with per-file focus, why, patch, and optional editor-open commands.
Use it for "review map", "review files", or "file checklist" requests.
`trail.agent_risk` returns a deterministic
low/medium/high/blocking risk level with reasons and mitigation commands before
apply. `trail.agent_confidence` returns a go/no-go verdict and score from
review freshness, validation, risk, and Git apply preflight; use it for "am I
good?", "final check", or "should I ship?" requests. `trail.agent_ready`
returns a read-only apply preflight that combines task readiness, risk, Git
dry-run status, blockers, warnings, and one next command.
`trail.agent_diagnose` explains a likely issue, supporting evidence,
friendly recovery targets, and safe inspection/recovery commands before an
editor suggests destructive undo or rewind. `trail.agent_test` and
`trail.agent_eval` run commands in the task
workdir and record durable gates without requiring the caller to know the lane
name. `trail.agent_brief` returns a compact task review packet with next
action, readiness, changed files, turn summaries, latest diff stats, and tools.
`trail.agent_report` returns a shareable review bundle with story, risk,
changes, transcript, readiness, suggestions, and Markdown.
`trail.agent_handoff` returns the receiver-friendly packet for another human or
agent: current state, next step, review commands, validation, risks, changed
files, turns, tools, related packet commands, and Markdown.
`trail.agent_receipt` returns the easier post-run artifact: summary,
validation gates, changed files, turns, tools, risk, checkpoint, next command,
and a Markdown receipt string.
`trail.agent_pr` returns a read-only pull request draft title and body generated
from the same recorded task state. It does not create a remote PR.
`trail.agent_changes` returns one primary `next` command, deterministic change
cards, then raw turn/operation groups, so editor panels can show intent-level
review chunks without asking the user to connect checkpoints manually. Each card
includes `review_command`, `focus_command`, `why_command`, and `diff_command`
fields when available. Pass `by-file` when the editor wants one review card per
changed file.
`trail.agent_review` returns the review dashboard for an agent task: readiness,
risk, blockers, warnings, prioritized files to inspect first, and exact next
commands.
`trail.agent_focus` bundles the next file to inspect with its review priority,
prompt/tool explanation, optional materialized-task `open_path`/`open_command`,
and focused diff summary.
`trail.agent_workdir` returns the exact materialized task directory plus a
shell-safe `cd` command. `trail.agent_change` expands one change card by rank,
key, or title into files, provenance, tools, commands, and optional focused
patches. `trail.agent_delta` returns the newest completed turn or operation as
one card, with changed files, provenance, next commands, and optional focused
patches. `trail.agent_new` returns the changes since the latest reviewed marker
or the whole task when no marker exists. `trail.agent_mark_reviewed` writes the
whole-task marker at the current task checkpoint, while
`trail.agent_mark_file_reviewed` records that one changed file has been
reviewed for the current checkpoint and leaves the rest of the review map open.
`trail.agent_archive` hides a task from default inbox/list/latest views without
deleting its lane, transcript, checkpoints, or provenance;
`trail.agent_unarchive` restores it.
`trail.agent_timeline` returns the chronological
prompt/operation timeline with checkpoints, tools, changed files, and per-item
follow-up commands. `trail.agent_files` returns a file-centric review
view with the turns, prompts, checkpoints, and commands behind each changed
file. `trail.agent_file` inspects one path, which is useful for editor panels
that know the currently open file. `trail.agent_checkpoints` lists friendly rewind
targets and exact checkpoint ids before an editor calls destructive recovery
tools. `trail.agent_why` answers "why did this file change?"
with the related prompt/turn, checkpoint, tools, and a focused diff command.
`trail.agent_turn` returns one prompt-sized receipt with prompt and assistant
previews, messages, tools, checkpoint, changed files, and optional focused
patch.
`trail.agent_diff` accepts `file` to keep a task, turn, operation, or
checkpoint diff scoped to one changed path.
`trail.agent_compare` compares two tasks, highlights shared changed files and
one-sided changes, returns both risk reports, and recommends a next command. The
tools resolve `latest`, agent task names, lane
names, Trail session ids, and ACP session ids so users do not need to manually
connect prompts, checkpoints, and operation ranges. Agent task reports include a
human `title` for display and a stable `name`/`lane` for exact follow-up
commands. Materialized task reports include `workdir`, the exact directory where
the agent edited files.
`trail.agent_undo` is the easy recovery tool for "undo the last prompt" or
"undo the prompt containing this text." `trail.agent_rewind` accepts exact
checkpoints and friendly targets such as `before-last-turn`, `turn:2`,
`before-turn:2`, `prompt:<text>`, `before-prompt:<text>`, and
`before-last-operation`. Both undo and rewind are marked destructive so agent
hosts can ask for explicit confirmation before moving task state or refreshing
materialized workdirs.
`trail.agent_apply` and `trail.agent_finish` are also marked destructive
because non-dry-run apply can record a task workdir, create a Git commit, and
fast-forward the current Git branch. `agent_finish` also archives the task after
success. Hosts should call `trail.agent_ready` first, then call apply/finish in
dry-run mode, and require explicit confirmation before non-dry-run apply or
finish.

## Agent Prompts

- `trail.review_agent`
- `trail.recover_agent`
- `trail.apply_agent`

These prompts guide editor hosts through the common agent-task workflows using
the high-level `trail.agent_*` tools. They accept an optional `selector`
argument that defaults to `latest`. The review prompt starts with
`agent_summary` and file-focused inspection. The recovery prompt starts with
`agent_diagnose` before destructive undo/rewind. The apply prompt starts with
`agent_summary` and `agent_ready`, then requires explicit user confirmation
before non-dry-run apply.

## Lanes

- `trail.lane_spawn`
- `trail.lane_claim`
- `trail.lane_list`
- `trail.lane_show`
- `trail.lane_status`
- `trail.lane_review`
- `trail.lane_contribution`
- `trail.gate_history`
- `trail.lane_readiness`
- `trail.lane_refresh_preview`
- `trail.lane_handoff`
- `trail.lane_rewind`
- `trail.lane_remove`

## Sessions, Approvals, Runs, Leases, Anchors

- `trail.session_start`
- `trail.session_list`
- `trail.session_current`
- `trail.session_show`
- `trail.session_context`
- `trail.session_end`
- `trail.approval_request`
- `trail.approval_list`
- `trail.approval_show`
- `trail.approval_decide`
- `trail.run_pause`
- `trail.run_list`
- `trail.run_show`
- `trail.run_resume`
- `trail.lease_acquire`
- `trail.lease_list`
- `trail.lease_release`
- `trail.anchor_create`
- `trail.anchor_list`
- `trail.anchor_resolve`
- `trail.anchor_delete`

## Merge and Conflicts

- `trail.lane_merge_queue_add`
- `trail.lane_merge_queue_list`
- `trail.lane_merge_queue_run`
- `trail.lane_merge_queue_explain`
- `trail.lane_merge_queue_remove`
- `trail.conflict_list`
- `trail.conflict_show` returns conflict details plus deterministic explanation evidence and conservative next steps.
- `trail.conflict_resolve`

Conflict explanations include the stored `base_root`, `target_root`, and
`source_root` snapshots used to reproduce the conflict, plus per-path
`conflict_class` values such as `modify/modify`, `delete/modify`,
`rename/modify`, `binary`, `mode`, and `same_insertion_gap`.
They can also include `known_resolutions` when a path/content conflict
signature matches a previously resolved conflict.
`trail.conflict_resolve` requires exactly one of `take` or `manual`. Manual
file values can be plain strings or objects with only `content`, `delete`, and
`executable`; unknown keys are rejected.

## Turns, Events, Spans, Patches, Gates, Workdirs

- `trail.begin_turn`
- `trail.add_message`
- `trail.add_event`
- `trail.event_list`
- `trail.span_start`
- `trail.span_end`
- `trail.span_list`
- `trail.span_summary`
- `trail.span_show`
- `trail.apply_patch`
- `trail.end_turn`
- `trail.show_turn`
- `trail.diff_lane`
- `trail.run_test`
- `trail.run_eval`
- `trail.lane_hydrate`
- `trail.sync_workdir`
- `trail.read_file`

`trail.lane_spawn` accepts `workdir_mode` values `virtual`, `sparse`,
`full-cow`, `overlay-cow`, and `nfs-cow`. `virtual` creates no workdir, `sparse` requires
`paths`, and `full-cow` creates a full materialized workdir using filesystem
clone COW when available. `overlay-cow` creates an empty workdir mountpoint and
records an overlay backend; a runtime such as `trail agent start
--workdir-mode overlay-cow` mounts the FUSE view and keeps it alive while the
agent runs. Spawn results include `workdir_mode`, `cow_backend`, `sparse_paths`,
and `overlay_available`.
On macOS, `nfs-cow` reports `cow_backend: "nfs-overlay"` and uses the built-in
loopback NFS client.

`trail.apply_patch` accepts either native `edits` or compatibility `files`;
provide exactly one non-empty array. Native edit objects and compatibility file
objects reject unknown keys, and line-id edits require `expected_text`.

`trail.lane_hydrate` hydrates selected paths into a sparse lane workdir before
filesystem edits. It uses the same dirty-workdir checks as path-scoped
`trail.sync_workdir`.

`trail.sync_workdir` returns `rescue_workdir` when a forced sync overwrites
dirty materialized workdir files or replaces a non-directory file at the lane
workdir path. That directory contains copied recoverable regular files and
`manifest.json`.

## Tool Risk Annotations

The MCP layer annotates tools as read-only, workspace write, destructive write, or open-world write. Read-only tool calls also run under Trail's read-only guard, so they must not persist SQLite database changes or mutate Trail sidecars such as `config.toml`, `HEAD`, ref files, daemon endpoint/token/cache files, or lane workdir metadata manifests while serving the request. The `trail://workspace/status` resource uses the same non-mutating status path. Read-only examples include status, diff, timeline, why, history, agent status/inbox/board/stack/next/guide/dashboard/review-data/review-flow/ask/view/brief/validate/test-plan/report/handoff/story/tools/impact/review-map/risk/confidence/ready/workdir/changes/files/checkpoints/why/turn/compare/diff/review/focus, lane status, review, readiness, handoff, sessions, approvals, runs, leases, anchors, lane merge queue list, conflict show, event/span queries, and guardrail check. Workspace-write examples include `trail.agent_mark_reviewed`, `trail.agent_mark_file_reviewed`, `trail.agent_archive`, and `trail.agent_unarchive` because they write task metadata markers without deleting provenance. Destructive-write examples include `trail.agent_apply`, `trail.agent_finish`, `trail.agent_undo`, `trail.agent_rewind`, `trail.lane_rewind`, and `trail.lane_merge_queue_run` because they intentionally move lane or shared branch refs and may refresh materialized workdirs.

Open-world write examples are `trail.agent_test`, `trail.agent_eval`,
`trail.run_test`, and `trail.run_eval` because they execute commands in lane
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

- `trail.lane_task`
- `trail.review_lane`
- `trail.resolve_conflict`

## Resources

Static resources include workspace status, doctor, lanes, merge queue,
conflicts, OpenAPI, documentation, and agent task dashboard resources:
`trail://workspace/agent-tasks`,
`trail://workspace/agent-tasks/latest/review`,
`trail://workspace/agent-tasks/latest/review-data`,
`trail://workspace/agent-tasks/latest/changes`,
`trail://workspace/agent-tasks/latest/files`, and
`trail://workspace/agent-tasks/latest/focus`. Resource templates cover
individual agent task review-data/review/changes/file/report/focus dashboards, lanes, lane
review packets, sessions, turns, conflicts, approvals, run states, and trace
spans.

## Code Facts Used

- Tools: `trail/src/mcp/tools`
- Tool calls: `trail/src/mcp/tool_call`
- Risk annotations: `trail/src/mcp/tools/annotations.rs`
- Prompts/resources: `trail/src/mcp/capabilities`

# ACP Agent Usage Runbook

Trail can run in front of a real ACP coding agent and record the agent's work
as lane sessions, turns, tool events, and checkpoints. This runbook is for
humans and automation agents that need to verify ACP behavior end to end.

The examples use Claude Code through the ACP adapter, but Trail also ships a
Codex profile and the relay side is provider-neutral.

## Mental Model

- **Agent task**: the easy-path unit a user starts, views, applies, or rewinds.
  Trail backs each task with a fresh lane by default.
- **ACP agent**: the real coding agent, such as Claude Code or Codex.
- **ACP client/editor**: the process that sends `initialize`, `session/new`,
  `session/prompt`, permission responses, and cancellation messages.
- **Trail ACP relay**: the process between the client and the real agent.
- **Lane**: Trail's branch-like workspace for one agent or task.
- **Turn**: one prompt/response/tool cycle.
- **Checkpoint**: the recorded lane state after a completed, cancelled, or
  failed turn.

For day-to-day editor use, generate a stable high-level command:

```sh
trail agent setup
```

The generated editor entry runs:

```sh
trail --workspace /path/to/repo agent acp --provider claude-code
```

Use `--provider codex` for the built-in Codex ACP adapter:

```sh
trail --workspace /path/to/repo agent acp --provider codex
```

That command creates a fresh lane for the ACP session, launches the real
provider, captures the transcript and tools, and records the workdir checkpoint.

For Claude Code the low-level relay command is:

```sh
trail acp relay --provider claude-code --materialize -- \
  npx -y @agentclientprotocol/claude-agent-acp@latest
```

For Codex the low-level relay command is:

```sh
trail acp relay --provider codex --materialize -- \
  npx -y @agentclientprotocol/codex-acp@latest
```

`--materialize` is the default mode for practical coding-agent work. Trail
creates a lane workdir under `.trail/worktrees/<lane>` and points the upstream
ACP session `cwd` there. The agent edits normal files, and Trail records the
lane checkpoint at the end of the prompt.

## Prerequisites

Build or install Trail:

```sh
make install
trail --help
```

Confirm the provider profile:

```sh
trail agent doctor --provider claude-code
trail agent doctor --provider codex
trail agent setup
trail acp list
```

Provider doctor validates the Trail workspace, the provider profile, the relay
command shape, and upstream command availability.

## Create a Playground Repo

Use a disposable copy for real-agent experiments:

```sh
PLAYGROUND="$(mktemp -d "${TMPDIR:-/tmp}/trail-acp-playground.XXXXXX")"
rsync -a \
  --exclude target \
  --exclude .git \
  --exclude .trail \
  --exclude prolly-tree-paper.pdf \
  ./ "$PLAYGROUND/"

git -C "$PLAYGROUND" init
git -C "$PLAYGROUND" add .
git -C "$PLAYGROUND" commit -m "baseline playground copy"

trail --workspace "$PLAYGROUND" init --working-tree
trail --workspace "$PLAYGROUND" agent doctor --provider claude-code
```

This gives the real agent a large enough repository to exercise file discovery
and editing without touching your active branch.

## Configure an ACP Editor

Prefer the high-level setup command:

```sh
trail agent setup
trail agent setup --provider claude-code --editor zed
trail agent setup --provider codex --editor zed
```

These snippets use `trail agent acp`, so users do not hard-code or rotate lane
names manually. Use `trail acp install` only when you intentionally want the
lower-level relay command.

### Zed

Zed supports ACP External Agents natively. Generate the Trail custom-agent
snippet:

```sh
trail agent setup --provider claude-code --editor zed
```

The generated shape is:

```json
{
  "agent_servers": {
    "trail-claude-code": {
      "type": "custom",
      "command": "trail",
      "args": [
        "--workspace",
        "/path/to/repo",
        "agent",
        "acp",
        "--provider",
        "claude-code"
      ]
    }
  }
}
```

If Zed cannot find `trail`, replace `"command": "trail"` with the absolute
path from `which trail`.

In Zed, open `agent: open settings`, add the `agent_servers` entry, then start
a new External Agent thread for `trail-claude-code`. After the prompt finishes,
run:

```sh
trail agent
trail agent board
trail agent stack
trail agent ask what needs attention
trail agent ask what should I do next
trail agent ask what did the agent do
trail agent ask where is the workdir
trail agent ask where did the agent edit
trail agent ask which prompt changed README.md
trail agent ask last prompt
trail agent ask what changed in the last prompt
trail agent ask what changed in README.md in the last prompt
trail agent ask show transcript
trail agent ask show dashboard
trail agent ask show actions
trail agent ask what should I review
trail agent ask what should I review first
trail agent ask what file should I review first
trail agent ask what file should I open
trail agent ask where should I look first
trail agent ask open review
trail agent ask review this task
trail agent ask what tools were used
trail agent tools latest
trail agent ask what is the blast radius
trail agent impact latest
trail agent ask review map
trail agent review-map latest
trail agent ask what did the agent change
trail agent ask what files did it touch
trail agent ask can I merge
trail agent ask why can't I apply
trail agent ask what is blocking this task
trail agent ask why did it fail
trail agent ask what went wrong
trail agent ask any red flags
trail agent ask what should I worry about
trail agent ask which files are risky
trail agent todo
trail agent ask what changed since I looked
trail agent ask what should I put in the PR
trail agent ask give me a summary to share
trail agent ask handoff this to another agent
trail agent ask what commit message should I use
trail agent ask explain README.md
trail agent ask show the diff
trail agent ask show changes by file
trail agent ask show patch for README.md
trail agent ask show turn diff
trail agent story latest
trail agent dashboard latest
trail agent review-data latest
trail agent review-flow latest
trail agent ask walk me through review
trail agent risk latest
trail agent confidence latest
trail agent ask final check, am I good?
trail agent can-land latest
trail agent ask what tests should I run
trail agent ask is it tested
trail agent ask how should I test this
trail agent test-plan latest
trail agent validate latest
trail agent test latest -- cargo test
trail agent brief latest
trail agent workdir latest
trail agent changes latest
trail agent changes latest --by-file
trail agent last latest
trail agent what-changed latest
trail agent mark-file-reviewed latest README.md
trail agent turn
trail agent turn-diff latest --patch
trail agent changed-files latest
trail agent explain README.md
trail agent turn-diff latest --file README.md --patch
trail agent review-plan latest
trail agent focus latest
trail agent open latest
trail agent view latest
trail agent can-land latest
trail agent land latest
trail agent finish latest
```

### VS Code

VS Code needs an ACP client extension. Configure the extension's custom ACP
agent command to launch Trail:

```text
command: trail
args:
  --workspace
  /path/to/repo
  agent
  acp
  --provider
  claude-code
```

The exact settings key depends on the ACP extension. The important boundary is
the same: VS Code or the extension is the ACP client, Trail is the ACP relay,
and Claude Code remains the real ACP agent.

## Run a Real Claude Code ACP Prompt

An ACP-capable editor normally drives the relay for you. For direct testing,
you can use a tiny JSON-RPC driver. The important protocol details are:

- Use high client request ids, such as `1000`, `1001`, and `1002`, so they do
  not collide with agent-to-client permission request ids.
- Treat `session/request_permission` as a request from the agent to the client.
- Respond to permission requests with the ACP response shape:

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "result": {
    "outcome": {
      "outcome": "selected",
      "optionId": "allow"
    }
  }
}
```

Do not respond with `{ "optionId": "allow" }` directly; Claude Code treats that
as a refused or invalid permission response.

The request sequence is:

```text
client -> relay: initialize
client -> relay: session/new
client -> relay: session/prompt
relay  -> client: session/update events
relay  -> client: session/request_permission, if needed
client -> relay: permission response
relay  -> client: final session/prompt response
```

Use a bounded prompt for the first real edit:

```text
Make one small real edit in this Trail playground repo.

Edit exactly one file: docs/integrations/acp.md.
Add a short section titled "Playground smoke test" near the setup or
troubleshooting area.
Mention that a real Claude Code ACP run can be verified with
`trail agent view latest` and `trail agent ready latest`.
Keep it concise: 2-4 sentences.
Do not modify any other files.
After editing, reply with a brief summary and the file path changed.
```

Launch the high-level ACP entrypoint:

```sh
trail --workspace "$PLAYGROUND" agent acp --provider claude-code
```

Then drive the relay over JSON-RPC stdio from your editor or test client.

## Verify a Real Edit

After the prompt completes:

```sh
trail --workspace "$PLAYGROUND" agent board
trail --workspace "$PLAYGROUND" agent stack
trail --workspace "$PLAYGROUND" agent ask what needs attention
trail --workspace "$PLAYGROUND" agent ask what should I do next
trail --workspace "$PLAYGROUND" agent summary latest
trail --workspace "$PLAYGROUND" agent ask what did the agent do
trail --workspace "$PLAYGROUND" agent ask where is the workdir
trail --workspace "$PLAYGROUND" agent ask where did the agent edit
trail --workspace "$PLAYGROUND" agent ask which prompt changed README.md
trail --workspace "$PLAYGROUND" agent ask last prompt
trail --workspace "$PLAYGROUND" agent ask what changed in the last prompt
trail --workspace "$PLAYGROUND" agent ask what changed in README.md in the last prompt
trail --workspace "$PLAYGROUND" agent ask show transcript
trail --workspace "$PLAYGROUND" agent ask what should I review
trail --workspace "$PLAYGROUND" agent ask what should I review first
trail --workspace "$PLAYGROUND" agent ask what file should I review first
trail --workspace "$PLAYGROUND" agent ask what file should I open
trail --workspace "$PLAYGROUND" agent ask where should I look first
trail --workspace "$PLAYGROUND" agent ask open review
trail --workspace "$PLAYGROUND" agent ask review this task
trail --workspace "$PLAYGROUND" agent ask what tools were used
trail --workspace "$PLAYGROUND" agent ask can I merge
trail --workspace "$PLAYGROUND" agent ask why can't I apply
trail --workspace "$PLAYGROUND" agent ask what is blocking this task
trail --workspace "$PLAYGROUND" agent ask why did it fail
trail --workspace "$PLAYGROUND" agent ask what went wrong
trail --workspace "$PLAYGROUND" agent ask any red flags
trail --workspace "$PLAYGROUND" agent ask what should I worry about
trail --workspace "$PLAYGROUND" agent ask which files are risky
trail --workspace "$PLAYGROUND" agent ask what changed since I looked
trail --workspace "$PLAYGROUND" agent ask explain README.md
trail --workspace "$PLAYGROUND" agent ask show changes by file
trail --workspace "$PLAYGROUND" agent ask show patch for README.md
trail --workspace "$PLAYGROUND" agent ask show turn diff
trail --workspace "$PLAYGROUND" agent changes latest
trail --workspace "$PLAYGROUND" agent last latest
trail --workspace "$PLAYGROUND" agent what-changed latest
trail --workspace "$PLAYGROUND" agent can-land latest
trail --workspace "$PLAYGROUND" agent recover latest
trail --workspace "$PLAYGROUND" agent handoff latest
trail --workspace "$PLAYGROUND" agent receipt latest
trail --workspace "$PLAYGROUND" agent pr latest
trail --workspace "$PLAYGROUND" agent turn-diff latest --file README.md --patch
trail --workspace "$PLAYGROUND" agent review-plan latest
trail --workspace "$PLAYGROUND" agent view latest
trail --workspace "$PLAYGROUND" agent land latest --dry-run
trail --workspace "$PLAYGROUND" agent finish latest --dry-run
```

Expected signals:

- `agent` opens the inbox home view, groups all agent tasks by what needs
  attention, shows new files/lines since the last review, names the first file
  to inspect, and prints one primary next command.
- `agent inbox` is the explicit form of the same grouped home view.
- `agent board` is the low-noise multi-agent view: it presents tasks as
  needs-record, conflicted, blocked, needs-review, ready, running, applied, and
  archived columns, with one next command.
- `agent stack` is the apply-order view: it finds shared changed files across
  tasks and tells you whether to compare overlap or preview finishing the safest
  task first.
- `agent handoff latest` prints a receiver-friendly Markdown packet for another
  human or agent, while `agent receipt latest` stays the shorter after-action
  note.
- `agent finish latest` applies a ready task and archives it after success.
  `agent close latest` only archives a task that has already been dealt with.
  Archived tasks disappear from default `agent`, `agent inbox`, `agent board`,
  `agent list`, and `latest` views without deleting transcripts, checkpoints,
  or provenance. Use `agent inbox --all`, `agent board --all`, and
  `agent unarchive <TASK>` when you need one back.
- `agent status` shows the latest task status, risk, and next useful action.
- `agent ask ...` routes plain-language questions to the right existing view.
  Try `what needs attention`, `what should I do next`, `what just changed`,
  `what changed since I looked`, `what did the agent do`,
  `where is the workdir`, `where did the agent edit`,
  `which prompt changed README.md`, `last prompt`,
  `what changed in the last prompt`,
  `what changed in README.md in the last prompt`, `show transcript`,
  `what should I review`,
  `what should I review first`, `open review`, `review this task`,
  `what file should I review first`, `what file should I open`,
  `where should I look first`,
  `what tools were used`, `changed files`, `what did the agent change`,
  `what files did it touch`, `what should I put in the PR`,
  `give me a summary to share`, `handoff this to another agent`,
  `what commit message should I use`,
  `is it tested`, `how should I test this`, `validation plan`,
  `can I merge`,
  `is it safe to land`, `why can't I apply`,
  `what is blocking this task`, `why did it fail`, `what went wrong`,
  `any red flags`, `what should I worry about`, `which files are risky`,
  `recover`, or
  `explain README.md`. Patch and diff phrasing such as `show the diff`,
  `show changes by file`, `show turn diff`, and `show patch for README.md`
  routes to whole-task or focused patch views.
- Bare `agent` shows the current task dashboard when there is one task, the
  grouped inbox when there are multiple tasks, and setup guidance when there
  are none.
- `agent todo` shows one primary command for the latest task. It is an alias for
  `agent next`.
- `agent dashboard latest` shows the compact daily-use task board with next
  action, focus file, open command, validation, changed files, risk, and apply
  readiness.
- `agent review-data latest` is the editor side-panel packet: file review
  progress, focus file, review map, changes by file, confidence, validation,
  risk, readiness, typed actions, and next commands in one structured report.
  Each action has a stable id, label, safety class, enabled state, exact command,
  disabled reason, optional MCP tool, and optional MCP arguments so an editor can
  render and execute buttons without guessing or parsing shell text. Use
  `agent action` to list actions for the latest task, `agent action <id>` to run
  one, or `agent action <task> <id>` for a specific task; add `--print` to
  inspect the command and `--confirm` for validation/apply actions that require
  explicit confirmation. Before the first task exists, `agent action` shows
  runnable setup, doctor, and terminal-start actions instead of failing; for
  example, `trail agent action setup_vscode` prints the VS Code setup report.
- `agent review-flow latest` walks the task through inspect, mark reviewed,
  validate, and finish as one checklist. Use this after an editor agent finishes
  when you do not want to remember which Trail command comes next.
- `agent summary latest` shows the fuller one-page post-run cockpit with readiness,
  risk, validation, Git preflight, receipt Markdown, PR draft, and next command.
- `agent story latest` explains what happened in one plain-language task
  summary with turns, changed files, tools, notes, and next action.
- `agent tools latest` shows tool calls, available ACP commands, statuses,
  turns, checkpoints, and changed files around tool activity without reading the
  whole transcript.
- `agent impact latest` shows the blast radius: touched areas, highest impact,
  risk, validation state, and recommended review/test checks.
- `agent review-map latest` shows the code-review checklist grouped by changed
  area, with per-file focus, why, patch, and editor-open commands. Use
  `agent mark-file-reviewed latest <PATH>` as each file is inspected; the file
  stays reviewed until a later checkpoint changes that path again.
- `agent risk latest` shows a deterministic apply risk level, reasons, and
  mitigation commands before you touch Git.
- `agent confidence latest` gives one go/no-go verdict from review freshness,
  validation, risk, and Git apply preflight. Use this when the question is
  "am I good?" rather than a specific risk or readiness check.
- `agent validate latest` shows latest test/eval gates and suggested validation
  commands without running anything. It is also available as `agent tests`.
- `agent test-plan latest` turns changed areas, impact, risk, and existing gates
  into ranked test/eval steps with exact commands, affected paths, and reasons.
  Use this for "what tests should I run?" and `agent validate` for gate status.
- `agent test latest -- cargo test` records a durable test gate in the task
  workdir without requiring the lane name.
- `agent brief latest` shows one compact review packet with next action,
  changes, tools, and risks.
- `agent receipt latest` prints a copyable post-run receipt with validation,
  changed files, turns, tools, risk, checkpoint, and next command.
- `agent handoff latest` prints a copyable handoff packet with receiver next
  step, review commands, validation, risks, changed files, turns, and tools.
- `agent pr latest` prints a read-only pull request draft title and body from
  the recorded task state without creating a remote PR.
- `agent report latest --markdown` prints a copyable review/handoff report with
  summary, readiness, risk, changed files, turns, tools, and next command.
- `agent can-land latest` checks task readiness, risk, Git preflight, blockers,
  warnings, and the next safe command without mutating Git. It is an alias for
  `agent ready latest`.
- `agent recover latest` explains likely issues, evidence, friendly recovery
  targets, and safe commands before destructive undo or rewind. It is an alias
  for `agent diagnose latest`.
- `agent workdir latest` shows the materialized directory where the agent edited
  files and a copyable `cd` command.
- `agent changes latest` shows one primary next command, high-level change
  cards, and the prompt/turn and checkpoint details behind each card. Each card
  includes ready review/focus/why/diff commands.
- `agent changes latest --by-file` switches the same review map to one card per
  changed file, which is useful when you are already looking at a file list in
  the editor.
- `agent last latest` shows the newest completed turn or operation. It is an
  alias for `agent delta latest`. Use `--patch` or `--file <PATH> --patch` when
  you want the fresh patch without reading the whole task.
- `agent what-changed latest` shows what changed since the task was last marked
  reviewed. It is an alias for `agent new latest`. Use `agent done latest` after
  inspection to make the current checkpoint the next baseline.
- `agent turn` shows the latest completed prompt-sized turn with prompt,
  assistant summary, tools, checkpoint, changed files, and optional patch.
- `agent turn-diff latest` shows the latest completed prompt-sized code diff
  without requiring `agent diff --last-turn`; add `--turn`, `--file`, and
  `--patch` as needed.
- `agent changed-files latest` shows changed files with the turns, prompts,
  checkpoints, and commands behind each file. It is an alias for
  `agent files latest`.
- `agent rewind-points latest` lists friendly rewind targets such as
  `before-turn:1` with exact checkpoint ids and ready-to-run rewind commands. It
  is an alias for `agent checkpoints latest`.
- `agent explain README.md` explains which prompt, turn, tools, and checkpoint
  changed one file, and prints the focused diff command. It is an alias for
  `agent why README.md`.
- `agent compare <A> <B>` compares two agent tasks, shared changed files,
  one-sided changes, risk, and a recommended next command.
- `agent stack` generalizes compare across all active tasks when you need apply
  order or overlap checks.
- `agent turn-diff latest --file README.md --patch` shows the focused file
  patch for the most recent prompt.
- `agent review-plan latest` shows readiness, risk, blockers, warnings, and the
  highest-priority files to inspect first.
- `agent focus latest` picks the next file to inspect and bundles why, review
  priority, a materialized-task open command, and a focused diff summary.
- `agent open latest` opens that focused file in `$EDITOR` for a materialized
  task. Add `--print` when you only want the command.
- `agent view latest` shows the user prompt, assistant response, tool summaries, and
  checkpoint id.
- `agent can-land latest` shows the safe Git apply preflight.
- `agent land latest` applies using a generated Git commit message from the task
  title. It is an alias for `agent apply latest`; pass `-m` only when you want
  to override it. If the task has already been applied, Trail reports
  `already_applied` and points you to `agent continue` for follow-up work.
- `agent finish latest` applies with the same safety checks as `agent land`,
  then hides the finished task from the default inbox. Use it when you do not
  need the applied task to stay visible.
- `agent continue latest` creates a fresh task from the latest checkpoint, so
  another round of edits does not reuse already-applied lane history.
- `Changed paths` contains only the file you intended the agent to edit.

When in doubt, start with:

```sh
trail --workspace "$PLAYGROUND" agent todo
```

It prints one primary command for the current task and a few alternatives.

Inside an ACP editor chat, ask the real agent to use Trail's injected MCP tools:

```text
Ask Trail what changed since I looked.
Ask Trail what the agent did.
Ask Trail what needs attention.
Ask Trail where the workdir is.
Ask Trail where the agent edited files.
Ask Trail which prompt changed README.md.
Ask Trail for the last prompt.
Ask Trail what changed in the last prompt.
Ask Trail what changed in README.md in the last prompt.
Ask Trail to show the transcript.
Ask Trail what I should review first.
Ask Trail what file I should review first.
Ask Trail what file I should open.
Ask Trail where I should look first.
Ask Trail to open review.
Ask Trail to review this task.
Ask Trail what tools were used.
Ask Trail to show tool activity.
Ask Trail if I can merge.
Ask Trail why I can't apply.
Ask Trail what is blocking this task.
Ask Trail why it failed.
Ask Trail what went wrong.
Ask Trail if the latest task is safe to land.
Ask Trail to explain README.md.
Ask Trail to show the patch for README.md.
What should I do next in Trail?
Show Trail agent status.
Show my Trail agent inbox.
Show a Trail brief for the latest agent task.
Create a Trail Markdown report for the latest agent task.
Show what changed in the latest Trail agent task.
Show files changed by the latest Trail agent task.
Show Trail rewind targets for the latest agent task.
Explain why README.md changed in the latest Trail agent task.
Compare two Trail agent tasks.
Ask Trail what tests I should run.
Show Trail validation guidance for the latest agent task.
Run a Trail test gate for the latest agent task with cargo test.
Show the latest Trail agent turn diff.
Ask Trail to show the latest turn diff.
Show the patch for the last Trail agent turn.
Show the Trail review plan for the latest agent task.
Review whether the latest Trail agent task is ready to apply.
```

ACP/MCP hosts can also expose Trail prompts directly:

```text
Use the trail.review_agent prompt for latest.
Use the trail.recover_agent prompt for latest.
Use the trail.apply_agent prompt for latest.
```

Plain-language "Ask Trail..." requests should use `trail.agent_ask`, which
routes to read-only task reports such as next actions, task story, review focus,
tools used, changes, readiness, recovery, file explanations, checkpoint targets,
focused patch/diff views, test plans, pull request drafts, and copyable
receipts. For validation guidance, editors should call `trail.agent_test_plan`
and present its checklist before calling `trail.agent_test` or
`trail.agent_eval`. For gate status, use `trail.agent_validate`.

Those map to `trail.agent_ask`, `trail.agent_next`, `trail.agent_status`,
`trail.agent_inbox`, `trail.agent_story`, `trail.agent_brief`, `trail.agent_report`, `trail.agent_workdir`,
`trail.agent_risk`, `trail.agent_confidence`, `trail.agent_ready`, `trail.agent_validate`, `trail.agent_test_plan`, `trail.agent_test`, `trail.agent_review_data`, `trail.agent_changes`, `trail.agent_delta`, `trail.agent_new`, `trail.agent_mark_reviewed`, `trail.agent_mark_file_reviewed`, `trail.agent_files`, `trail.agent_checkpoints`, `trail.agent_why`,
`trail.agent_turn`, `trail.agent_compare`, `trail.agent_stack`, `trail.agent_diff`, `trail.agent_review_flow`, `trail.agent_review`,
`trail.agent_focus`, and
`trail.agent_apply`.

Editors can also render dashboards directly from MCP resources:

```text
trail://workspace/agent-tasks
trail://workspace/agent-tasks/latest/review
trail://workspace/agent-tasks/latest/review-data
trail://workspace/agent-tasks/latest/changes
trail://workspace/agent-tasks/latest/files
trail://workspace/agent-tasks/latest/focus
trail://workspace/agent-tasks/<task-or-lane>/changes
trail://workspace/agent-tasks/<task-or-lane>/report
```

For a direct file diff against the playground baseline:

```sh
LANE="$(trail --workspace "$PLAYGROUND" --json agent view latest | jq -r .task.lane)"
LANE_WORKDIR="$PLAYGROUND/.trail/worktrees/$LANE"
git -C "$PLAYGROUND" show HEAD:docs/integrations/acp.md > /tmp/acp-baseline.md
diff -u /tmp/acp-baseline.md "$LANE_WORKDIR/docs/integrations/acp.md"
```

For structured verification:

```sh
trail --workspace "$PLAYGROUND" --json agent view latest | jq '{
  title: .task.title,
  task_id: .task.name,
  workdir: .task.workdir,
  lane_name: .transcript.lane_name,
  acp_session: .transcript.acp_session.acp_session_id,
  provider: .transcript.acp_session.provider,
  turn: .transcript.turns[0].turn.turn_id,
  status: .transcript.turns[0].turn.status,
  checkpoint: .transcript.turns[0].checkpoint,
  changed_event_paths: [
    .transcript.turns[0].events[]
    | select(.event_type == "workdir_recorded")
    | .payload.changed_paths
  ],
  thought_event_count: (
    [
      .transcript.turns[0].events[]
      | select(
          (.event_type | tostring | contains("thought")) or
          ((.payload | tostring) | contains("agent_thought"))
        )
    ] | length
  )
}'
```

`thought_event_count` should be `0`. The relay may observe private or thought
chunks in the live ACP stream, but those chunks must not be persisted.

## Rewind, Continue, or Merge

Inspect the prompt-to-checkpoint timeline:

```sh
trail --workspace "$PLAYGROUND" agent next
trail --workspace "$PLAYGROUND" agent changes latest
trail --workspace "$PLAYGROUND" agent turn
trail --workspace "$PLAYGROUND" agent turn-diff latest --file README.md --patch
```

If the agent went sideways, undo the task with a friendly target:

```sh
trail --workspace "$PLAYGROUND" agent undo-last latest
trail --workspace "$PLAYGROUND" agent undo-last latest --turn 2
trail --workspace "$PLAYGROUND" agent undo-last latest --prompt 'Add hook support'
```

Direct `agent rewind --to <CHECKPOINT_OR_LABEL>` still works, but
`agent undo-last` avoids copying `ch_...` ids from transcripts.

If the task is ready, check apply readiness:

```sh
trail --workspace "$PLAYGROUND" agent can-land latest
```

## Troubleshooting

### `agent doctor --provider claude-code` passes but no prompt runs

The doctor checks the provider profile and launch command. Run a real ACP
prompt through your editor or JSON-RPC driver, then inspect it with
`trail acp sessions`, `trail transcript <lane>`, and
`trail lane review <lane>`.

### The agent says permission was refused

Check the permission response shape. It must be:

```json
{
  "result": {
    "outcome": {
      "outcome": "selected",
      "optionId": "allow"
    }
  }
}
```

The direct shape below is not enough:

```json
{
  "result": {
    "optionId": "allow"
  }
}
```

### The final prompt response is confused with a permission request

Use high client request ids and only treat a message as the final prompt
response when it has the matching id and no `method` field:

```text
id == 1002 && method is absent && (result or error is present)
```

ACP is bidirectional JSON-RPC. Agent-to-client request ids can overlap with
client-to-agent request ids.

### The workspace root did not change

With `--materialize`, the real edit is in the lane workdir:

```sh
echo "$PLAYGROUND/.trail/worktrees/<lane>"
```

Use `trail lane review <lane>` or diff the materialized lane workdir against
the playground baseline.

### The transcript shows a cancelled turn

That means the relay flushed the open turn and checkpointed the lane when the
client disconnected or cancelled. Inspect it:

```sh
trail transcript <lane>
trail turn show <turn-id>
```

Then start a fresh lane or fresh turn.

# ACP Agent Usage Runbook

CrabDB can run in front of a real ACP coding agent and record the agent's work
as lane sessions, turns, tool events, and checkpoints. This runbook is for
humans and automation agents that need to verify ACP behavior end to end.

The examples use Claude Code through the official ACP adapter, but the CrabDB
side is provider-neutral.

## Mental Model

- **Agent task**: the easy-path unit a user starts, views, applies, or rewinds.
  CrabDB backs each task with a fresh lane by default.
- **ACP agent**: the real coding agent, such as Claude Code.
- **ACP client/editor**: the process that sends `initialize`, `session/new`,
  `session/prompt`, permission responses, and cancellation messages.
- **CrabDB ACP relay**: the process between the client and the real agent.
- **Lane**: CrabDB's branch-like workspace for one agent or task.
- **Turn**: one prompt/response/tool cycle.
- **Checkpoint**: the recorded lane state after a completed, cancelled, or
  failed turn.

For day-to-day editor use, generate a stable high-level command:

```sh
crabdb agent setup --provider claude-code --editor vscode
```

The generated editor entry runs:

```sh
crabdb --workspace /path/to/repo agent acp --provider claude-code
```

That command creates a fresh lane for the ACP session, launches the real
provider, captures the transcript and tools, and records the workdir checkpoint.

For Claude Code the low-level relay command is:

```sh
crabdb acp relay --provider claude-code --materialize -- \
  npx -y @agentclientprotocol/claude-agent-acp@latest
```

`--materialize` is the default mode for practical coding-agent work. CrabDB
creates a lane workdir under `.crabdb/worktrees/<lane>` and points the upstream
ACP session `cwd` there. The agent edits normal files, and CrabDB records the
lane checkpoint at the end of the prompt.

## Prerequisites

Build or install CrabDB:

```sh
make install
crabdb --help
```

Confirm the provider profile:

```sh
crabdb agent doctor --provider claude-code
crabdb agent setup --provider claude-code --editor vscode
crabdb acp list
```

`claude-code` doctor validates the CrabDB workspace, the provider profile, the
relay command shape, and upstream command availability.

## Create a Playground Repo

Use a disposable copy for real-agent experiments:

```sh
PLAYGROUND="$(mktemp -d "${TMPDIR:-/tmp}/crabdb-acp-playground.XXXXXX")"
rsync -a \
  --exclude target \
  --exclude .git \
  --exclude .crabdb \
  --exclude prolly-tree-paper.pdf \
  ./ "$PLAYGROUND/"

git -C "$PLAYGROUND" init
git -C "$PLAYGROUND" add .
git -C "$PLAYGROUND" commit -m "baseline playground copy"

crabdb --workspace "$PLAYGROUND" init --working-tree
crabdb --workspace "$PLAYGROUND" agent doctor --provider claude-code
```

This gives the real agent a large enough repository to exercise file discovery
and editing without touching your active branch.

## Configure an ACP Editor

Prefer the high-level setup command:

```sh
crabdb agent setup --provider claude-code --editor vscode
crabdb agent setup --provider claude-code --editor zed
```

These snippets use `crabdb agent acp`, so users do not hard-code or rotate lane
names manually. Use `crabdb acp install` only when you intentionally want the
lower-level relay command.

### Zed

Zed supports ACP External Agents natively. Generate the CrabDB custom-agent
snippet:

```sh
crabdb agent setup --provider claude-code --editor zed
```

The generated shape is:

```json
{
  "agent_servers": {
    "crabdb-claude-code": {
      "type": "custom",
      "command": "crabdb",
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

If Zed cannot find `crabdb`, replace `"command": "crabdb"` with the absolute
path from `which crabdb`.

In Zed, open `agent: open settings`, add the `agent_servers` entry, then start
a new External Agent thread for `crabdb-claude-code`. After the prompt finishes,
run:

```sh
crabdb agent
crabdb agent ask what needs attention
crabdb agent ask what should I do next
crabdb agent ask what did the agent do
crabdb agent ask where is the workdir
crabdb agent ask where did the agent edit
crabdb agent ask which prompt changed README.md
crabdb agent ask last prompt
crabdb agent ask what changed in the last prompt
crabdb agent ask what changed in README.md in the last prompt
crabdb agent ask show transcript
crabdb agent ask what should I review
crabdb agent ask what should I review first
crabdb agent ask open review
crabdb agent ask review this task
crabdb agent ask what tools were used
crabdb agent ask can I merge
crabdb agent todo
crabdb agent ask what changed since I looked
crabdb agent ask explain README.md
crabdb agent ask show patch for README.md
crabdb agent ask show turn diff
crabdb agent story latest
crabdb agent risk latest
crabdb agent can-land latest
crabdb agent ask what tests should I run
crabdb agent validate latest
crabdb agent test latest -- cargo test
crabdb agent brief latest
crabdb agent workdir latest
crabdb agent changes latest
crabdb agent last latest
crabdb agent what-changed latest
crabdb agent turn
crabdb agent turn-diff latest --patch
crabdb agent changed-files latest
crabdb agent explain README.md
crabdb agent turn-diff latest --file README.md --patch
crabdb agent review-plan latest
crabdb agent focus latest
crabdb agent view latest
crabdb agent can-land latest
crabdb agent land latest
```

### VS Code

VS Code needs an ACP client extension. Configure the extension's custom ACP
agent command to launch CrabDB:

```text
command: crabdb
args:
  --workspace
  /path/to/repo
  agent
  acp
  --provider
  claude-code
```

The exact settings key depends on the ACP extension. The important boundary is
the same: VS Code or the extension is the ACP client, CrabDB is the ACP relay,
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
Make one small real edit in this CrabDB playground repo.

Edit exactly one file: docs/integrations/acp.md.
Add a short section titled "Playground smoke test" near the setup or
troubleshooting area.
Mention that a real Claude Code ACP run can be verified with
`crabdb agent view latest` and `crabdb agent ready latest`.
Keep it concise: 2-4 sentences.
Do not modify any other files.
After editing, reply with a brief summary and the file path changed.
```

Launch the high-level ACP entrypoint:

```sh
crabdb --workspace "$PLAYGROUND" agent acp --provider claude-code
```

Then drive the relay over JSON-RPC stdio from your editor or test client.

## Verify a Real Edit

After the prompt completes:

```sh
crabdb --workspace "$PLAYGROUND" agent ask what needs attention
crabdb --workspace "$PLAYGROUND" agent ask what should I do next
crabdb --workspace "$PLAYGROUND" agent summary latest
crabdb --workspace "$PLAYGROUND" agent ask what did the agent do
crabdb --workspace "$PLAYGROUND" agent ask where is the workdir
crabdb --workspace "$PLAYGROUND" agent ask where did the agent edit
crabdb --workspace "$PLAYGROUND" agent ask which prompt changed README.md
crabdb --workspace "$PLAYGROUND" agent ask last prompt
crabdb --workspace "$PLAYGROUND" agent ask what changed in the last prompt
crabdb --workspace "$PLAYGROUND" agent ask what changed in README.md in the last prompt
crabdb --workspace "$PLAYGROUND" agent ask show transcript
crabdb --workspace "$PLAYGROUND" agent ask what should I review
crabdb --workspace "$PLAYGROUND" agent ask what should I review first
crabdb --workspace "$PLAYGROUND" agent ask open review
crabdb --workspace "$PLAYGROUND" agent ask review this task
crabdb --workspace "$PLAYGROUND" agent ask what tools were used
crabdb --workspace "$PLAYGROUND" agent ask can I merge
crabdb --workspace "$PLAYGROUND" agent ask what changed since I looked
crabdb --workspace "$PLAYGROUND" agent ask explain README.md
crabdb --workspace "$PLAYGROUND" agent ask show patch for README.md
crabdb --workspace "$PLAYGROUND" agent ask show turn diff
crabdb --workspace "$PLAYGROUND" agent changes latest
crabdb --workspace "$PLAYGROUND" agent last latest
crabdb --workspace "$PLAYGROUND" agent what-changed latest
crabdb --workspace "$PLAYGROUND" agent can-land latest
crabdb --workspace "$PLAYGROUND" agent recover latest
crabdb --workspace "$PLAYGROUND" agent receipt latest
crabdb --workspace "$PLAYGROUND" agent pr latest
crabdb --workspace "$PLAYGROUND" agent turn-diff latest --file README.md --patch
crabdb --workspace "$PLAYGROUND" agent review-plan latest
crabdb --workspace "$PLAYGROUND" agent view latest
crabdb --workspace "$PLAYGROUND" agent land latest --dry-run
```

Expected signals:

- `agent` opens the inbox home view, groups all agent tasks by what needs
  attention, shows new files/lines since the last review, names the first file
  to inspect, and prints one primary next command.
- `agent inbox` is the explicit form of the same grouped home view.
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
  `what tools were used`, `changed files`,
  `can I merge`,
  `is it safe to land`, `recover`, or
  `explain README.md`. Patch and diff phrasing such as `show turn diff` and
  `show patch for README.md` routes to focused patch views.
- `agent todo` shows one primary command for the latest task. It is an alias for
  `agent next`.
- `agent summary latest` shows the one-page post-run cockpit with readiness,
  risk, validation, Git preflight, receipt Markdown, PR draft, and next command.
- `agent story latest` explains what happened in one plain-language task
  summary with turns, changed files, tools, notes, and next action.
- `agent risk latest` shows a deterministic apply risk level, reasons, and
  mitigation commands before you touch Git.
- `agent validate latest` shows latest test/eval gates and suggested validation
  commands without running anything. It is also available as `agent tests`.
- `agent test latest -- cargo test` records a durable test gate in the task
  workdir without requiring the lane name.
- `agent brief latest` shows one compact review packet with next action,
  changes, tools, and risks.
- `agent receipt latest` prints a copyable post-run receipt with validation,
  changed files, turns, tools, risk, checkpoint, and next command.
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
- `agent turn-diff latest --file README.md --patch` shows the focused file
  patch for the most recent prompt.
- `agent review-plan latest` shows readiness, risk, blockers, warnings, and the
  highest-priority files to inspect first.
- `agent focus latest` picks the next file to inspect and bundles why, review
  priority, and a focused diff summary.
- `agent view latest` shows the user prompt, assistant response, tool summaries, and
  checkpoint id.
- `agent can-land latest` shows the safe Git apply preflight.
- `agent land latest` applies using a generated Git commit message from the task
  title. It is an alias for `agent apply latest`; pass `-m` only when you want
  to override it.
- `Changed paths` contains only the file you intended the agent to edit.

When in doubt, start with:

```sh
crabdb --workspace "$PLAYGROUND" agent todo
```

It prints one primary command for the current task and a few alternatives.

Inside an ACP editor chat, ask the real agent to use CrabDB's injected MCP tools:

```text
Ask CrabDB what changed since I looked.
Ask CrabDB what the agent did.
Ask CrabDB what needs attention.
Ask CrabDB where the workdir is.
Ask CrabDB where the agent edited files.
Ask CrabDB which prompt changed README.md.
Ask CrabDB for the last prompt.
Ask CrabDB what changed in the last prompt.
Ask CrabDB what changed in README.md in the last prompt.
Ask CrabDB to show the transcript.
Ask CrabDB what I should review first.
Ask CrabDB to open review.
Ask CrabDB to review this task.
Ask CrabDB what tools were used.
Ask CrabDB if I can merge.
Ask CrabDB if the latest task is safe to land.
Ask CrabDB to explain README.md.
Ask CrabDB to show the patch for README.md.
What should I do next in CrabDB?
Show CrabDB agent status.
Show my CrabDB agent inbox.
Show a CrabDB brief for the latest agent task.
Create a CrabDB Markdown report for the latest agent task.
Show what changed in the latest CrabDB agent task.
Show files changed by the latest CrabDB agent task.
Show CrabDB rewind targets for the latest agent task.
Explain why README.md changed in the latest CrabDB agent task.
Compare two CrabDB agent tasks.
Ask CrabDB what tests I should run.
Show CrabDB validation guidance for the latest agent task.
Run a CrabDB test gate for the latest agent task with cargo test.
Show the latest CrabDB agent turn diff.
Ask CrabDB to show the latest turn diff.
Show the patch for the last CrabDB agent turn.
Show the CrabDB review plan for the latest agent task.
Review whether the latest CrabDB agent task is ready to apply.
```

ACP/MCP hosts can also expose CrabDB prompts directly:

```text
Use the crabdb.review_agent prompt for latest.
Use the crabdb.recover_agent prompt for latest.
Use the crabdb.apply_agent prompt for latest.
```

Plain-language "Ask CrabDB..." requests should use `crabdb.agent_ask`, which
routes to read-only task reports such as next actions, task story, review focus,
tools used, changes, readiness, recovery, file explanations, checkpoint targets,
and focused patch/diff views.
For validation, editors should ask `crabdb.agent_validate` first and present its
suggested command before calling `crabdb.agent_test` or `crabdb.agent_eval`.

Those map to `crabdb.agent_ask`, `crabdb.agent_next`, `crabdb.agent_status`,
`crabdb.agent_inbox`, `crabdb.agent_story`, `crabdb.agent_brief`, `crabdb.agent_report`, `crabdb.agent_workdir`,
`crabdb.agent_risk`, `crabdb.agent_ready`, `crabdb.agent_validate`, `crabdb.agent_test`, `crabdb.agent_changes`, `crabdb.agent_delta`, `crabdb.agent_new`, `crabdb.agent_mark_reviewed`, `crabdb.agent_files`, `crabdb.agent_checkpoints`, `crabdb.agent_why`,
`crabdb.agent_turn`, `crabdb.agent_compare`, `crabdb.agent_diff`, `crabdb.agent_review`,
`crabdb.agent_focus`, and
`crabdb.agent_apply`.

Editors can also render dashboards directly from MCP resources:

```text
crabdb://workspace/agent-tasks
crabdb://workspace/agent-tasks/latest/review
crabdb://workspace/agent-tasks/latest/changes
crabdb://workspace/agent-tasks/latest/files
crabdb://workspace/agent-tasks/latest/focus
crabdb://workspace/agent-tasks/<task-or-lane>/changes
crabdb://workspace/agent-tasks/<task-or-lane>/report
```

For a direct file diff against the playground baseline:

```sh
LANE="$(crabdb --workspace "$PLAYGROUND" --json agent view latest | jq -r .task.lane)"
LANE_WORKDIR="$PLAYGROUND/.crabdb/worktrees/$LANE"
git -C "$PLAYGROUND" show HEAD:docs/integrations/acp.md > /tmp/acp-baseline.md
diff -u /tmp/acp-baseline.md "$LANE_WORKDIR/docs/integrations/acp.md"
```

For structured verification:

```sh
crabdb --workspace "$PLAYGROUND" --json agent view latest | jq '{
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
crabdb --workspace "$PLAYGROUND" agent next
crabdb --workspace "$PLAYGROUND" agent changes latest
crabdb --workspace "$PLAYGROUND" agent turn
crabdb --workspace "$PLAYGROUND" agent turn-diff latest --file README.md --patch
```

If the agent went sideways, undo the task with a friendly target:

```sh
crabdb --workspace "$PLAYGROUND" agent undo-last latest
crabdb --workspace "$PLAYGROUND" agent undo-last latest --turn 2
crabdb --workspace "$PLAYGROUND" agent undo-last latest --prompt 'Add hook support'
```

Direct `agent rewind --to <CHECKPOINT_OR_LABEL>` still works, but
`agent undo-last` avoids copying `ch_...` ids from transcripts.

If the task is ready, check apply readiness:

```sh
crabdb --workspace "$PLAYGROUND" agent can-land latest
```

## Troubleshooting

### `agent doctor --provider claude-code` passes but no prompt runs

The doctor checks the provider profile and launch command. Run a real ACP
prompt through your editor or JSON-RPC driver, then inspect it with
`crabdb acp sessions`, `crabdb transcript <lane>`, and
`crabdb lane review <lane>`.

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
echo "$PLAYGROUND/.crabdb/worktrees/<lane>"
```

Use `crabdb lane review <lane>` or diff the materialized lane workdir against
the playground baseline.

### The transcript shows a cancelled turn

That means the relay flushed the open turn and checkpointed the lane when the
client disconnected or cancelled. Inspect it:

```sh
crabdb transcript <lane>
crabdb turn show <turn-id>
```

Then start a fresh lane or fresh turn.

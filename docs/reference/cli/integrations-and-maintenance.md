# CLI Reference: Integrations and Maintenance

This page covers commands that connect Trail to external tools and commands
that keep a workspace healthy.

Use it when you want to:

- Run Claude Code, Codex, Cursor, Gemini, Aider, OpenCode, or another CLI agent
  through Trail.
- Expose Trail as an MCP server.
- Capture ACP sessions from an editor.
- Export or import Git changes.
- Run diagnostics, backups, indexing, cleanup, or the daemon.

For day-to-day code-agent work, start with the `agent` commands. They create
fresh task lanes, keep agent work isolated, record checkpoints, and guide review
and apply.

## Quick Start

### Set up an agent provider

```sh
trail agent doctor --provider claude-code
trail agent setup
```

`agent setup` defaults to Claude Code plus VS Code. Use `--provider` for another
agent:

```sh
trail agent setup --provider codex
trail agent setup --provider cursor
trail agent setup --provider gemini --editor generic
```

### Run an agent task from the terminal

```sh
trail agent start --provider claude-code
trail agent start --provider codex
trail agent start --provider cursor
trail agent start --provider gemini
trail agent start --provider aider
trail agent start --provider opencode
```

Use `--workdir-mode overlay-cow` when a large repository should be exposed as a
mounted COW filesystem view instead of a full copied workdir:

```sh
trail agent start --provider codex --workdir-mode overlay-cow
```

For an unsupported terminal agent, pass the exact command after `--`:

```sh
trail agent start --provider custom -- my-agent --flag
```

### After the agent runs

```sh
trail agent
trail agent next
trail agent dashboard latest
trail agent review latest
trail agent validate latest
trail agent land latest --dry-run
```

These commands are intentionally state-aware. If no task exists, they show setup
guidance instead of failing. If a task exists, they point to the next review,
validation, apply, or recovery step.

## Command Families

| Family | Use it for |
| --- | --- |
| `agent` | Task-oriented coding-agent workflow |
| `acp` | Low-level ACP relay, install snippets, and captured sessions |
| `mcp` | MCP stdio server for agent context tools |
| `git` | Export, import, and inspect Git mappings |
| `api` | Generate OpenAPI output |
| `daemon` | Run Trail as a local HTTP service |
| `doctor` | Workspace and integration diagnostics |
| `backup` | Create, verify, and restore backups |
| `fsck` | Verify repository integrity |
| `index` | Rebuild or watch rich-text indexes |
| `gc` | Garbage-collect unused data |

## Agent Workflow

### Mental model

| Term | Meaning |
| --- | --- |
| Task | One unit of agent work tracked by Trail |
| Lane | Isolated Trail branch-like workspace for the task |
| Workdir | Filesystem directory where a terminal agent edits; usually full-cow, optionally overlay-cow |
| Turn | One prompt or response cycle captured from ACP |
| Checkpoint | Recorded code state that can be reviewed, applied, or rewound |
| `latest` | The most recent non-archived agent task |

The high-level workflow is:

1. Configure or start an agent.
2. Let the agent work in a fresh lane.
3. Review the task with `agent next`, `agent dashboard`, and focused file views.
4. Record validation with `agent test` or `agent eval`.
5. Apply with `agent land` or recover with `agent undo`.

### Provider support

| Provider | ACP | MCP | Terminal default | Notes |
| --- | --- | --- | --- | --- |
| `claude-code` | Yes | Yes | `claude` | Default setup provider |
| `codex` | Yes | Yes | `codex` | Uses the Codex ACP adapter for ACP mode |
| `cursor` | Yes | Yes | `agent` | Uses `agent acp` for ACP mode |
| `gemini` | No | Yes | `gemini` | Terminal-first provider |
| `aider` | No | No | `aider` | Terminal-first provider |
| `opencode` | No | No | `opencode` | Terminal-first provider |
| Custom | Command required | Depends on agent | Command required | Pass the command after `--` |

ACP mode gives Trail the richest live capture. Terminal mode works with any CLI
agent and records the final checkpoint after the process exits. MCP gives native
agents direct Trail context tools when they support MCP.

### Setup and diagnostics

| Command | Use it when |
| --- | --- |
| `trail agent setup` | Print the default Claude Code + VS Code setup |
| `trail agent setup --provider codex` | Print Codex setup |
| `trail agent setup --provider cursor` | Print Cursor setup |
| `trail agent setup --provider gemini` | Print Gemini setup notes |
| `trail agent doctor --provider <PROVIDER>` | Check workspace and provider readiness |
| `trail agent action` | Show runnable setup, review, validation, apply, and recovery actions |

`agent setup` output includes:

- The selected mode: `acp` or `terminal`.
- Provider capabilities: ACP, MCP, and terminal.
- A copyable command or editor snippet.
- Next-step commands for doctor, task inbox, and the action palette.

### Start or continue work

```text
trail agent acp --provider <claude-code|codex|cursor> \
  [--name <NAME>] [--from <REF>] [--no-mcp] [-- <COMMAND>...]

trail agent start --provider <claude-code|codex|cursor|gemini|aider|opencode> \
  [--name <NAME>] [--from <REF>] [--workdir-mode full-cow|overlay-cow] \
  [-- <COMMAND>...]

trail agent continue [latest|<TASK_OR_LANE_OR_SESSION>] \
  [--provider <PROVIDER>] [--name <NAME>] \
  [--workdir-mode full-cow|overlay-cow] [-- <COMMAND>...]
```

Use `agent acp` as the stable editor entrypoint. It creates a fresh task lane
for each ACP session.

Use `agent start` when launching an agent directly from the terminal. It creates
a task workdir, runs the agent there, and records a checkpoint when the command
exits. The default `full-cow` mode creates a full materialized workdir using
filesystem clone COW when possible. `overlay-cow` mounts a FUSE view for the
duration of the run so the agent sees normal files without the initial full
copy; it requires macFUSE on macOS or FUSE access on Linux.

Use `agent continue` after a task has landed or when you want another round of
edits from a known checkpoint. `agent follow-up` is an alias.

### Find the next step

| Command | Output |
| --- | --- |
| `trail agent` | Current task dashboard, grouped inbox, or setup guidance |
| `trail agent next` | One recommended next command |
| `trail agent status` | Latest task status and risk signal |
| `trail agent guide latest` | Short state-aware workflow |
| `trail agent dashboard latest` | Task board with next action, risk, and readiness |
| `trail agent action latest` | Runnable command palette for the task |

Readable aliases:

- `agent home` -> `agent inbox`
- `agent todo` -> `agent next`
- `agent help-me` -> `agent guide`
- `agent dash` -> `agent dashboard`
- `agent do` -> `agent action`

### Ask in plain language

Use `agent ask` when you remember the question but not the command.

```sh
trail agent ask what should I do next
trail agent ask what changed
trail agent ask what did the agent do
trail agent ask what should I review
trail agent ask what tests should I run
trail agent ask is it safe to land
trail agent ask why did it fail
trail agent ask explain README.md
```

Add `--selector <TASK>` to ask about a specific task:

```sh
trail agent ask --selector agent-claude-code-a1b2c3 what changed
```

### Review the work

| Need | Command |
| --- | --- |
| One compact review dashboard | `trail agent review latest` |
| File-by-file review checklist | `trail agent review-map latest` |
| First file to inspect | `trail agent focus latest` |
| Open the focus file | `trail agent open latest` |
| Changed files with provenance | `trail agent files latest` |
| Why one file changed | `trail agent why latest README.md` |
| Focused context for one file | `trail agent file latest README.md` |
| One ranked change card | `trail agent change latest 1` |
| Chronological timeline | `trail agent timeline latest` |
| One prompt-sized receipt | `trail agent turn latest 2` |
| Latest prompt-sized diff | `trail agent turn-diff latest --patch` |
| Whole task or checkpoint diff | `trail agent diff latest --patch` |

Readable aliases:

- `agent review-plan` -> `agent review`
- `agent review-files` and `agent file-checklist` -> `agent review-map`
- `agent changed-files` -> `agent files`
- `agent inspect` -> `agent file`
- `agent explain` -> `agent why`
- `agent last` -> `agent delta`

### Track reviewed changes

| Command | Use it when |
| --- | --- |
| `trail agent new latest` | Show changes since the task was last marked reviewed |
| `trail agent mark-reviewed latest` | Mark the current task checkpoint as reviewed |
| `trail agent mark-file-reviewed latest README.md` | Mark one file as reviewed |

Readable aliases:

- `agent what-changed` -> `agent new`
- `agent done` -> `agent mark-reviewed`
- `agent done-file` -> `agent mark-file-reviewed`

### Validate the task

| Command | Use it when |
| --- | --- |
| `trail agent validate latest` | Check latest gates and suggested validation |
| `trail agent test-plan latest` | Get a ranked validation checklist |
| `trail agent test latest -- cargo test` | Run and record a test gate |
| `trail agent eval latest -- <COMMAND>` | Run and record an evaluation gate |

Readable aliases:

- `agent tests` -> `agent validate`
- `agent validation-plan` and `agent test-checklist` -> `agent test-plan`

### Decide if it can land

| Command | Use it when |
| --- | --- |
| `trail agent risk latest` | Inspect deterministic risk reasons |
| `trail agent confidence latest` | Get the go/no-go verdict |
| `trail agent ready latest` | Check readiness and Git preflight |
| `trail agent land latest --dry-run` | Preview the safe apply path |
| `trail agent land latest` | Apply the task as a Git commit |
| `trail agent finish latest` | Apply and archive the task after success |

Readable aliases:

- `agent go` and `agent go-no-go` -> `agent confidence`
- `agent can-land` -> `agent ready`
- `agent apply` -> `agent land`
- `agent ship` -> `agent finish`

`agent land` records dirty task workdirs, creates a Git commit with a generated
message, and fast-forwards only when safe. Pass `-m <MESSAGE>` to override the
generated message.

### Recover from bad or stuck work

| Command | Use it when |
| --- | --- |
| `trail agent diagnose latest` | Explain likely failure modes and safe options |
| `trail agent checkpoints latest` | List rewind targets and checkpoint ids |
| `trail agent undo latest` | Undo the latest completed turn |
| `trail agent undo latest --turn 2` | Undo a specific turn |
| `trail agent rewind latest --to before-turn:2` | Rewind to a friendly target |
| `trail agent archive latest` | Hide a completed or irrelevant task |
| `trail agent unarchive <TASK>` | Restore an archived task |

Readable aliases:

- `agent recover` -> `agent diagnose`
- `agent rewind-points` -> `agent checkpoints`
- `agent undo-last` -> `agent undo`
- `agent close` -> `agent archive`

### Work with multiple tasks

| Command | Use it when |
| --- | --- |
| `trail agent inbox` | Group tasks by what needs attention |
| `trail agent board` | Show a low-noise multi-task board |
| `trail agent stack` | Find overlap and safe apply order |
| `trail agent compare <A> <B>` | Compare two tasks directly |
| `trail agent list --all` | List tasks, including archived tasks |

Readable aliases:

- `agent tasks` -> `agent board`
- `agent order` -> `agent stack`

### Share results

| Command | Output |
| --- | --- |
| `trail agent story latest` | Plain-language explanation of what happened |
| `trail agent summary latest` | Post-run cockpit with readiness, risk, and PR draft |
| `trail agent receipt latest` | Copyable Markdown receipt |
| `trail agent handoff latest` | Markdown handoff for another human or agent |
| `trail agent pr latest` | Pull request title and body draft |
| `trail agent report latest --markdown` | Deeper review source bundle |
| `trail agent brief latest` | Compact review packet |
| `trail agent tools latest` | Tool-call audit |
| `trail agent impact latest` | Blast-radius summary |

Readable aliases:

- `agent share` -> `agent handoff`

### Agent command map

This table lists the main `agent` commands without repeating every alias.

| Goal | Commands |
| --- | --- |
| Set up | `setup`, `doctor`, `action` |
| Start work | `acp`, `start`, `continue` |
| Navigate | `agent`, `inbox`, `board`, `next`, `status`, `guide`, `dashboard` |
| Ask | `ask` |
| Review dashboards | `review`, `review-map`, `focus`, `open`, `changes` |
| Review details | `delta`, `new`, `change`, `files`, `file`, `timeline`, `turn` |
| Diffs and provenance | `turn-diff`, `diff`, `why` |
| Mark reviewed | `mark-reviewed`, `mark-file-reviewed` |
| Validate | `validate`, `test-plan`, `test`, `eval` |
| Readiness | `risk`, `confidence`, `ready` |
| Apply | `land`, `finish` |
| Recover | `diagnose`, `checkpoints`, `undo`, `rewind` |
| Archive | `archive`, `unarchive` |
| Share | `brief`, `summary`, `receipt`, `handoff`, `pr`, `report`, `story`, `tools`, `impact` |
| Multi-task | `list`, `stack`, `compare` |

## ACP

Use `acp` when you need the low-level ACP relay rather than the higher-level
task workflow.

```text
trail acp relay [--lane <LANE>] [--from <REF>] \
  [--materialize[=true|false]] [--no-materialize] [--workdir <PATH>] \
  [--provider <NAME>] [--model <NAME>] [--no-mcp] -- <COMMAND>...

trail acp install --agent <claude-code|codex|cursor> \
  [--editor generic|zed] [--dry-run] [--print]

trail acp doctor --agent <claude-code|codex|cursor> \
  [--relay-command <COMMAND>...]

trail acp list
trail acp sessions [--lane <LANE>]
```

Built-in ACP upstream commands:

```sh
trail acp relay --provider claude-code --materialize -- \
  npx -y @agentclientprotocol/claude-agent-acp@latest

trail acp relay --provider codex --materialize -- npx -y @agentclientprotocol/codex-acp@latest
trail acp relay --provider cursor --materialize -- agent acp
```

Use `acp install` to print setup snippets. It does not mutate editor
configuration. Use `acp sessions` to inspect captured ACP sessions.

## MCP

```text
trail mcp
```

Starts the MCP stdio server.

Register this command in agents that support MCP when you want the agent to ask
Trail for workspace context. `agent setup --provider gemini --editor generic`
prints the same command as part of the Gemini setup notes.

## Git Integration

```text
trail git export <RANGE> [-m <MESSAGE>] [--output <PATH>]
trail git import-update [-m <MESSAGE>]
trail git mappings [--limit <N>]
```

| Command | Use it when |
| --- | --- |
| `git export` | Convert a Trail range to a Git patch or commit-like export |
| `git import-update` | Pull the current Git working tree back into Trail |
| `git mappings` | Inspect recorded Git-to-Trail mappings |

`git mappings` defaults to 30 rows.

## API

```text
trail api openapi [--output <PATH>]
```

Writes the OpenAPI description to stdout or to `--output`.

## Daemon

```text
trail daemon [--host <HOST>] [--port <PORT>] [--once] \
  [--max-requests <N>] [--rate-limit-requests <N>] \
  [--rate-limit-window-secs <SECONDS>] \
  [--connection-timeout-secs <SECONDS>] \
  [--auth-token <TOKEN>] [--auth-token-file <PATH>] [--no-auth]
```

Defaults:

| Setting | Default |
| --- | --- |
| Host | `127.0.0.1` |
| Port | `8765` |
| Auth | Enabled |
| Rate limit | 600 accepted requests per peer per 60 seconds |
| Socket read/write timeout | 30 seconds |

`--no-auth` is allowed only for loopback listeners. It prints a stderr
`WARNING` even with `--quiet`, and should only be used for trusted local
automation.

Rate-limit and timeout values must be greater than zero.

## Doctor

```text
trail doctor
```

Runs workspace and integration diagnostics.

Use provider-specific diagnostics when the question is about agent setup:

```sh
trail agent doctor --provider claude-code
trail acp doctor --agent claude-code
```

## Backup

```text
trail backup create <OUTPUT> [--overwrite]
trail backup verify <PATH>
trail backup restore <PATH> [--force]
```

| Command | Use it when |
| --- | --- |
| `backup create` | Write a workspace backup |
| `backup verify` | Check that a backup is readable |
| `backup restore` | Restore from a backup |

## Fsck

```text
trail fsck
```

Verifies repository integrity.

## Index

```text
trail index rebuild [--rich-text]
trail index watch [--once] [--iterations <N>] [--interval-ms <MS>]
```

| Command | Use it when |
| --- | --- |
| `index rebuild` | Rebuild indexes for the workspace |
| `index watch` | Rebuild repeatedly while files change |

`index watch` defaults to a 1000 ms interval.

## Garbage Collection

```text
trail gc [--dry-run]
```

Use `--dry-run` to preview cleanup before deleting unused data.

## Implementation References

Use these files when you need to verify the CLI surface from code:

- `crates/trail/src/cli/command/maintenance_args.rs`
- `crates/trail/src/cli/command/handler/maintenance.rs`
- `crates/trail/src/model/reports/maintenance.rs`

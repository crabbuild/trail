# CLI Reference: Integrations and Maintenance

## `git`

```text
crabdb git export <RANGE> [-m <MESSAGE>] [--output <PATH>]
crabdb git import-update [-m <MESSAGE>]
crabdb git mappings [--limit <N>]
```

Mapping limit defaults to 30.

## `api`

```text
crabdb api openapi [--output <PATH>]
```

## `daemon`

```text
crabdb daemon [--host <HOST>] [--port <PORT>] [--once] [--max-requests <N>] [--rate-limit-requests <N>] [--rate-limit-window-secs <SECONDS>] [--connection-timeout-secs <SECONDS>] [--auth-token <TOKEN>] [--auth-token-file <PATH>] [--no-auth]
```

Defaults: host `127.0.0.1`, port `8765`, auth enabled.
Rate limiting defaults to 600 accepted requests per peer per 60 seconds, and
the socket read/write timeout defaults to 30 seconds. The rate-limit and
timeout values must be greater than zero.
`--no-auth` is allowed only with a loopback listener, prints a stderr `WARNING`
even with `--quiet`, and should only be used for trusted local automation.

## `mcp`

```text
crabdb mcp
```

Starts the MCP stdio server.

## `agent`

```text
crabdb agent
crabdb agent setup --provider claude-code [--editor generic|vscode|zed]
crabdb agent acp --provider claude-code [--name <NAME>] [--from <REF>] [--no-mcp] [-- <COMMAND>...]
crabdb agent start --provider claude-code [--name <NAME>] [-- <COMMAND>...]
crabdb agent ask [--selector latest|<TASK_OR_LANE_OR_SESSION>] <QUESTION>...
crabdb agent inbox
crabdb agent home
crabdb agent next [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent todo [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent status
crabdb agent brief [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent summary [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent validate [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent tests [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent receipt [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent pr [latest|<TASK_OR_LANE_OR_SESSION>] [--title-only|--body-only]
crabdb agent report [latest|<TASK_OR_LANE_OR_SESSION>] [--markdown]
crabdb agent story [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent risk [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent ready [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent can-land [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent diagnose [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent recover [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent compare <TASK_OR_LANE_OR_SESSION_A> <TASK_OR_LANE_OR_SESSION_B>
crabdb agent test [latest|<TASK_OR_LANE_OR_SESSION>] [--turn <TURN>] [--timeout-secs <SECONDS>] [--suite <SUITE>] [--score <SCORE>] [--threshold <THRESHOLD>] -- <COMMAND>...
crabdb agent eval [latest|<TASK_OR_LANE_OR_SESSION>] [--turn <TURN>] [--timeout-secs <SECONDS>] [--suite <SUITE>] [--score <SCORE>] [--threshold <THRESHOLD>] -- <COMMAND>...
crabdb agent workdir [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent list
crabdb agent view [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent changes [latest|<TASK_OR_LANE_OR_SESSION>] [--by-turn|--by-operation]
crabdb agent delta [latest|<TASK_OR_LANE_OR_SESSION>] [--by-turn|--by-operation] [--file <PATH>] [--patch]
crabdb agent last [latest|<TASK_OR_LANE_OR_SESSION>] [--by-turn|--by-operation] [--file <PATH>] [--patch]
crabdb agent new [latest|<TASK_OR_LANE_OR_SESSION>] [--file <PATH>] [--patch]
crabdb agent what-changed [latest|<TASK_OR_LANE_OR_SESSION>] [--file <PATH>] [--patch]
crabdb agent mark-reviewed [latest|<TASK_OR_LANE_OR_SESSION>] [--note <TEXT>]
crabdb agent done [latest|<TASK_OR_LANE_OR_SESSION>] [--note <TEXT>]
crabdb agent change [<TASK_OR_LANE_OR_SESSION>] [<RANK_OR_KEY>] [--patch]
crabdb agent files [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent changed-files [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent file [<TASK_OR_LANE_OR_SESSION>] <PATH> [--patch]
crabdb agent inspect [<TASK_OR_LANE_OR_SESSION>] <PATH> [--patch]
crabdb agent checkpoints [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent rewind-points [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent timeline [latest|<TASK_OR_LANE_OR_SESSION>] [--by-turn|--by-operation]
crabdb agent turn [latest|<TASK_OR_LANE_OR_SESSION>] [<TURN|last>] [--file <PATH>] [--patch]
crabdb agent turn-diff [latest|<TASK_OR_LANE_OR_SESSION>] [--turn <N_OR_TURN_ID>] [--file <PATH>] [--stat] [--patch]
crabdb agent why <PATH>
crabdb agent why <TASK_OR_LANE_OR_SESSION> <PATH>
crabdb agent explain <PATH>
crabdb agent explain <TASK_OR_LANE_OR_SESSION> <PATH>
crabdb agent diff [latest|<TASK_OR_LANE_OR_SESSION>] [--last-turn|--turn <N_OR_TURN_ID>|--operation <CHANGE>|--checkpoint <CHANGE>] [--file <PATH>] [--stat] [--patch]
crabdb agent review [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent review-plan [latest|<TASK_OR_LANE_OR_SESSION>]
crabdb agent focus [latest|<TASK_OR_LANE_OR_SESSION>] [--file <PATH>] [--patch]
crabdb agent apply [latest|<TASK_OR_LANE_OR_SESSION>] [--dry-run] [-m <MESSAGE>] [--into-current-git-branch]
crabdb agent land [latest|<TASK_OR_LANE_OR_SESSION>] [--dry-run] [-m <MESSAGE>] [--into-current-git-branch]
crabdb agent undo [latest|<TASK_OR_LANE_OR_SESSION>] [--last-turn|--turn <N_OR_TURN_ID>|--prompt <TEXT>|--last-operation]
crabdb agent undo-last [latest|<TASK_OR_LANE_OR_SESSION>] [--last-turn|--turn <N_OR_TURN_ID>|--prompt <TEXT>|--last-operation]
crabdb agent rewind [latest|<TASK_OR_LANE_OR_SESSION>] --to <CHECKPOINT|before-last-turn|turn:N|before-turn:N|prompt:TEXT|before-prompt:TEXT|before-last-operation>
crabdb agent doctor --provider claude-code
```

`agent` is the task-oriented workflow for coding agents. It creates fresh lanes
by default, resolves `latest`, records dirty lane workdirs before apply, checks
that the current Git tree matches CrabDB's internal apply base, creates a Git
commit, and fast-forwards only when safe.

Run bare `crabdb agent` when you are not sure what to do next. It opens the
agent inbox home view, groups tasks by what needs attention, shows new
files/lines since the last review, names the first file to inspect, and prints
one primary next command. It is equivalent to `crabdb agent inbox`.

Use `agent ask` when you do not remember a command name. It deterministically
routes common plain-language questions to existing views, so JSON and human
output stay the same as the underlying command. Examples: `crabdb agent ask what
needs attention`, `crabdb agent ask what should I do next`,
`crabdb agent ask what did the agent do`,
`crabdb agent ask where is the workdir`,
`crabdb agent ask where did the agent edit`,
`crabdb agent ask which prompt changed README.md`,
`crabdb agent ask last prompt`,
`crabdb agent ask what changed in the last prompt`,
`crabdb agent ask what changed in README.md in the last prompt`,
`crabdb agent ask show transcript`,
`crabdb agent ask what should I review`,
`crabdb agent ask what should I review first`,
`crabdb agent ask open review`, `crabdb agent ask review this task`,
`crabdb agent ask what tools were used`, `crabdb agent ask what just changed`,
`crabdb agent ask what changed since I looked`, `crabdb agent ask changed files`,
`crabdb agent ask what tests should I run`, `crabdb agent ask can I merge`,
`crabdb agent ask is it safe to land`,
`crabdb agent ask recover`, and `crabdb agent ask explain README.md`.
Patch and diff wording routes to focused patch views, for example
`crabdb agent ask show last patch`, `crabdb agent ask show turn diff`, and
`crabdb agent ask show patch for README.md`. Add `--selector <TASK>` before the
question to ask about a specific task.

Use `agent status` when you only want the latest task status and embedded risk
signal.

Agent task output shows a human `title` first, derived from the prompt or from
`--name`; the stable task id/lane name remains in JSON and human output when it
differs from the title. Materialized tasks also include `workdir`, the exact
filesystem directory where the agent edited files.

Use `agent workdir latest` when you want to jump into the files the editor or
terminal agent touched. It prints the resolved task workdir plus a shell-safe
`cd` command, and its JSON output is suitable for editor panels.

Use `agent todo` first after an editor or terminal agent run. It is an alias for
`agent next` and returns one primary command based on current state, for example
review a ready task, preview apply for a dirty task workdir, inspect blockers,
or configure an editor when no tasks exist. `agent status` also embeds the
latest task risk level so the first status check carries a safety signal.

Use `agent story` when you want the fastest human explanation of what happened.
It returns one plain-language summary, prompt/turn summaries, changed files,
tools, readiness notes, and the next action.

Use `agent risk` before applying when you want a compact go/no-go signal. It
returns a deterministic low/medium/high/blocking risk level, reasons, and
concrete mitigation commands such as checking validation guidance or inspecting
the last turn diff.

Use `agent can-land` when you want the safest pre-apply answer without invoking
the destructive-looking apply command. It is an alias for `agent ready`: it
combines task readiness, risk, Git preflight, blockers, warnings, and the one
next command. `ready = true` means the dry-run apply path is clean.

Use `agent recover latest` when a task looks stuck, blocked, risky, or sideways.
It is an alias for `agent diagnose latest`: it explains the likely issue, shows
evidence, lists recent friendly recovery targets, and prints safe
inspection/recovery commands before you run destructive undo or rewind actions.

Use `agent compare <A> <B>` when two agents or follow-up tasks may overlap. It
shows shared changed files, files only one task changed, both risk scores, and
one recommended next command before you decide what to review or apply first.

Use `agent test latest -- cargo test` or `agent eval latest -- <command>` to
record durable gates in the task workdir without dropping to `crabdb lane`.
These commands default to `latest`, so `crabdb agent test -- cargo test` is the
short form for the current task.

Use `agent validate latest` before running gates when you are not sure what is
missing. It is read-only: it reports the latest test/eval gates, whether more
validation is needed, and suggested `agent test`/`agent eval` commands. Higher
level review, risk, readiness, and summary views route here first so users can
inspect the validation plan before running open-world commands. Use `agent
tests latest` or `agent ask what tests should I run` for the same view.

Use `agent brief` when you want one compact review packet instead of separately
running `next`, `changes`, `diff`, `review`, and `view`. It includes the next
action, risk level, readiness, blockers/warnings, changed files, turn or
operation groups, latest diff stats, and tool summaries.

Use `agent summary latest` when you want the easiest post-run cockpit for one
task. It combines readiness, risk, validation, Git preflight, receipt Markdown,
PR draft text, and the one next command. JSON output is suitable for an editor
panel.

Use `agent receipt latest` when you need the fastest copyable post-run artifact.
It prints Markdown by default with the task summary, validation gates, changed
files, turns, tools, risk, checkpoint, and next command. JSON output keeps the
same receipt data structured for editor panels.

Use `agent pr latest` when you need a pull request draft. It prints a title and
Markdown body generated from the recorded task state, but does not create a
remote PR or mutate Git. Use `--title-only` or `--body-only` when scripting.

Use `agent report latest --markdown` when you need the deeper review or handoff
bundle. It includes the same task context plus the full story, changes, review
packet, transcript, and Markdown.

Use `agent changes` when reviewing agent work at the intent level. It starts
with deterministic change cards, then includes the prompt/turn or operation
checkpoint groups behind those cards. It also returns one primary `next` command
and each card carries a ready `review_command`, plus focus/why/diff commands, so
editors and terminals can guide users to the highest-priority card without
exposing turn ids or checkpoint ids first. Use `agent turn-diff` for
prompt-sized patch detail, and use `agent diff` for operation, checkpoint, or
whole-task patch detail. Add `--file <PATH>` to keep either view focused on one
changed file.

Use `agent last` when you only need what is newest. It is an alias for
`agent delta`: it picks the latest completed turn by default, falls back to the
latest operation when no turn transcript exists, and can include a focused patch
with `--patch` or `--file <PATH> --patch`.

Use `agent what-changed` when you need what changed since the last reviewed
checkpoint. It is an alias for `agent new`. `agent done` stores the current task
head as that checkpoint; `agent mark-reviewed` is the explicit spelling for the
same action. Until a task has a reviewed marker, `agent what-changed` shows the
whole task as unreviewed.

Use `agent change latest 1` when one change card needs deeper review. It expands
the selected rank/key/title into files, prompts or operations that touched the
files, tool summaries, next commands, and optional focused patches with
`--patch`.

Use `agent timeline latest` when reviewing agent work chronologically. The
default view groups by turn and connects prompt previews, assistant previews,
tool summaries, checkpoints, changed files, and per-item view/diff/rewind
commands. Add `--by-operation` to zoom into lower-level CrabDB operations.

Use `agent turn` when you want one prompt-sized receipt. With no arguments it
shows the latest completed turn for the latest task. `agent turn 2` shows turn
2 for the latest task, and `agent turn <TASK> 2 --file README.md --patch`
includes a focused patch for that file.

Use `agent turn-diff` when you want the prompt-sized code diff without spelling
out `agent diff --last-turn`. With no flags it shows the latest completed turn.
Pass `--turn <N_OR_TURN_ID>` for a specific prompt, `--file <PATH>` for the file
open in your editor, and `--patch` when exact hunks matter.

Use `agent review-plan latest` when you want the shortest review dashboard. It
shows readiness, risk, blockers and warnings, then ranks the files to inspect
first with ready `agent why` and `agent turn-diff` commands. `agent review
latest` is the shorter alias for the same review-priority dashboard.

Use `agent focus latest` when you want CrabDB to pick the next file to inspect
and combine its review priority, prompt/tool explanation, and focused diff. Add
`--patch` when you want the unified patch inline.

Use `agent changed-files` when you want a code-review-shaped view. It is an
alias for `agent files`: it lists every file the agent changed, which turn or
operation touched it, and the ready commands to explain why the file changed or
inspect the related patch.

Use `agent inspect README.md` when you are looking at one file and want the
agent-specific context for that path. It is an alias for `agent file README.md`:
it reports whether the task changed the file, which change set contains it,
which prompt/operation touched it, and the next command. Add `--patch` for the
focused diff.

Use `agent rewind-points` before rewind or undo when you want to see friendly
recovery targets. It is an alias for `agent checkpoints`: it lists each captured
turn or operation, the exact checkpoint ids, labels such as `before-turn:2`, and
ready-to-run rewind commands.

Use `agent explain README.md` when you are looking at a changed file and want
the captured prompt, turn, tools, checkpoint, and focused diff command that
explain where it came from. It is an alias for `agent why README.md`. Pass an
explicit task first, as in `crabdb agent explain latest README.md`, when you do
not want `latest`.

Use `agent land latest` after a clean dry run. It is an alias for `agent apply`:
both record dirty task workdirs, create a Git commit with a generated message
from the task title, and fast-forward only when safe. Pass `-m <MESSAGE>` only
to override the generated commit message.

Use `agent undo-last` for everyday recovery from a bad prompt. It is an alias
for `agent undo`. For example, `crabdb agent undo-last latest` preserves the
current head, moves the task back to the state before the latest completed turn,
and syncs the materialized task workdir when one exists. Use
`agent rewind --to <CHECKPOINT_OR_LABEL>` only when you need an exact checkpoint
or advanced friendly target.

## `acp`

```text
crabdb acp relay [--lane <LANE>] [--from <REF>] [--materialize[=true|false]] [--no-materialize] [--workdir <PATH>] [--provider <NAME>] [--model <NAME>] [--no-mcp] -- <COMMAND>...
crabdb acp install --agent claude-code [--editor generic|zed] [--dry-run] [--print]
crabdb acp doctor --agent claude-code [--relay-command <COMMAND>...]
crabdb acp list
crabdb acp sessions [--lane <LANE>]
```

`acp install` prints setup snippets and does not mutate editor config. `acp
relay` remains the low-level ACP stdio relay in front of the real coding agent.

## `demo`

```text
crabdb demo acp [--agent claude-code]
```

`demo acp` prints a guided workflow for configuring an ACP editor and reviewing
captured agent work.

## `doctor`

```text
crabdb doctor
```

Runs workspace and integration diagnostics.

## `backup`

```text
crabdb backup create <OUTPUT> [--overwrite]
crabdb backup verify <PATH>
crabdb backup restore <PATH> [--force]
```

## `fsck`

```text
crabdb fsck
```

Verifies repository integrity.

## `index`

```text
crabdb index rebuild [--rich-text]
crabdb index watch [--once] [--iterations <N>] [--interval-ms <MS>]
```

`index watch` default interval is 1000 ms.

## `gc`

```text
crabdb gc [--dry-run]
```

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/maintenance_args.rs`
- Handlers: `crates/crabdb/src/cli/command/handler/maintenance.rs`
- Reports: `crates/crabdb/src/model/reports/maintenance.rs`

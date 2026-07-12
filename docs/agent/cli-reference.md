# Trail Agent CLI Reference

This reference covers the high-level `trail agent` command family. Run
`trail agent --help` for the short daily-use list and
`trail agent <COMMAND> --help` for the options accepted by one command.

## General Syntax

```text
trail [GLOBAL OPTIONS] agent [COMMAND] [TASK] [OPTIONS]
```

Running `trail agent` without a subcommand opens the current task dashboard or,
when several tasks exist, the grouped inbox.

Most single-task commands use this selector syntax:

```text
trail agent <COMMAND> [TASK]
```

`TASK` can be a task ID, lane, session, ACP session, or `latest`. When omitted,
it normally defaults to `latest`. Archived tasks are not considered by
`latest`.

Use Trail's global JSON option for structured output:

```sh
trail --json agent dashboard latest
```

## Setup and Task Creation

| Command | Purpose and important options |
| --- | --- |
| `setup` | Print editor setup. `--provider <NAME>` defaults to `claude-code`; `--editor <NAME>` defaults to `vscode`. |
| `doctor` | Check provider and workspace readiness. `--provider <NAME>` defaults to `claude-code`. |
| `start` | Create a fresh task lane and launch a terminal agent. Supports `--provider`, `--name`, `--from`, `--workdir-mode`, and a command after `--`. |
| `continue [TASK]` | Create a follow-up task from an existing task checkpoint. Supports `--provider`, `--name`, `--workdir-mode`, and a command after `--`. Alias: `follow-up`. |
| `acp` | Hidden stable ACP entry point used by editor integrations. Supports `--provider`, `--name`, `--from`, `--no-mcp`, and a command after `--`. |

Built-in terminal profiles are `claude-code`, `codex`, `cursor`, `gemini`,
`aider`, and `opencode`.

Valid terminal workdir modes are `native-cow`, `fuse-cow`, `nfs-cow`, and `dokan-cow`;
the default is `native-cow`.

```sh
trail agent setup --provider codex --editor vscode
trail agent doctor --provider codex
trail agent start --provider codex --name docs-task
trail agent start --provider codex --from main --workdir-mode native-cow
trail agent continue latest --provider claude-code
```

## Navigation and Daily Workflow

| Command | Purpose |
| --- | --- |
| `guide [TASK]` | Show the shortest state-aware workflow. Alias: `help-me`. |
| `ask [--selector TASK] <QUESTION...>` | Route a plain-language phrase to a Trail Agent view. |
| `next [TASK]` | Show one primary next action and alternatives. Alias: `todo`. |
| `status` | Show the latest task and its next useful action. |
| `dashboard [TASK]` | Show next action, review focus, validation, risk, and apply readiness. Alias: `dash`. |
| `inbox [--all]` | Group tasks by attention state. Alias: `home`. |
| `board [--all]` | Show a low-noise multi-task board. Alias: `tasks`. |
| `stack [--all]` | Show file overlap and a safe apply order. Alias: `order`. |
| `list [--all]` | List recorded tasks and stable selectors. |

`ask` uses deterministic phrase routing. Its selector is an option, not a
positional argument:

```sh
trail agent ask what changed
trail agent ask --selector <TASK> what should I review first
trail agent ask explain README.md
```

## Actions and Guided Review

| Command | Purpose and important options |
| --- | --- |
| `action [SELECTOR_OR_ACTION] [ACTION]` | List or run a published action. `--print` shows its command; `--confirm` permits confirmation-required actions; `-m/--message` and `--note` supply action inputs. Alias: `do`. |
| `review-flow [TASK]` | Walk through review, validation, and finish. Aliases: `walkthrough`, `review-loop`. |
| `review-data [TASK]` | Return the structured editor-panel packet. Hidden from short help. Aliases: `cockpit`, `side-panel`. |

With one positional argument, `action` treats it as an action ID for `latest`.
With two, the first is the task selector:

```sh
trail agent action
trail agent action inspect_focus_file --print
trail agent action <TASK> inspect_focus_file
trail agent action <TASK> run_validation --confirm
```

## Task and Change Inspection

| Command | Purpose and important options |
| --- | --- |
| `view [TASK]` | Show transcript, tools, changed paths, and checkpoint. |
| `changes [TASK]` | Show review-oriented change cards. Group with `--by-turn`, `--by-operation`, or `--by-file`. |
| `change [SELECTOR_OR_CHANGE] [CHANGE]` | Expand one change card by rank, key, or title. Add `--patch`. |
| `delta [TASK]` | Show the newest turn or operation delta. Supports `--by-turn`, `--by-operation`, `--file`, and `--patch`. Alias: `last`. |
| `new [TASK]` | Show changes since the last reviewed checkpoint. Supports `--file` and `--patch`. Alias: `what-changed`. |
| `timeline [TASK]` | Connect prompts, tools, checkpoints, and files. Supports `--by-turn` or `--by-operation`. |
| `files [TASK]` | List changed files and their prompts, turns, and operations. Alias: `changed-files`. |
| `file <PATH>` or `file <TASK> <PATH>` | Inspect one changed file. Add `--patch`. Alias: `inspect`. |
| `why <PATH>` or `why <TASK> <PATH>` | Explain which prompt, turn, tools, and checkpoint changed a file. Alias: `explain`. |
| `turn [SELECTOR_OR_TURN] [TURN]` | Inspect a turn by 1-based index, ID, `latest`, or `last`. Supports `--file` and `--patch`. |
| `turn-diff [TASK]` | Diff the newest or `--turn` selected turn. Supports `--file`, `--stat`, and `--patch`. |
| `diff [TASK]` | Diff the task or one `--turn`, `--operation`, `--checkpoint`, or `--last-turn`; supports `--file`, `--stat`, and `--patch`. |
| `checkpoints [TASK]` | List checkpoint IDs and friendly rewind targets. Alias: `rewind-points`. |
| `workdir [TASK]` | Print the task's filesystem workdir and a copyable `cd` command. |
| `tools [TASK]` | Show tool calls, commands, turns, and checkpoints. Hidden from short help. |
| `story [TASK]` | Explain the task in plain language. Hidden from short help. |
| `impact [TASK]` | Group changed areas and report blast radius. Hidden from short help. |

Commands with `SELECTOR_OR_*` allow a short latest-task form and an explicit
task form:

```sh
trail agent change 1
trail agent change <TASK> 1 --patch
trail agent file README.md --patch
trail agent file <TASK> README.md --patch
trail agent turn 2 --patch
trail agent turn <TASK> 2 --patch
```

## Review, Risk, and Readiness

| Command | Purpose |
| --- | --- |
| `review [TASK]` | Show review readiness, transcript, changes, and next steps. Alias: `review-plan`. |
| `review-map [TASK]` | Show a file-by-file checklist grouped by changed area. Aliases: `review-files`, `file-checklist`. |
| `focus [TASK]` | Select the highest-priority changed file, or use `--file <PATH>`; add `--patch`. |
| `open [TASK]` | Open the focus file in `$EDITOR`, or choose `--file`; use `--print` to avoid launching. Alias: `edit`. |
| `brief [TASK]` | Show a compact review brief. Hidden from short help. |
| `summary [TASK]` | Show the post-run cockpit. Hidden from short help. |
| `risk [TASK]` | Explain apply risk and mitigations. Hidden from short help. |
| `confidence [TASK]` | Give a go/no-go verdict across review, gates, risk, and preflight. Aliases: `go`, `go-no-go`. |
| `ready [TASK]` | Check safe apply readiness without changing Git. Alias: `can-land`. |
| `diagnose [TASK]` | Explain stuck tasks and safe recovery options. Alias: `recover`. |
| `compare <LEFT> <RIGHT>` | Compare task overlap, risk, and apply order. Hidden from short help. |

## Validation Commands

| Command | Purpose and important options |
| --- | --- |
| `validate [TASK]` | Read existing gates and suggest validation commands without running them. Alias: `tests`. |
| `test-plan [TASK]` | Produce a prioritized test/eval checklist. Aliases: `validation-plan`, `test-checklist`. |
| `test [TASK] -- <COMMAND...>` | Run a command in the task workdir and record a test gate. |
| `eval [TASK] -- <COMMAND...>` | Run a command and record an evaluation gate. |

`test` and `eval` support:

| Option | Meaning |
| --- | --- |
| `--turn <TURN>` | Associate the gate with a selected turn. |
| `--timeout-secs <SECONDS>` | Set the command timeout; default: `600`. |
| `--suite <NAME>` | Record a suite name. |
| `--score <NUMBER>` | Record a numeric evaluation score. |
| `--threshold <NUMBER>` | Record the passing threshold. |

The command after `--` is required:

```sh
trail agent test latest -- cargo test --workspace
trail agent eval latest --suite quality --score 0.95 --threshold 0.9 -- ./eval.sh
```

## Review Markers and Task Visibility

| Command | Purpose and important options |
| --- | --- |
| `mark-file-reviewed <PATH>` or `mark-file-reviewed <TASK> <PATH>` | Mark one file reviewed at the current checkpoint. Supports `--note`. Aliases: `done-file`, `reviewed-file`. |
| `mark-reviewed [TASK]` | Mark the entire current checkpoint reviewed. Supports `--note`. Alias: `done`. |
| `archive [TASK]` | Hide a task from default views while preserving history. Supports `--note`. Alias: `close`. |
| `unarchive [TASK]` | Restore an archived task to normal views. |

## Reports and Sharing

| Command | Purpose and important options |
| --- | --- |
| `receipt [TASK]` | Print a concise, copyable post-run receipt. |
| `handoff [TASK]` | Print a handoff packet for another human or agent. Alias: `share`. |
| `report [TASK]` | Create the deeper review report. Use `--markdown` for copyable Markdown. |
| `pr [TASK]` | Print a local PR draft. Use `--title-only` or `--body-only`. Does not create a remote PR. |

## Apply and Finish

| Command | Purpose and important options |
| --- | --- |
| `apply [TASK]` | Safely apply the task to the current Git branch. Supports `--dry-run`, `-m/--message`, and `--into-current-git-branch`. Alias: `land`. |
| `finish [TASK]` | Apply and archive after success. Supports the apply options plus `--note`. Alias: `ship`. |

Use this sequence:

```sh
trail agent ready latest
trail agent apply latest --dry-run
trail agent apply latest
```

The explicit `--into-current-git-branch` option documents the apply target for
scripts and integrations; the high-level apply workflow targets the current
Git branch.

## Recovery

### Native hooks, evidence, and managed capture

| Command | Purpose and important options |
| --- | --- |
| `hooks add PROVIDER` | Safely install Trail-owned project or user hooks. Use `--scope project|user`, `--lane`, `--dry-run`, and `--force` only after reviewing foreign config. |
| `hooks remove PROVIDER` | Remove only exact Trail-owned entries or files. Supports scope, dry-run, and force. |
| `hooks list` | List all eight adapters and recorded installations; `--installed` filters the catalog. |
| `hooks status PROVIDER` | Compare persisted ownership digests with current provider configuration. |
| `hooks doctor [PROVIDER]` | Report compatibility, drift, receipt failures, spool pressure, capture ownership, transcript fidelity, and stale finalizers. Use `--all` or `--probe`. |
| `hooks events PROVIDER` | List durable redacted receipt rows; `--failed` selects retry, quarantine, and discarded states. |
| `hooks replay --pending` | Drain the secure fallback spool, recover stale processing leases, and replay due receipts with bounded exponential backoff. |
| `hooks retry RECEIPT` | Move a retrying or quarantined receipt back to the replay queue. |
| `hooks discard RECEIPT` | Retain the receipt audit row while explicitly preventing later replay. |
| `capture begin` | Declare a leased managed run with `--owner`, `--session`, optional `--executor`, `--lane`, `--workdir`, `--work-item`, and `--ttl-ms`. |
| `capture renew RUN` | Renew the exact owner/session lease. |
| `capture end RUN` | Idempotently end the exact owner/session run. |
| `capture status` | List active runs; `--all` includes ended and expired runs. |
| `capture reconcile` | Expire abandoned runs and close their open mappings, turns, and sessions as interrupted. |
| `artifacts SESSION` | List immutable native transcript, canonical export, or reconstructed evidence artifacts. |
| `provenance SESSION` | Read factual and explicitly derived causal nodes and edges. |
| `attest create|list|show|verify` | Create and verify content-addressed, chained session attestations. |
| `learnings list|accept|reject` | Review reusable findings; Trail never edits provider context files automatically. |
| `export SESSION` | Write or print the verified portable agent-trace representation; `--attachments` includes bounded attachment bytes. |
| `git-link link|list` | Record or query exact Git commit, Trail change, turn, and session associations. |

The singular `trail agent hook receive PROVIDER EVENT` command is an internal,
fail-open provider callback. Integration authors may call it, but users should
manage installations through `trail agent hooks`.

| Command | Purpose and important options |
| --- | --- |
| `undo [TASK]` | Undo the latest completed turn by default. Select with `--last-turn`, `--turn <N_OR_ID>`, `--prompt <TEXT>`, or `--last-operation`. Alias: `undo-last`. |
| `rewind [TASK] --to <TARGET>` | Rewind to a checkpoint ID or friendly target. |

Friendly rewind targets include:

- `before-last-turn`
- `turn:2`
- `before-turn:2`
- `before-prompt:<text>`
- `before-last-operation`

The undo target flags are mutually exclusive.

## Alias Index

| Alias | Canonical command |
| --- | --- |
| `follow-up` | `continue` |
| `help-me` | `guide` |
| `todo` | `next` |
| `dash` | `dashboard` |
| `cockpit`, `side-panel` | `review-data` |
| `do` | `action` |
| `walkthrough`, `review-loop` | `review-flow` |
| `home` | `inbox` |
| `tasks` | `board` |
| `order` | `stack` |
| `tests` | `validate` |
| `validation-plan`, `test-checklist` | `test-plan` |
| `share` | `handoff` |
| `review-files`, `file-checklist` | `review-map` |
| `go`, `go-no-go` | `confidence` |
| `can-land` | `ready` |
| `recover` | `diagnose` |
| `last` | `delta` |
| `what-changed` | `new` |
| `done` | `mark-reviewed` |
| `done-file`, `reviewed-file` | `mark-file-reviewed` |
| `close` | `archive` |
| `changed-files` | `files` |
| `inspect` | `file` |
| `rewind-points` | `checkpoints` |
| `explain` | `why` |
| `review-plan` | `review` |
| `edit` | `open` |
| `land` | `apply` |
| `ship` | `finish` |
| `undo-last` | `undo` |

## Visibility in Help

The short `trail agent --help` output intentionally emphasizes the common
workflow. Specialist commands such as deep reports, provenance views, raw
diffs, recovery helpers, and integration packets remain supported even when
they are hidden from that first screen. This page documents both visible and
hidden commands.

## Related Documentation

- [Trail Agent overview](overview.md)
- [Trail Agent user guide](user-guide.md)
- [Trail Agent troubleshooting](troubleshooting.md)
- [Global options and environment](../reference/cli/global-options-and-env.md)
- [Errors and output formats](../reference/cli/errors-and-output-formats.md)

## Code Facts Used

- Agent arguments and aliases: `trail/src/cli/command/agent_args.rs`
- Agent dispatcher and task behavior: `trail/src/cli/command/handler/agent.rs`
- Human-readable output: `trail/src/cli/command/render/agent.rs`

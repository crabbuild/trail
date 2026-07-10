# Trail Agent User Guide

This guide walks through a complete Trail Agent task: setup, isolated agent
work, review, validation, safe apply, follow-up, and recovery.

## Prerequisites

Install Trail and initialize the repository first:

```sh
make install
trail init
trail status
```

If the workspace is already initialized, `trail init` is not needed.

## 1. Check the Agent Provider

Trail has terminal profiles for `claude-code`, `codex`, `cursor`, `gemini`,
`aider`, and `opencode`.

```sh
trail agent doctor --provider codex
```

The doctor checks provider and workspace readiness. Fix reported provider,
workspace, or workdir problems before starting a task.

For an ACP-capable editor, print a configuration snippet:

```sh
trail agent setup --provider codex --editor vscode
```

`--editor` defaults to `vscode`; `zed` and `generic` are also supported setup
targets. Paste the printed snippet into the editor's custom-agent settings.

## 2. Start an Isolated Terminal Task

Create a named task and launch the provider inside its workdir:

```sh
trail agent start --provider codex --name update-agent-docs
```

Trail creates a fresh task lane from the current base. To start from another
Trail ref, task, lane, or checkpoint, use `--from`:

```sh
trail agent start --provider codex --from main --name update-agent-docs
```

Choose a workdir mode when the default full materialization is unsuitable:

```sh
trail agent start --provider codex --workdir-mode overlay-cow
trail agent start --provider codex --workdir-mode nfs-cow
```

You can override a provider profile's executable by placing the command after
`--`:

```sh
trail agent start --provider codex -- codex --your-provider-flag
```

For editor-based ACP work, the configured editor entry point creates a fresh
task and records each prompt as a turn. The review workflow is the same whether
the task came from a terminal or an editor.

## 3. Find What Needs Attention

Start from the inbox:

```sh
trail agent
```

Useful multi-task views are:

```sh
trail agent inbox
trail agent board
trail agent stack
```

- `inbox` groups tasks by attention state and highlights new changes.
- `board` gives each task one low-noise next action.
- `stack` detects shared files and recommends a safe apply order.

If you have forgotten the workflow, use:

```sh
trail agent guide
trail agent ask what should I do next
```

## 4. Understand the Task

Open the compact dashboard, then inspect the changes:

```sh
trail agent dashboard latest
trail agent changes latest
trail agent changes latest --by-file
```

The most useful review views are:

```sh
trail agent delta latest                 # newest completed turn
trail agent new latest                   # changes since last review marker
trail agent timeline latest              # prompts, tools, checkpoints, files
trail agent files latest                 # file provenance
trail agent turn-diff latest --patch     # exact newest-turn patch
```

To investigate one file:

```sh
trail agent file latest README.md --patch
trail agent why latest README.md
trail agent focus latest --file README.md --patch
```

`why` connects the file to its prompt, turn, tools, and checkpoint. `focus`
combines review priority, provenance, and a focused diff.

## 5. Review Files Systematically

Ask Trail for the review checklist:

```sh
trail agent review-map latest
trail agent focus latest
```

Open the recommended file in `$EDITOR`:

```sh
trail agent open latest
trail agent open latest --print
```

After reviewing a file, mark it at the current checkpoint:

```sh
trail agent mark-file-reviewed latest README.md
```

If the agent changes that file again, it becomes unreviewed. When you have
reviewed the entire checkpoint, set the task-wide baseline:

```sh
trail agent mark-reviewed latest --note "Reviewed implementation and docs"
```

Now `trail agent new latest` reports only later changes.

## 6. Validate the Work

First inspect the suggested validation plan without running anything:

```sh
trail agent test-plan latest
trail agent validate latest
```

Run a test gate in the task workdir:

```sh
trail agent test latest -- cargo test
```

Run an evaluation gate in the same way:

```sh
trail agent eval latest --suite docs-quality -- ./scripts/check-docs.sh
```

Both commands record the command and result with the task. The default timeout
is 600 seconds; override it with `--timeout-secs`. Evaluations may also record a
numeric result:

```sh
trail agent eval latest \
  --suite quality \
  --score 0.96 \
  --threshold 0.90 \
  -- ./scripts/run-eval.sh
```

`validate` is read-only: it summarizes existing gates and recommends what to
run next. `test-plan` gives a prioritized checklist derived from changed areas.

## 7. Make a Go/No-Go Decision

Use these read-only checks before applying:

```sh
trail agent risk latest
trail agent confidence latest
trail agent ready latest
```

- `risk` explains risk signals and mitigations.
- `confidence` combines review, validation, risk, and Git preflight.
- `ready` reports blockers and warnings for the apply operation.

For a one-page post-run view, use:

```sh
trail agent summary latest
```

## 8. Preview and Apply

Always preview the operation:

```sh
trail agent apply latest --dry-run
```

If preflight succeeds, apply it:

```sh
trail agent apply latest
```

Trail uses the task title as the default commit message. Override it only when
needed:

```sh
trail agent apply latest -m "docs: add Trail Agent guide"
```

To apply and remove the task from normal inbox views in one step:

```sh
trail agent finish latest
```

`apply` is also available as `land`; `finish` is also available as `ship`.

## 9. Continue After Apply

An applied task is complete. Start subsequent work in a fresh task based on its
checkpoint:

```sh
trail agent continue latest --name agent-docs-follow-up
```

You can override the provider and workdir mode:

```sh
trail agent continue latest \
  --provider claude-code \
  --workdir-mode full-cow
```

## 10. Share the Result

Generate local, copyable artifacts from the recorded task:

```sh
trail agent receipt latest
trail agent handoff latest
trail agent report latest --markdown
trail agent pr latest
```

- `receipt` is a concise after-action record.
- `handoff` includes state, risks, review commands, validation, and next steps.
- `report` is the deeper review bundle.
- `pr` prints a draft title and body; it does not create a remote pull request.

Use `trail agent pr latest --title-only` or `--body-only` in scripts.

## 11. Undo or Rewind Safely

Inspect the available recovery points first:

```sh
trail agent diagnose latest
trail agent checkpoints latest
```

Undo the latest completed prompt turn:

```sh
trail agent undo latest
```

Target a particular turn or prompt:

```sh
trail agent undo latest --turn 2
trail agent undo latest --prompt "Add hook support"
```

For an explicit checkpoint or friendly label:

```sh
trail agent rewind latest --to before-last-turn
trail agent rewind latest --to turn:2
trail agent rewind latest --to before-turn:2
trail agent rewind latest --to 'before-prompt:Add hook support'
trail agent rewind latest --to before-last-operation
```

Undo and rewind change the task lane, not the active Git branch. Review the
resulting delta before continuing.

## 12. Archive and Restore Tasks

Archive hides a task but preserves its lane, transcript, checkpoints, and
provenance:

```sh
trail agent archive latest --note "Completed"
```

List or restore archived work:

```sh
trail agent inbox --all
trail agent list --all
trail agent unarchive <TASK>
```

## Automation and Structured Output

Trail's global JSON output is useful for scripts and editor integrations:

```sh
trail --json agent dashboard latest
trail --json agent review-data latest
```

`review-data` is the structured editor-panel packet. It includes review
progress, focus, validation, risk, readiness, and typed actions. Use `action`
to inspect or run an advertised action:

```sh
trail agent action
trail agent action inspect_focus_file --print
trail agent action latest inspect_focus_file
```

Actions classified as confirmation-required need `--confirm` before Trail will
run them.

## Related Documentation

- [Trail Agent overview](overview.md)
- [Trail Agent CLI reference](cli-reference.md)
- [Trail Agent troubleshooting](troubleshooting.md)
- [ACP integration](../integrations/acp.md)
- [Tests, evals, gates, and readiness](../lanes/tests-evals-gates-and-readiness.md)

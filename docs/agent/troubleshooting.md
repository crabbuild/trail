# Trail Agent Troubleshooting

Start with Trail's state-aware diagnosis commands:

```sh
trail agent guide
trail agent diagnose latest
trail agent doctor codex
```

`diagnose` inspects an existing task and suggests safe recovery actions.
`doctor` checks whether a provider and workspace are ready to start tasks.

## No Agent Task Exists

Commands using `latest` cannot select a task until an editor prompt or terminal
task has created one. Configure an editor or start a terminal task:

```sh
trail agent acp setup codex
trail agent start codex --name first-task
```

`trail agent action` also publishes first-run setup, doctor, and start actions
when no task exists.

## The Wrong Task Is Selected

`latest` ignores archived tasks and selects the newest active task. List the
stable identifiers and pass one explicitly:

```sh
trail agent list --all
trail agent dashboard <TASK>
```

Commands accept a task ID, lane, session, or ACP session selector.

## The Provider Cannot Start

Run the provider check:

```sh
trail agent doctor claude-code
trail agent doctor codex
```

Confirm that the provider executable is installed and available in `PATH`. If
you need a custom executable or flags, override the profile after `--`:

```sh
trail agent start codex -- /absolute/path/to/codex --flag
```

## FUSE COW Mount Fails

`fuse-cow` requires macFUSE on macOS or FUSE access such as `/dev/fuse` on
Linux. Trail reports the mount error and does not silently fall back to a full
copy.

Use the portable mode while diagnosing the host setup:

```sh
trail agent start codex --workdir-mode native-cow
```

On macOS, `nfs-cow` is another large-workspace option:

```sh
trail agent start codex --workdir-mode nfs-cow
```

See [Spawn and materialize workdirs](../lanes/spawn-and-materialize-workdirs.md)
for platform requirements and mount behavior.

## Changes Are Missing

First verify which task workdir the agent used:

```sh
trail agent workdir latest
trail agent timeline latest
trail agent files latest
```

Then check the newest turn and raw task diff:

```sh
trail agent delta latest --patch
trail agent diff latest --stat --patch
```

If the expected command ran outside the task workdir, Trail cannot attribute
those filesystem edits to that task.

## A File Appears Reviewed and Then Becomes Unreviewed

File review markers are checkpoint-aware. If the agent changes the file after
you mark it reviewed, Trail correctly returns it to the review queue.

Inspect the new changes and mark the file again:

```sh
trail agent new latest --file README.md --patch
trail agent mark-file-reviewed latest README.md
```

## Apply Is Blocked

Do not bypass the preflight. Ask Trail for the specific blocker:

```sh
trail agent ready latest
trail agent risk latest
trail agent diagnose latest
trail agent apply latest --dry-run
```

Common causes include:

- The active Git worktree no longer matches Trail's recorded apply base.
- Another task overlaps the same files.
- Required review or validation work is incomplete.
- The task is conflicted, blocked, still running, or has no applicable changes.
- The task was already applied.

For multiple tasks, inspect overlap and order before applying either task:

```sh
trail agent board
trail agent stack
trail agent compare <TASK_A> <TASK_B>
```

If a task is already applied, create a follow-up instead of reusing it:

```sh
trail agent continue <TASK>
```

## A Validation Gate Failed or Timed Out

Review the recorded result and suggested plan:

```sh
trail agent validate latest
trail agent test-plan latest
```

The default gate timeout is 600 seconds. Increase it for a legitimately longer
command:

```sh
trail agent test latest --timeout-secs 1800 -- cargo test --workspace
```

Run commands after `--`; otherwise provider or Trail arguments may consume
them.

## The Latest Prompt Went Sideways

Inspect before changing task history:

```sh
trail agent delta latest --patch
trail agent diagnose latest
trail agent checkpoints latest
```

Then undo the prompt-sized turn:

```sh
trail agent undo latest
```

Use an explicit target only when necessary:

```sh
trail agent undo latest --turn 2
trail agent rewind latest --to before-turn:2
```

After recovery, run `trail agent delta latest --patch` again to verify the task
state.

## An Archived Task Disappeared

Archiving removes a task from default inbox, board, list, and `latest`
selection. It does not delete task history.

```sh
trail agent list --all
trail agent inbox --all
trail agent unarchive <TASK>
```

## An Editor Cannot Use Human-Readable Output

Use Trail's global structured output and the review-data packet:

```sh
trail --json agent review-data latest
trail --json agent dashboard latest
```

Do not parse the formatted terminal tables. `review-data` publishes stable
action IDs, safety classes, disabled reasons, and structured arguments for
editor integrations.

## Collecting a Support Bundle

These read-only commands capture the most useful task context:

```sh
trail agent summary latest
trail agent report latest --markdown
trail agent timeline latest
trail agent workdir latest
trail agent doctor <PROVIDER>
```

For workspace-level corruption or storage problems, continue with the general
[maintenance and recovery guide](../guides/maintenance-and-recovery.md).

# Managed Task Lifecycle

## Launch or Select

Check provider readiness before launching:

```sh
trail agent doctor codex
trail agent start codex --name <task-name>
```

Built-in terminal profiles include `claude-code`, `codex`, `cursor`, `gemini`, `aider`, and `opencode`. Override the provider command only after `--`. For several tasks, use `trail agent inbox`, `board`, and `stack`; `stack` identifies overlapping files and suggests order.

## Inspect and Review

```sh
trail agent dashboard <task>
trail agent changes <task> --by-file
trail agent review-map <task>
trail agent focus <task> --patch
trail agent why <task> path/to/file
trail agent file <task> path/to/file --patch
```

After inspecting the current checkpoint:

```sh
trail agent mark-file-reviewed <task> path/to/file --note "Reviewed current checkpoint"
trail agent mark-reviewed <task> --note "Reviewed implementation and evidence"
```

## Validate

Ask Trail for a read-only plan, then run the exact relevant commands as recorded gates:

```sh
trail agent test-plan <task>
trail agent validate <task>
trail agent test <task> --suite unit -- cargo test
trail agent eval <task> --suite quality -- ./scripts/run-eval.sh
```

Never invent a passing gate or reuse evidence from a different checkpoint or environment generation.

## Decide, Hand Off, and Apply

```sh
trail agent risk <task>
trail agent confidence <task>
trail agent ready <task>
trail agent handoff <task>
trail agent report <task> --markdown
trail agent pr <task>
trail agent apply <task> --dry-run
```

`trail agent pr` prints a draft; it does not create a remote pull request. Stop on readiness blockers. Run non-dry-run `apply` or `finish` only with explicit Git handoff authority. `finish` archives only after successful application.

## Recover Deliberately

```sh
trail agent diagnose <task>
trail agent delta <task> --patch
trail agent checkpoints <task>
```

Use `undo` for a prompt-sized turn and `rewind --to` for a known checkpoint. Both change task history and require explicit intent. Preserve and inspect the failed head before recovery.

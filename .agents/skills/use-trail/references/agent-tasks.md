# High-Level Trail Agent Tasks

Use this surface when the user is operating coding-agent tasks. It owns task lanes, workdirs, transcripts, checkpoints, review state, gates, and safe Git application. Do not use it to launch another provider from inside an already-running Trail task.

## Start or Join

Check prerequisites:

```sh
trail agent doctor --provider codex
trail agent setup --provider codex --editor vscode
```

Start one isolated terminal task:

```sh
trail agent start --provider codex --name <task-name>
```

Profiles include `claude-code`, `codex`, `cursor`, `gemini`, `aider`, and `opencode`. Use `--workdir-mode full-cow` as the portable default. Use `overlay-cow` only when FUSE is available and `nfs-cow` on macOS when its tradeoffs are acceptable. Override the provider command only after `--`.

If already launched inside the task, edit and test in the provided workdir. Do not create a nested task.

## Orient

```sh
trail agent
trail agent guide
trail agent dashboard latest
trail agent ask what should I do next
```

`trail agent ask` is deterministic phrase routing, not an LLM call. With multiple tasks, use `inbox`, `board`, and `stack`, then replace `latest` with the stable task ID or lane name:

```sh
trail agent inbox
trail agent board
trail agent stack
trail agent list --all
```

Use `stack` before applying overlapping tasks; it identifies shared files and suggests order.

## Inspect and Review

Start broad, then focus:

```sh
trail agent changes <task> --by-file
trail agent new <task>
trail agent review-map <task>
trail agent focus <task> --patch
trail agent why <task> <path>
trail agent file <task> <path> --patch
```

Useful causal views are `timeline`, `delta`, `turn`, `turn-diff`, `files`, and `tools`. Mark a file reviewed only after inspecting it at the current checkpoint:

```sh
trail agent mark-file-reviewed <task> <path> --note "Reviewed at current checkpoint"
trail agent mark-reviewed <task> --note "Reviewed implementation and evidence"
```

Later edits invalidate the relevant checkpoint-aware marker. Re-run `new` and review again.

## Validate

First request the read-only plan:

```sh
trail agent test-plan <task>
trail agent validate <task>
```

Then run appropriate commands in the task workdir and record the gates:

```sh
trail agent test <task> --suite unit -- cargo test
trail agent eval <task> --suite quality -- ./scripts/run-eval.sh
```

The default timeout is 600 seconds. Increase `--timeout-secs` only for a legitimately longer command. Never invent a passing gate or treat a command run outside Trail as recorded evidence.

## Decide and Apply

Use the full read-only preflight sequence:

```sh
trail agent risk <task>
trail agent confidence <task>
trail agent ready <task>
trail agent apply <task> --dry-run
```

Stop on blockers. Non-dry-run `apply` can record the task workdir, merge inside Trail, create a Git commit, and fast-forward the current Git branch. Run it only when the user has clearly authorized that handoff:

```sh
trail agent apply <task>
```

`trail agent finish <task>` performs the same apply flow and archives only after success. Do not use `finish` merely to tidy the inbox.

After an applied task, create a follow-up rather than reusing completed history:

```sh
trail agent continue <task> --name <follow-up-name>
```

## Recover and Hand Off

Inspect before moving task history:

```sh
trail agent diagnose <task>
trail agent delta <task> --patch
trail agent checkpoints <task>
```

Use `undo` for the latest prompt-sized turn or `rewind --to` for an explicit checkpoint/friendly target. Both change the task lane, not the active Git branch, but still require deliberate confirmation.

Share recorded context with:

```sh
trail agent receipt <task>
trail agent handoff <task>
trail agent report <task> --markdown
trail agent pr <task>
```

`pr` only prints a draft; it does not create a remote pull request. `archive` preserves history and removes the task from normal `latest`/inbox selection; use `list --all` and `unarchive` to recover it.

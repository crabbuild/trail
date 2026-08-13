---
name: trail-agent-tasks
description: Operate Trail's high-level managed coding-agent tasks. Use when a user asks to launch or inspect a Trail agent task; navigate its dashboard, inbox, stack, changes, checkpoints, or review map; run and record tests or evals; mark review evidence; diagnose or rewind a task; generate a handoff or PR draft; check readiness; or safely preview and apply completed task work to Git.
---

# Trail Agent Tasks

Use the high-level `trail agent` surface for human-supervised coding-agent tasks. It owns task lanes, workdirs, transcripts, checkpoints, review state, gates, and safe Git application.

## Avoid Recursive Launches

If `TRAIL_LANE` or `TRAIL_VIEW` is set, continue the assigned work inside the current task. Do not call `trail agent start` or create another managed task unless the outer coordinator explicitly requests delegation.

## Follow the Task Lifecycle

Read [task-lifecycle.md](references/task-lifecycle.md) before launching, reviewing, validating, recovering, or applying a task. Use explicit task IDs when `latest` would be ambiguous.

Start with read-only orientation:

```sh
trail agent dashboard latest
trail agent changes latest --by-file
trail agent new latest
```

Do not treat a normal command transcript as a Trail gate. Do not mark work reviewed without inspecting the current checkpoint. Later edits invalidate checkpoint-aware review evidence.

Non-dry-run `apply` and `finish` may record the task workdir, merge in Trail, create a Git commit, and fast-forward the current Git branch. Require clear user intent, readiness, and an apply dry-run before crossing that boundary.

---
name: use-trail
description: Operate Trail, the local-first operation database that adds agent tasks, isolated lanes, transcripts, checkpoints, provenance, readiness, and safe Git handoff to code worktrees. Use when an agent must initialize or inspect a Trail workspace; record or explain local edits; launch, review, validate, recover, hand off, or apply a Trail Agent task; create or manage a lane; use structured patches, gates, approvals, merge queues, MCP, ACP, or the HTTP daemon; or diagnose Trail errors and blocked merges.
---

# Use Trail

Treat Trail as local operational memory beside Git. Use Git for shared committed history; use Trail for local attempts, task isolation, provenance, review evidence, recovery, and pre-commit coordination. Never imply that a Trail branch or lane creates or switches a Git branch.

## Orient Before Acting

1. Determine whether the user is operating agent tasks, working inside a lane, recording ordinary local work, or building an integration.
2. Locate the executable with `command -v trail`. Inspect the relevant command with `trail --help` and `trail <group> <command> --help`; Trail does not currently expose `--version`.
3. Locate workspace state without mutating it. Trail walks upward for `.trail`; use `trail --json status` or an explicit `trail --workspace <root> --json status` in automation.
4. Inspect Git and worktree state separately when a task may ultimately be applied to Git.
5. If no Trail workspace exists, initialize only after choosing the intended baseline:
   - Use `trail init --from-git` for Git-tracked state.
   - Use `trail init --working-tree` for visible current files.
   - Use `trail init` for an empty Trail root.
   Inspect `.trailignore` before importing sensitive or generated content.

## Choose the Correct Surface

- Use the high-level `trail agent` workflow when a human wants Trail to launch, review, validate, recover, or safely apply coding-agent tasks. Read [agent-tasks.md](references/agent-tasks.md).
- Use `trail lane` primitives when the current agent or another tool needs a directly controlled isolated work container, structured patching, sessions/turns, gates, handoff, or merge queues. Read [lanes.md](references/lanes.md).
- Use core commands for ordinary local recording, branches, provenance, Git interop, and maintenance. Read [core-workflows.md](references/core-workflows.md).
- Before recovery, merge, force, bypass, approval, or external-service work, read [safety-and-recovery.md](references/safety-and-recovery.md).
- For MCP, ACP, daemon/HTTP, editor, or script integration work, read [integrations.md](references/integrations.md).

Do not launch `trail agent start` recursively when already running as the provider inside a Trail task workdir. Continue the current task and let the outer operator use `trail agent` review/apply commands.

## Apply the Read-Preview-Mutate-Verify Loop

1. Read current state using status, dashboard, diff, readiness, or diagnosis commands.
2. Preview any consequential action with its dry-run or preview form.
3. Explain blockers instead of bypassing them.
4. Perform only the scoped mutation the user authorized.
5. Re-read state and record evidence such as a lane operation, test/eval gate, review marker, handoff, or receipt.

Treat non-dry-run Git apply/finish, merges into shared refs, lane merge-queue execution, rewind/undo, conflict resolution, restore, garbage collection, lane removal, and force/bypass flags as consequential. Require clear user intent before using them. Never substitute `--allow-stale`, `--allow-ignored`, `--force`, `--direct`, or `--no-auth` for resolving the underlying safety condition.

## Use Stable Automation Patterns

- Put global flags before the command: `trail --json agent dashboard latest`.
- Prefer explicit task/lane selectors when more than one exists; `latest` excludes archived tasks and can become ambiguous to a human.
- Put test/eval commands after `--`: `trail agent test <task> -- cargo test`.
- Parse `--json` output and stable error codes, not human tables.
- Pass `--workspace <root>` from lane workdirs, scripts, and integrations to avoid accidental workspace discovery.
- Use read-only reports first. MCP hosts should honor Trail's tool risk annotations and request confirmation for destructive or open-world tools.

## Complete the Workflow

For an agent task, finish with reviewed changes, recorded validation, `ready`, and an apply dry-run; apply or finish only when explicitly intended. For a lane, finish with a recorded clean workdir, review/readiness evidence, a merge dry-run, and queued merge when the target is shared. For local recording, finish with status/diff verification and the requested provenance or history result. Always report remaining blockers and the exact safe next command.

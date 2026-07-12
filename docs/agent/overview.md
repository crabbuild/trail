# Trail Agent Overview

`trail agent` is Trail's high-level workflow for running, reviewing, validating,
and applying coding-agent tasks. It presents human task names and familiar
actions while Trail manages the underlying lane, workdir, transcript, turns,
checkpoints, and provenance.

Use this interface for normal agent work. Use the lower-level `trail lane` and
`trail acp` commands when you need direct control over those primitives.

## What Trail Agent Provides

Each task has an isolated Trail lane and, for terminal agents, a materialized
workdir. Trail records the task's prompts, tool activity, changed files, and
checkpoints. This enables you to:

- Run supported terminal agents without modifying the active Git branch.
- See which tasks need attention from one inbox or multi-task board.
- Review changes by task, prompt, turn, change group, or file.
- Trace a file back to the prompt and tools that changed it.
- Record test and evaluation results with the task.
- Check risk and apply readiness without changing Git.
- Undo a prompt-sized turn or rewind to a known checkpoint.
- Apply a reviewed task to the current Git branch safely.
- Generate receipts, handoff notes, review reports, and pull-request drafts.

## The Task Lifecycle

```text
setup -> start or editor prompt -> inspect -> review -> validate -> apply -> archive
                                      |                      |
                                      +---- undo/rewind <----+
```

1. **Setup:** Check the provider and print editor configuration.
2. **Start:** Create a fresh task from the current state or another Trail ref.
3. **Inspect:** Let the agent work, then inspect its transcript and changes.
4. **Review:** Work through the highest-priority files and mark them reviewed.
5. **Validate:** Run and record appropriate tests or evaluations.
6. **Apply:** Preview the Git handoff, then apply only when preflight succeeds.
7. **Archive:** Hide completed tasks without deleting their history.

If more work is needed after applying, `trail agent continue` starts a fresh
follow-up task from the applied checkpoint. It does not reuse the completed
task's history.

## The Everyday Interface

You do not need to memorize every subcommand. These commands cover the normal
path:

```sh
trail agent guide
trail agent
trail agent ask what should I do next
trail agent changes latest
trail agent focus latest
trail agent validate latest
trail agent ready latest
trail agent apply latest --dry-run
trail agent finish latest
```

Running `trail agent` without a subcommand shows the current task dashboard, or
the grouped inbox when multiple tasks exist. `latest` selects the newest
non-archived task in commands that accept a task selector.

`trail agent ask` uses deterministic phrase routing to open the appropriate
Trail view. It does not send your question to a language model. For example:

```sh
trail agent ask what changed
trail agent ask what should I review first
trail agent ask explain README.md
trail agent ask is it safe to land
trail agent ask what tests should I run
```

## Tasks and Selectors

Most commands accept any of the following:

- A Trail Agent task ID.
- The task's lane name.
- A session or ACP session ID.
- `latest`, which is the default for most single-task commands.

Human output leads with a task title derived from `--name` or the prompt, while
also showing the stable identifier when precision is necessary. Archived tasks
are excluded from `latest` and normal inbox/list views.

## Isolation and Workdirs

Terminal tasks support these workdir modes:

| Mode | Behavior | Typical use |
| --- | --- | --- |
| `full-cow` | Materializes the full task root using filesystem cloning when available. | Portable default. |
| `fuse-cow` | Mounts a FUSE-backed copy-on-write view for the duration of the agent run. | Avoid copying a large tree on Linux/macOS with FUSE available. |
| `nfs-cow` | Mounts a loopback NFSv3 copy-on-write view on macOS. | Large macOS workspaces without macFUSE. |
| `dokan-cow` | Mounts a Dokan-backed copy-on-write view on Windows. | Large Windows workspaces with Dokan 2.x. |

The default for `trail agent start` and `trail agent continue` is `full-cow`.
FUSE, NFS, and Dokan mounts exist only while the terminal agent is running. Trail
records their writable changes before unmounting.

## Safety Model

`trail agent apply` is intentionally conservative. It records dirty task
workdirs, verifies that the current Git tree still matches Trail's apply base,
merges the task inside Trail, creates a Git commit, and fast-forwards the
current branch only when those checks succeed.

Always preview first:

```sh
trail agent ready latest
trail agent apply latest --dry-run
trail agent apply latest
```

`trail agent finish` performs the same apply flow and archives the task after a
successful result. Neither `ready` nor `--dry-run` mutates Git.

## Where to Go Next

- [Trail Agent user guide](user-guide.md)
- [Trail Agent CLI reference](cli-reference.md)
- [Trail Agent troubleshooting](troubleshooting.md)
- [Agent workflow quick start](../getting-started/agent-workflow.md)
- [Workdir modes](../lanes/spawn-and-materialize-workdirs.md)
- [Readiness gates and merge safety](../concepts/readiness-gates-and-merge-safety.md)

## Code Facts Used

- CLI definitions: `trail/src/cli/command/agent_args.rs`
- Command handling: `trail/src/cli/command/handler/agent.rs`
- Reports and terminal rendering: `trail/src/cli/command/render/agent.rs`

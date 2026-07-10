# Trail

Trail gives AI coding agents branch-like memory, transcripts, checkpoints, and
rewind without polluting your active Git branch.

Trail is a local-first operation database for code and text worktrees. It records
the meaningful work that happens between Git commits: local edits, recorded
operations, branch movement, lane patches, review handoffs, merges, and
line-level provenance.

Git remains the source of shared repository history. Trail adds a local layer for
questions Git does not answer well by itself:

- What operation introduced this current line?
- What changed on this branch before it became a Git commit?
- What happened in a lane, which paths changed, and is it ready to merge?
- Which changes are blocked by conflicts, pending approvals, dirty workdirs, or
  missing test/eval gates?
- Can an editor, agent host, or local service query the same state through CLI,
  HTTP, MCP, or Rust?

Trail stores its local state under `.trail/`, uses `.trailignore` and Git ignore
files to avoid accidental private/generated captures, and can run fully through
the CLI without a background service. The HTTP daemon and MCP stdio server are
opt-in integration surfaces.

## Trail vs Git

Trail is not a Git replacement. It sits next to Git as a local operation,
provenance, and lane-coordination layer.

```text
  human edits, editor saves, lane turns, tool calls, patches
                         |
                         v
  +---------------------------------------------------------+
  | Trail                                                   |
  | record operations, preserve line identity, isolate      |
  | lane branches, run guardrails, check readiness,         |
  | produce review and handoff reports                      |
  +--------------------------+------------------------------+
                             |
                             v
  reviewed state, resolved conflicts, accepted operations
                             |
                             v
  +---------------------------------------------------------+
  | Git                                                     |
  | commit, branch, tag, push, pull, rebase, share history  |
  | with remotes and existing developer workflows           |
  +---------------------------------------------------------+
```

Use Git for durable shared source-control history. Use Trail for the messy,
high-frequency, local work that happens before a commit is ready.

| Need | Git | Trail |
| --- | --- | --- |
| Shared project history | Excellent: commits, branches, remotes, tags, PR workflows | Complements Git and can import/export mappings |
| Local in-between work | Mostly unstaged/staged diffs, stash, reflog | First-class recorded operations with messages, actors, roots, and parents |
| Line provenance | Blame by committed lines | Stable `LineId` history for current and recorded local lines |
| Lane isolation | Branches and worktrees, but no agent activity model | `refs/lanes/<name>`, sessions, turns, traces, patches, gates, handoffs |
| Safety before mutation | Hooks and review conventions | Ignore policy, guardrails, approvals, leases, readiness blockers |
| Machine interfaces | Git CLI and plumbing | CLI JSON, HTTP/OpenAPI, MCP tools/resources/prompts, Rust reports |
| Merge readiness | Merge attempt plus conflicts | Readiness reports before merge: conflicts, dirty workdirs, approvals, gates |

The practical position is:

- Git is the publication and synchronization layer.
- Trail is the local operational memory and coordination layer.
- Git answers "what committed snapshot did the project accept?"
- Trail answers "what happened locally, why did it happen, which lane contains it,
  is it safe to accept, and what still blocks merge?"

## Branches and Lanes: Mental Model

Trail has two kinds of local code refs that serve different jobs:

- A **Trail branch** is a long-lived line of code state, such as `main`,
  `release`, or `experiment`.
- A **lane** is a short-lived, isolated work container created from a branch for
  one task, person, automation run, or coding agent.

Think of a lane as **a branch plus the context needed to safely finish one
piece of work**.

```text
Git branch (shared, committed history)
                |
                | import / export / safe apply
                v
Trail branch (long-lived local code line, for example main)
                |
                | spawn a lane for each active task
                +------------------------------+
                |                              |
                v                              v
      lane: fix-login                 lane: update-docs
      edits, tests, review            edits, tests, review
      approvals, handoff              approvals, handoff
                |                              |
                +---------- merge when ready --+
                               |
                               v
                    Trail branch: main
```

| Use this | When you need | Examples |
| --- | --- | --- |
| **Trail branch** | A durable local code line that can receive completed work | `main`, `release`, a long-running experiment |
| **Lane** | One bounded piece of active work that needs isolation, review, validation, recovery, or coordination | A bug fix, a documentation update, one agent task, a migration attempt |

A branch stores code state and its operation history. A lane has its own
branch-backed code state **and** records the work around it: sessions, turns,
messages, approvals, test/eval gates, workdirs, handoffs, and rewind
checkpoints. This is why a lane can answer both “what changed?” and “what
happened while making that change?”

A lane is not a paired Git branch. Spawning `fix-login` creates the Trail ref
`refs/lanes/fix-login`; it does not create or switch to a Git branch with that
name. A lane can also be virtual, so it has no filesystem workdir until a tool
needs real files. Git branches are already lightweight refs; lanes are lighter
to start only when virtual, and intentionally carry more metadata once work
begins.

The usual workflow is:

```text
choose or create a Trail branch
        -> spawn a lane from it
        -> edit, test, review, or hand off work in the lane
        -> merge the ready lane into the Trail branch
        -> export or safely apply accepted state to Git
```

For example:

```sh
# `main` is the long-lived Trail branch.
trail lane spawn fix-login --from main --materialize=true

# Work is isolated in the lane until it is reviewed and validated.
trail lane record fix-login -m "Fix login validation"
trail lane readiness fix-login
trail merge-lane fix-login --into main --dry-run
```

## Why This Matters for AI Agents

AI coding agents produce more than final diffs. They create attempts,
intermediate patches, tool calls, test runs, review notes, approvals, and
handoffs. Treating all of that as either "unstaged changes" or "a Git commit"
loses important context.

Trail gives lane workflows a native coordination layer:

- Each external agent can work inside an isolated Trail lane without
  immediately touching `main`.
- Structured patches can target stable file and line identity instead of fragile
  line numbers.
- Sessions, turns, messages, events, and trace spans preserve what happened in
  the lane and why.
- Guardrail checks and human approvals make risky actions explicit before they
  mutate the workspace or external systems.
- Test/eval gates, dirty-workdir checks, open conflicts, and pending approvals
  roll up into one readiness signal.
- Handoff and contribution reports give humans or other agents enough context to
  continue, review, or reject lane work.
- Merge queues serialize accepted lane work so parallel agents do not silently
  overwrite each other.

In short: Git is where accepted code history lives. Trail is where local and
lane work becomes understandable, reviewable, and mergeable before it becomes
Git history.

## Who It Is For

Trail is useful for several overlapping audiences.

**Developers working locally:** record useful worktree operations before they
become commits, inspect branch history, ask why a line exists, and safely
checkout or merge local refs.

**Reviewers and maintainers:** inspect provenance, changed paths, operation
timelines, conflict sets, anchors, and diagnostics before accepting work.

**Coding-agent operators:** give each external agent an isolated lane, durable
sessions and turns, structured patches, trace events/spans, guardrail checks, human
approvals, test/eval gates, readiness reports, handoff packets, and serialized
merge queues.

**Tool and integration authors:** use human/JSON CLI output, a loopback HTTP API
with OpenAPI 3.1, an MCP stdio server with tools/resources/prompts, or the Rust
`Trail` API and exported model/report types.

## What Trail Provides

- Local operation history with content-addressed objects, refs, roots, and
  rebuildable indexes.
- Stable `ChangeId`, `FileId`, and `LineId` identity for provenance and
  line-aware patching.
- Worktree status, selective recording, timeline, show, diff, checkout, branch,
  merge, why, history, and code-from workflows.
- Ignore policy and guardrail preflight for private paths, ignored files, risky
  shell/network/deploy/destructive actions, and workspace policy rules.
- Lane branches under `refs/lanes/<name>` with optional materialized or sparse
  workdirs.
- Durable lane sessions, turns, messages, events, trace spans, paused run
  checkpoints, approvals, tests, evals, readiness, contribution, and handoff
  reports.
- Direct lane merges and merge queues with readiness checks, conflict sets, and
  manual conflict resolution.
- Git import/export mappings, backup/restore, fsck, index rebuild, garbage
  collection, and doctor diagnostics.
- CLI, HTTP daemon, OpenAPI, MCP, and Rust library integration surfaces backed by
  the same core implementation.

## Architecture at a Glance

Trail is organized around one core library object, `Trail`. The CLI, HTTP
daemon, MCP server, tests, and Rust callers all route through that same core so
they can share behavior and report types.

![Trail architecture diagram](diagram/trail-architecture/trail-architecture@2x.png)

The same architecture in text form:

```text
                         entry points
  +-----------+   +---------------+   +--------------+   +-------------+
  | CLI       |   | HTTP daemon   |   | MCP stdio    |   | Rust API    |
  | trail     |   | /v1 JSON API  |   | tools/docs   |   | Trail       |
  +-----+-----+   +-------+-------+   +------+-------+   +------+------+
        |                 |                  |                  |
        +-----------------+------------------+------------------+
                                     |
                                     v
  +-------------------------------------------------------------------+
  | Trail core                                                        |
  | workspace policy, refs, objects, records, lanes, merges, reports  |
  +-----------+------------------+--------------------+---------------+
              |                  |                    |
              v                  v                    v
  +-------------------+  +------------------+  +----------------------+
  | SQLite            |  | Prolly maps      |  | .trail sidecars      |
  | objects, refs,    |  | path maps, file  |  | config, HEAD, refs,  |
  | indexes, queues,  |  | indexes, text    |  | daemon files,        |
  | lane state        |  | and line order   |  | workdir manifests    |
  +-------------------+  +------------------+  +----------------------+
```

### Command Flow

Most commands run directly against the local database. Selected hot paths can
use the daemon when a daemon URL is supplied or when `.trail/daemon.json` is
auto-discovered.

```text
  user or host
      |
      v
  trail CLI
      |
      v
  parse command + build RuntimeContext
      |
      +--> daemon-capable command and daemon available?
      |        |
      |        +-- yes --> HTTP daemon --> Trail core --> report JSON
      |
      +-- no or fallback --> local Trail core --> report struct
                                      |
                                      v
                         human output, JSON, or NDJSON
```

The important design point is that daemon-backed and local paths return the same
report shapes. A script can use CLI JSON, the HTTP API, MCP tools, or Rust types
without learning separate data models for each surface.

### Durable History Model

Trail records operations as durable history. Refs point to operation/root pairs;
operations point to parent operations and before/after roots; roots point to
ordered maps and content objects.

```text
  refs/branches/main              refs/lanes/doc-bot
          |                               |
          v                               v
  +----------------+              +----------------+
  | Operation C    |              | Operation D    |
  | after_root Rc  |              | after_root Rd  |
  +-------+--------+              +-------+--------+
          |                               |
          +---------------+---------------+
                          |
                          v
                  +---------------+
                  | Operation B   |
                  | parent: A     |
                  +-------+-------+
                          |
                          v
                  +---------------+
                  | Operation A   |
                  +---------------+

  each after_root
          |
          v
  +-------------------+      +-------------------+
  | WorktreeRoot      |----->| path map          |-----> FileEntry
  | map root ids      |      | path -> file      |
  +---------+---------+      +-------------------+
            |
            v
  +-------------------+      +-------------------+
  | file index map    |      | TextContent/Blob  |
  | FileId -> path    |      | lines or bytes    |
  +-------------------+      +-------------------+
```

The operation and object graph is the durable source of truth. SQLite indexes
such as `operations`, `file_history`, `line_history`, `messages`, trace spans,
and worktree scan rows make common queries fast and can be rebuilt from reachable
objects.

```text
  durable refs + operation objects + message objects
                    |
                    v
          index rebuild / fsck / gc
                    |
                    v
  derived query tables: timeline, history, why, code-from, traces
```

### Lane Coordination Model

A lane is a branch-backed work container. A normal branch stores code state; a
lane stores code state plus the work history around it: sessions, turns,
messages, events, spans, approvals, gates, workdirs, and rewind checkpoints.
Use branches for long-lived code lines such as `main` or `release`, and lanes
for active work by humans, automation, or external coding agents.

A normal lane workflow writes to `refs/lanes/<name>`, reviews readiness, then
merges into a target branch only after checks pass.

```text
  +-------------------+        +----------------------+
  | lanes             |        | lane_branches        |
  | identity, model,  |------->| ref, base/head root, |
  | provider, metadata|        | session, workdir,    |
  +-------------------+        | status               |
                               +----------+-----------+
                                          |
                                          v
                               refs/lanes/<name>
                                          |
                                          v
                               operation/root history

  activity around the branch:

  sessions -> turns -> messages/events/spans
       |          |          |
       v          v          v
  approvals   run states   test/eval gates
       \          |          /
        \         v         /
          readiness + handoff + merge queue
```

Materialized workdirs are optional. Structured patches can update a lane branch
without checking out a full filesystem tree; materialized or sparse workdirs are
available when tools need real files and command execution.

### Safety Boundaries

Trail's safety checks sit between user, automation, or agent requests and workspace mutation.

```text
  request
     |
     v
  normalize paths
     |
     v
  block .trail/.git/private hardcoded paths
     |
     v
  apply .trailignore and .gitignore policy
     |
     v
  guardrail risk check + workspace policy
     |
     v
  pending/approved/rejected human approvals
     |
     v
  allowed operation, approval_required report, or blocked report
```

These checks are deliberately local and explainable. They protect Trail/Git
internals, ignored/private paths, risky lane actions, dirty materialized
workdirs, stale refs, and conflicted merges before changes are accepted.

## Quick Start

Trail is a Rust workspace. The repository declares Rust 1.81 in `Cargo.toml`.
Build from source with the Makefile:

```sh
# Build the debug binary at target/debug/trail.
make build

# Print CLI help from the local debug binary.
target/debug/trail --help
```

Install a local optimized binary with the Makefile. By default this installs to
`$HOME/.cargo/bin/trail`:

```sh
# Build the release binary and install trail locally.
make install

# Verify the installed trail command is on your PATH.
trail --help
```

For ACP coding-agent setup, keep installation simple and use the guided Trail
commands after the binary is installed:

```sh
trail agent doctor --provider claude-code
trail agent doctor --provider codex
trail agent setup
```

For a project-local install directory, override `PREFIX`:

```sh
# Install to ./.local/bin/trail instead of $HOME/.cargo/bin/trail.
make install PREFIX="$PWD/.local"
```

The equivalent direct Cargo build command is:

```sh
# Build the debug binary without using the Makefile.
cargo build -p trail

# Print CLI help from the Cargo-built debug binary.
target/debug/trail --help
```

Initialize a workspace from the current working tree:

```sh
# Import visible working tree files into .trail/.
trail init --working-tree
```

Inspect and record an edit:

```sh
# Show whether the current worktree differs from Trail's recorded root.
trail status

# Record the current edit as a named local operation.
trail record -m "record current edit"

# List recent recorded operations.
trail timeline --limit 10

# Inspect one recorded operation from the timeline output.
trail show <change-id>
```

Ask provenance questions:

```sh
# Explain what operation introduced the current README.md line 2.
trail why README.md:2

# Show recorded history for README.md.
trail history README.md

# Show the current unrecorded worktree diff as a patch.
trail diff --dirty --patch
```

Start a lane for task work:

```sh
# Create an isolated lane branch with its own materialized workdir.
trail lane spawn docs-lane --from main --materialize=true

# Print the path to that workdir, then edit there or point a coding agent there.
trail lane workdir docs-lane

# Record, review, and check readiness before merge.
trail lane record docs-lane -m "record docs update"
trail lane diff docs-lane --patch
trail lane readiness docs-lane
trail merge-lane docs-lane --into main --dry-run
```

Example CLI output from a tiny workspace looks like this. IDs, object hashes,
workspace IDs, and actor names will differ on your machine.

## Common ID prefixes:

| Prefix | Meaning | Example use |
| --- | --- | --- |
| `wk_` | Workspace ID derived when `.trail/` is initialized | Identifies one local Trail workspace |
| `ch_` | Change/operation ID allocated when Trail records an operation | Appears as `Head`, `Initial operation`, timeline entries, and `show` selectors |
| `obj_` | Content-addressed object ID | Identifies stored roots, operations, text objects, blobs, and other durable objects |
| `msg_` | Message ID | Used for durable operation, agent, or review messages |
| `anc_` | Anchor ID | Used for durable labels tied to file and line identity |
| `ch_...:<n>` | Stable file or line identity with an origin change and local sequence | Appears in `why` output as a `Line ID` |

## Example output

1. Initialize Trail from the visible files in the current working tree. The
   output shows the workspace ID, active branch, initial operation ID, and import
   summary.

```text
$ trail init --working-tree
Initialized Trail workspace
Workspace: wk_24ec99f68d1db8716f4df8a87580e3da
Branch: main
Initial operation: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Imported: 1 files (1 text, 0 opaque, 0 binary)
```

2. Check the recorded branch and worktree state. Immediately after
   initialization, the worktree is clean.

```text
$ trail status
Branch: main
Head: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Root: obj_46b1a72c6ff5e66a7b3026113243681493e79c2e659b6ef9658a2db57fdac431
Worktree: clean
```

3. After editing `README.md`, run `status` again. Trail reports the worktree as
   dirty and lists the modified path.

```text
$ trail status
Branch: main
Head: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Root: obj_46b1a72c6ff5e66a7b3026113243681493e79c2e659b6ef9658a2db57fdac431
Worktree: dirty
  Modified README.md
```

4. Record the edit as a named local operation. The output returns the new
   operation ID and the changed path summary.

```text
$ trail record -m "record current edit"
Recorded ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa
  Modified README.md
```

5. Read the recent operation timeline. The newest record appears first, followed
   by the initial import operation.

```text
$ trail timeline --limit 10
ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa ManualRecord main record current edit
ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6 GitImport main Initialize Trail workspace
```

6. Inspect one operation from the timeline. `show` expands the operation kind,
   actor, message, parent, before/after roots, and path-level summary.

```text
$ trail show ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa
Operation: ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa
Kind: ManualRecord
Branch: main
Actor: demo
Message: record current edit
Parents:
  ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Before root: obj_46b1a72c6ff5e66a7b3026113243681493e79c2e659b6ef9658a2db57fdac431
After root: obj_7e39865c8542fe846b528c28debed69daecc4b53c34ff17f8a4da8bacbb773a4
  Modified README.md (+1 -0)
```

7. Ask why a current line exists. `why` resolves the path and line number to a
   stable line ID, then shows the operation that introduced it and the last
   operation that changed its content.

```text
$ trail why README.md:2
README.md:2 First recorded line
Line ID: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6:2
Introduced by: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Last content change: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
```

8. Show file history. `history` lists operations that affected the selected
   file.

```text
$ trail history README.md
README.md
ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6 Added README.md
ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa Modified README.md
```

9. After making another edit without recording it, inspect the dirty diff. The
   patch shows what is currently in the worktree but not yet recorded by Trail.

```text
$ trail diff --dirty --patch
Diff main..dirty
  Modified README.md (+1 -0)
diff --trail a/README.md b/README.md
--- a/README.md
+++ b/README.md
 Trail sample
 First recorded line
 Second recorded line
+Unrecorded working tree line
```

Initialization creates `.trail/` state and a `.trailignore` file when needed.
Default ignore patterns protect Trail/Git internals, environment files, private
key/certificate files, dependency folders, build output, and coverage output.

Later examples use `trail` for readability. If the binary is not on your PATH,
install it with `make install` or replace `trail` with `target/debug/trail`.

## Common CLI Reference

| Command | Description |
| --- | --- |
| `trail init --working-tree` | Initialize `.trail/` from visible working tree files |
| `trail status` | Show branch head, root object, cleanliness, and changed paths |
| `trail record -m "<message>"` | Record current worktree changes as a named local operation |
| `trail record --paths <path>... -m "<message>"` | Record only selected paths |
| `trail watch --once` | Watch for changes and record once after debounce |
| `trail timeline --limit <n>` | List recent operations, newest first |
| `trail show <selector>` | Inspect an operation, message, ref, or object |
| `trail diff --dirty --patch` | Show unrecorded worktree changes as a patch |
| `trail why <path:line>` | Explain which operation introduced a current line |
| `trail history <path>` | Show recorded history for a file or selector |
| `trail code-from <selector>` | Find changed paths connected to a message, session, or agent |
| `trail branch` | List local Trail branches |
| `trail branch <name> --from <ref>` | Create a branch from another ref |
| `trail checkout <branch> --dry-run` | Preview checkout effects before changing the worktree |
| `trail merge <branch> --into <target> --dry-run` | Preview a branch merge and possible conflicts |
| `trail ignore check <path>` | Check whether ignore policy records or skips a path |
| `trail guardrails check --action <action>` | Preflight a risky action against workspace policy |
| `trail agent setup` | Print editor config for fresh Trail agent tasks |
| `trail agent continue latest` | Start a fresh follow-up task from the latest task checkpoint |
| `trail agent` | Open the agent inbox home view with grouped tasks, review-first hints, and one next action |
| `trail agent board` | Show a multi-agent board with low-noise columns and one next action |
| `trail agent ask show agent board` | Route a plain-language question to the multi-agent board |
| `trail agent ask what needs attention` | Route a plain-language question to the inbox home view |
| `trail agent ask what changed` | Route a plain-language question to the right task view |
| `trail agent ask show actions` | Route a plain-language question to the action palette |
| `trail agent ask last prompt` | Route a plain-language question to the latest prompt turn |
| `trail agent ask what changed in the last prompt` | Route a plain-language question to the newest prompt delta |
| `trail agent ask what changed in README.md in the last prompt` | Route a plain-language question to a file-scoped prompt delta |
| `trail agent ask show transcript` | Route a plain-language question to the task transcript |
| `trail agent ask what should I put in the PR` | Route to a read-only pull request draft |
| `trail agent ask give me a summary to share` | Route to the copyable task receipt |
| `trail agent ask handoff this to another agent` | Route to the copyable handoff packet |
| `trail agent ask what commit message should I use` | Route to apply readiness and the generated commit message |
| `trail agent ask which task first` | Route to overlap checks and safe apply order |
| `trail agent` | Show the current task dashboard, or grouped inbox when there are multiple tasks |
| `trail agent guide` | Show the shortest state-aware workflow for setup, review, apply, or recovery |
| `trail agent help-me` | Friendly alias for the agent guide |
| `trail agent home` | Alias for the agent inbox home view |
| `trail agent inbox` | Group all agent tasks by attention state, new changes, and review-first file |
| `trail agent board` | Group all agent tasks as a low-noise board for multiple agents or tasks |
| `trail agent tasks` | Friendly alias for the multi-agent board |
| `trail agent stack` | Show shared files and a safe apply order across agent tasks |
| `trail agent next` | Show the one next useful action for the latest agent task |
| `trail agent todo` | Friendly alias for the one next useful action |
| `trail agent status` | Show the latest agent task, risk, and next useful action |
| `trail agent dashboard latest` | Show one compact task board with next action, focus, validation, and apply readiness |
| `trail agent review-data latest` | Show one structured review packet for editor panels and integrations |
| `trail agent action` | Show runnable review actions for the latest task |
| `trail agent action inspect_focus_file` | Run one published review action for the latest task |
| `trail agent review-flow latest` | Walk review, validation, and finish as one guided checklist |
| `trail agent walkthrough latest` | Friendly alias for the guided review checklist |
| `trail agent brief latest` | Show a compact task brief with risk, next action, changes, and tools |
| `trail agent summary latest` | Show one post-run cockpit with readiness, risk, receipt, PR draft, and next command |
| `trail agent validate latest` | Show latest gates and suggested validation commands without running anything |
| `trail agent test-plan latest` | Show a prioritized test/eval checklist with exact commands |
| `trail agent receipt latest` | Print a copyable post-run receipt with validation, changes, risk, and next command |
| `trail agent handoff latest` | Print a copyable handoff packet for another human or agent |
| `trail agent share latest` | Friendly alias for the handoff packet |
| `trail agent pr latest` | Print a pull request draft title and body without creating a remote PR |
| `trail agent report latest --markdown` | Print the deeper review bundle behind a task |
| `trail agent story latest` | Explain what happened in plain language |
| `trail agent tools latest` | Show tool calls, available commands, and the turns/checkpoints around them |
| `trail agent impact latest` | Show changed areas, blast radius, and recommended review/test checks |
| `trail agent review-map latest` | Show a file-by-file review checklist grouped by changed area |
| `trail agent risk latest` | Show apply risk, reasons, and concrete mitigations |
| `trail agent confidence latest` | Show one go/no-go verdict across review, validation, risk, and apply preflight |
| `trail agent go-no-go latest` | Friendly alias for the confidence verdict |
| `trail agent ready latest` | Check apply readiness without mutating Git |
| `trail agent can-land latest` | Friendly alias for safe apply readiness |
| `trail agent diagnose latest` | Explain likely issues and safe recovery options before undo/rewind |
| `trail agent recover latest` | Friendly alias for recovery diagnosis |
| `trail agent compare <TASK_A> <TASK_B>` | Compare two agent tasks, shared files, risk, and next action |
| `trail agent test latest -- cargo test` | Run and record a test gate in the task workdir |
| `trail agent eval latest -- <command>` | Run and record an eval gate in the task workdir |
| `trail agent workdir latest` | Print the task workdir and a copyable `cd` command |
| `trail agent view latest` | Inspect transcript, tools, changed paths, and checkpoint |
| `trail agent changes latest` | Show high-level change cards plus turn/checkpoint details |
| `trail agent delta latest` | Show the newest completed turn or operation delta |
| `trail agent last latest` | Friendly alias for the newest completed turn or operation delta |
| `trail agent new latest` | Show changes since the task was last marked reviewed |
| `trail agent what-changed latest` | Friendly alias for changes since the last reviewed checkpoint |
| `trail agent mark-reviewed latest` | Mark the current task checkpoint as reviewed |
| `trail agent mark-file-reviewed latest README.md` | Mark one changed file reviewed in the review map |
| `trail agent done latest` | Friendly alias for marking the current checkpoint reviewed |
| `trail agent archive latest` | Hide a finished or irrelevant task from default inbox/list/latest views |
| `trail agent close latest` | Friendly alias for archiving an agent task |
| `trail agent unarchive <TASK>` | Restore an archived task to the default agent inbox |
| `trail agent turn` | Inspect the latest completed turn with prompt, tools, checkpoint, and files |
| `trail agent turn-diff latest --patch` | Show the latest or selected turn diff without spelling out diff flags |
| `trail agent files latest` | Show changed files with the turns and commands behind each file |
| `trail agent changed-files latest` | Friendly alias for changed files with provenance |
| `trail agent inspect README.md` | Friendly alias for file-centered agent context |
| `trail agent checkpoints latest` | List friendly rewind targets and checkpoint ids |
| `trail agent rewind-points latest` | Friendly alias for checkpoint and rewind targets |
| `trail agent why latest README.md` | Explain which prompt, turn, tools, and checkpoint changed a file |
| `trail agent explain README.md` | Friendly alias for explaining why a file changed |
| `trail agent review-plan latest` | Show readiness, risk, review-priority files, and next commands |
| `trail agent review latest` | Short alias for the review-priority dashboard |
| `trail agent focus latest` | Inspect the next file to review with why, a materialized-task open command, and focused diff summary |
| `trail agent open latest` | Open the focused file in `$EDITOR` for a materialized task |
| `trail agent apply latest --dry-run` | Preview safe Git apply for an agent task |
| `trail agent apply latest` | Record, merge, export, and fast-forward with a task-title commit message |
| `trail agent land latest` | Friendly alias for applying an agent task safely |
| `trail agent finish latest` | Apply the task and hide it from the default inbox after success |
| `trail agent undo latest` | Undo the latest agent turn without copying checkpoint ids |
| `trail agent undo-last latest` | Friendly alias for undoing the latest agent turn |
| `trail lane spawn <name> --from <ref>` | Create an isolated lane branch |
| `trail agent start --provider codex --workdir-mode nfs-cow` | Run a macOS terminal agent in a loopback NFS copy-on-write workdir |
| `trail lane apply-patch <name> --patch <file>` | Apply a structured patch to a lane branch |
| `trail lane review <name>` | Produce a compact review packet for a lane branch |
| `trail lane readiness <name>` | Report blockers before merging a lane branch |
| `trail lane handoff <name>` | Produce a review and continuation packet for a lane |
| `trail merge-lane <name> --into <branch> --dry-run` | Preview merging a lane branch into a target branch |
| `trail merge-queue run` | Run queued lane merges with readiness and conflict checks |
| `trail daemon` | Start the loopback HTTP daemon for editor and automation integrations |
| `trail mcp` | Start the MCP stdio server for agent hosts |
| `trail acp list` | List built-in aliases and current official ACP registry agents |
| `trail acp install --agent claude-code` | Print an ACP relay command and editor snippet |
| `trail acp install --agent codex` | Print the Codex ACP relay command and editor snippet |
| `trail acp doctor --agent claude-code` | Check ACP provider and relay readiness |
| `trail acp sessions` | List captured ACP sessions |
| `trail transcript <lane-or-session>` | Read captured prompts, assistant messages, tools, and checkpoints |
| `trail doctor` | Run workspace and integration diagnostics |
| `trail backup create <output>` | Create a Trail workspace backup |
| `trail fsck` | Verify repository integrity |

## Core Local Workflows

Record the whole worktree:

```sh
trail record -m "describe the operation"
```

Record only selected paths:

```sh
trail record --paths README.md docs/ -m "record docs only"
```

Manage branches and merges:

```sh
trail branch
trail branch experiment --from main
trail checkout experiment --dry-run
trail merge experiment --into main --dry-run
```

Run safety and maintenance checks:

```sh
trail ignore check README.md
trail guardrails check --action shell.exec --summary "Run smoke tests"
trail doctor
trail fsck
```

Use `--json` or `--format json` on commands when a script, editor, or agent
needs machine-readable output.

## Agent Workflow

Use `trail agent` when you want Trail to hide lane names, ACP sessions, export
ranges, and Git handoff details. Each task gets a fresh lane by default.
Task lists and summaries show a human title first, derived from the prompt or
from `--name`; the stable task id is still shown when you need precision.
When a task has a materialized filesystem, summaries also show `Workdir`, the
exact directory where the agent edited files.

Configure an ACP editor once:

```sh
trail agent setup
```

`agent setup` defaults to Claude Code plus VS Code, and Codex is also available
with `trail agent setup --provider codex`. Use `--editor zed` or
`--editor generic` when you want another snippet. It prints the editor snippet
plus the next verification and review commands.
Paste the printed snippet into the editor's ACP custom-agent settings. After one
prompt, ask Trail what needs attention:

```sh
trail agent
```

Bare `agent` is the inbox home view. It shows the task queue, the attention
state, new files/lines since the last review, the first file to inspect, and the
one next command. Use `agent status` for only the latest task or
`agent todo latest` for a single primary command. Use `agent board` when several
editor or terminal agents are active and you want one low-noise board grouped by
needs-record, conflicted, blocked, needs-review, ready, running, applied, and
archived. Use `agent stack` when you need overlap checks and a safe apply order
across tasks. Then drill into a task only as needed:

`agent guide` is the first-run and "I forgot what to do" command. It explains
the current task state, prints one next command, shows a short setup/review/apply
or recovery workflow, and keeps the public mental model to agent task, changes,
apply, and recover.

`trail agent --help` intentionally shows the common path first. Specialist
inspection commands still exist, but the first screen points you toward
`agent guide`, `agent ask ...`, `agent action`, `agent changes`, and safe
`agent apply --dry-run` instead of making you choose from every low-level view.
If no task exists yet, daily-path commands such as `agent view latest`,
`agent changes latest`, and `agent apply latest --dry-run` return setup guidance
and first-run actions instead of a dead-end error.

```sh
trail agent
trail agent board
trail agent stack
trail agent guide
trail agent ask help me
trail agent ask what needs attention
trail agent ask what should I do next
trail agent ask what did the agent do
trail agent ask where is the workdir
trail agent ask where did the agent edit
trail agent ask which prompt changed README.md
trail agent ask last prompt
trail agent ask what changed in the last prompt
trail agent ask what changed in README.md in the last prompt
trail agent ask show transcript
trail agent ask show dashboard
trail agent ask show actions
trail agent ask what should I review
trail agent ask what should I review first
trail agent ask what file should I review first
trail agent ask what file should I open
trail agent ask where should I look first
trail agent ask open review
trail agent ask review this task
trail agent ask what tools were used
trail agent tools latest
trail agent ask what is the blast radius
trail agent impact latest
trail agent ask review map
trail agent review-map latest
trail agent ask what did the agent change
trail agent ask what files did it touch
trail agent ask can I merge
trail agent ask why can't I apply
trail agent ask what is blocking this task
trail agent ask why did it fail
trail agent ask what went wrong
trail agent ask any red flags
trail agent ask what should I worry about
trail agent ask which files are risky
trail agent ask what changed since I looked
trail agent ask what should I put in the PR
trail agent ask give me a summary to share
trail agent ask what commit message should I use
trail agent ask explain README.md
trail agent ask show the diff
trail agent ask show changes by file
trail agent ask show patch for README.md
trail agent ask show turn diff
trail agent story latest
trail agent summary latest
trail agent ask what tests should I run
trail agent ask is it tested
trail agent ask how should I test this
trail agent test-plan latest
trail agent validate latest
trail agent risk latest
trail agent confidence latest
trail agent ask final check, am I good?
trail agent can-land latest
trail agent recover latest
trail agent compare <TASK_A> <TASK_B>
trail agent stack
trail agent receipt latest
trail agent handoff latest
trail agent pr latest
trail agent report latest --markdown
trail agent test latest -- cargo test
trail agent brief latest
trail agent dashboard latest
trail agent review-flow latest
trail agent ask walk me through review
trail agent workdir latest
trail agent last latest
trail agent what-changed latest
trail agent mark-file-reviewed latest README.md
trail agent done latest
trail agent close latest
trail agent inbox --all
trail agent unarchive <TASK>
trail agent changes latest
trail agent changes latest --by-file
trail agent turn
trail agent turn-diff latest --patch
trail agent changed-files latest
trail agent rewind-points latest
trail agent explain README.md
trail agent turn-diff latest --file README.md --patch
trail agent review-plan latest
trail agent focus latest
trail agent open latest
trail agent view latest
trail agent land latest --dry-run
trail agent land latest
trail agent finish latest
```

If the latest prompt went sideways, undo by task language instead of copying a
checkpoint id:

```sh
trail agent rewind-points latest
trail agent undo-last latest
trail agent undo-last latest --turn 2
trail agent undo-last latest --prompt 'Add hook support'
```

For terminal-first work, create a fresh materialized lane and launch Claude Code
inside it:

```sh
trail agent start --provider claude-code --name docs-edit
trail agent
trail agent ask what should I do next
trail agent todo
trail agent last latest
trail agent what-changed latest
trail agent changes latest
trail agent changes latest --by-file
trail agent change latest 1
trail agent inspect README.md
trail agent timeline latest
trail agent turn
trail agent turn-diff latest --patch
trail agent validate latest
trail agent test latest -- cargo test
trail agent can-land latest
trail agent explain README.md
trail agent turn-diff latest --file README.md --patch
trail agent finish latest
trail agent continue latest
```

`agent land` is an alias for `agent apply`; use whichever verb feels more
natural. Both record dirty lane workdirs first, check that the current Git tree
matches Trail's internal apply base, merges the task into Trail, creates a Git
commit using the task title by default, and fast-forwards the current Git branch
only when safe. Use `-m` only when you want to override the generated commit
message. If the task has already been applied, `agent land` reports
`already_applied` and suggests `agent continue` so follow-up work starts from the
applied checkpoint in a fresh task instead of reusing old lane history.

Use `agent finish latest` when you want the low-maintenance path: it performs
the same safe apply flow as `agent land` and, after success, hides the finished
task from the default inbox. `agent ship` is a readable alias. Use `agent land`
when you want to keep the applied task visible for more inspection.

`agent summary latest` is the easiest post-run view for one task. It combines
readiness, risk, validation, Git preflight, receipt and PR pointers, and the one
next command. Use it when you are not sure whether to review, test, apply, or
draft a PR.

`agent dashboard latest` is the compact daily-use board. It shows the one next
action, review focus, open command, validation status, changed files, risk, and
apply readiness without exposing lane, turn, or checkpoint ids unless you ask
for JSON. Human output also points to `agent action <task>`, the small command
palette for review, validation, apply, and recovery actions.

`agent review-data latest` is the editor-panel packet. It returns file review
progress, focus file, review map, changes by file, confidence, validation, risk,
apply readiness, and typed actions for side-panel buttons in one structured
report. Actions include stable ids, safety classes, disabled reasons, optional
MCP tool names and MCP arguments, so UI buttons do not need to parse shell
commands. Run `agent action` to list the available actions, `agent action <id>`
to execute one for the latest task, `agent action <task> <id>` for a specific
task, or add `--print` to show the exact command without running it.
Before the first task exists, `agent action` shows runnable setup, doctor, and
terminal-start actions instead of failing; for example,
`trail agent action setup_vscode` prints the VS Code setup report.
Confirmation-required actions still need `--confirm`. `agent cockpit latest`
and `agent side-panel latest` are aliases.

`agent validate latest` is the read-only validation guide. It shows the latest
test/eval gates, whether more validation is needed, and suggested
`agent test`/`agent eval` commands. Risk, review, readiness, and summary views
point here before suggesting commands that execute validation. `agent tests
latest` is an alias.

`agent test-plan latest` is the actionable validation checklist. It turns
changed areas and existing gates into ranked test/eval steps with exact
commands, affected paths, and reasons. Use it when you are asking what to run;
use `agent validate latest` when you only want to know whether gates already
passed. `agent ask what tests should I run` and `agent ask how should I test
this` route here.

`agent receipt latest` is the copyable after-action note. It prints Markdown by
default with validation gates, changed files, turns, tools, risk, checkpoint,
and the next command; use `--json` when an editor panel needs the same data.

`agent handoff latest` is the copyable packet for another human or agent. It
prints Markdown by default with the current state, receiver next step, review
commands, validation, risks, changed files, turns, tools, and links to the
shorter receipt and deeper report. `agent share latest` is the friendly alias.

`agent pr latest` prints a pull request draft title and body from the same
recorded task state. It is read-only: it does not create a GitHub/GitLab PR or
touch Git. Use `--title-only` or `--body-only` for scripts.

`agent ask ...` is the easiest front door when you do not remember a command.
It uses deterministic routing, not model inference, and reuses the normal agent
reports. Examples: `agent ask what needs attention`,
`agent ask what should I do next`,
`agent ask what did the agent do`, `agent ask where is the workdir`,
`agent ask where did the agent edit`, `agent ask which prompt changed README.md`,
`agent ask last prompt`, `agent ask what changed in the last prompt`,
`agent ask what changed in README.md in the last prompt`,
`agent ask show transcript`,
`agent ask what should I review`,
`agent ask what should I review first`,
`agent ask what file should I review first`,
`agent ask what file should I open`,
`agent ask where should I look first`, `agent ask open review`,
`agent ask review this task`,
`agent ask what tools were used`, `agent ask what just changed`,
`agent ask what changed since I looked`, `agent ask changed files`,
`agent ask what is the blast radius`,
`agent ask review map`,
`agent ask what did the agent change`,
`agent ask what files did it touch`,
`agent ask what tests should I run`, `agent ask validation plan`,
`agent ask is it tested`, `agent ask how should I test this`, `agent ask can I merge`,
`agent ask is it safe to land`, `agent ask why can't I apply`,
`agent ask what is blocking this task`,
`agent ask why did it fail`, `agent ask what went wrong`,
`agent ask any red flags`, `agent ask what should I worry about`,
`agent ask which files are risky`,
`agent ask what should I put in the PR`,
`agent ask give me a summary to share`,
`agent ask handoff this to another agent`,
`agent ask what commit message should I use`,
`agent ask recover`, and `agent ask explain README.md`.
Patch and diff phrasing also works:
`agent ask show the diff`, `agent ask show last patch`,
`agent ask show changes by file`, `agent ask show turn diff`, and
`agent ask show patch for README.md`.

`agent todo latest` is the "what should I do now?" command. It is an alias for
`agent next latest` and returns one primary next action plus a few alternatives.

`agent can-land latest` is the "is it safe?" command. It is an alias for
`agent ready latest` and combines task readiness, risk, Git preflight, blockers,
warnings, and the next command without mutating Git.

`agent recover latest` is the recovery front door. It is an alias for
`agent diagnose latest`. It explains the likely issue, shows evidence, lists
friendly checkpoint targets, and prints safe
inspection/recovery commands before you run destructive undo or rewind actions.

`agent timeline latest` is the chronological view. It connects prompts,
assistant responses, tool summaries, checkpoints, changed files, and exact
follow-up commands so you can understand what happened without chasing turn ids
or operation ids manually. Use `--by-operation` when you need the lower-level
Trail operation timeline.

`agent last latest` starts with the newest completed turn. It is the fastest
way to answer "what just changed?" after an editor or terminal agent finishes a
prompt. It is an alias for `agent delta latest`. Add `--patch` for exact hunks
or `--file <PATH> --patch` to keep the view pinned to one open file.

`agent what-changed latest` starts from the last reviewed checkpoint. It is
the fastest way to answer "what changed since I looked?" without manually
comparing turns or operation ids. If no reviewed marker exists yet, it shows the
whole task as unreviewed. Run `agent done latest` after inspection to make the
current checkpoint the next baseline. It is an alias for `agent new latest`;
`agent mark-reviewed latest` remains the explicit reviewed-marker form.

`agent close latest` is the low-clutter cleanup command. It archives the task so
default `agent`, `agent inbox`, `agent board`, `agent list`, and `latest` ignore
it, but it does not delete the lane, transcript, checkpoints, or provenance. Use
`agent inbox --all`, `agent board --all`, or `agent list --all` to see archived tasks, and
`agent unarchive <TASK>` to restore one.

`agent changes latest` is the start-here review map for the whole task. It
prints one `Next` command, then ranks change cards by likely review importance.
Use `--by-file` when you want one card per changed file, or `--by-operation`
when you want raw Trail checkpoint operations. Each card includes ready
`Review`, `Focus`, `Why`, and `Diff` commands so you can open the card, inspect
the first file, explain provenance, or jump straight to a patch without copying
turn ids or checkpoint ids.

`agent change latest 1` expands one change card into a focused change set:
files, provenance, tools, ready commands, and optional focused patches with
`--patch`. You can select by rank or key, for example `agent change latest docs`.

`agent impact latest` is the blast-radius view. It groups changed files into
areas such as dependencies, build config, public API, integrations, CLI/UI,
tests, docs, and core code, then combines that with risk, validation status, and
recommended review/test commands. `agent ask what is the blast radius` routes
here.

`agent review-map latest` is the lowest-burden code review checklist. It groups
files by changed area, ranks each file with existing review priority signals,
and prints ready `focus`, `why`, `patch`, and editor-open commands. Use it when
you want to review every file without stitching together impact, changes,
timeline, and focus views by hand. `agent ask review map` routes here.

`agent mark-file-reviewed latest README.md` marks one changed file reviewed at
the current task checkpoint. The next `agent review-map latest` shows that file
as reviewed unless the agent changes it again. Use `agent mark-reviewed latest`
only when the whole checkpoint has been reviewed.

`agent turn-diff latest` is the shortcut for prompt-sized code review. It shows
the newest completed turn diff without requiring `agent diff --last-turn`.
Pass `--turn 3` for a specific prompt, `--file <PATH>` for the file open in your
editor, and `--patch` when you want exact hunks.

`agent changed-files latest` is the code-review-shaped view. It is an alias for
`agent files latest` and lists every changed file with the prompt, turn,
operation, and ready commands behind that file.

`agent inspect README.md` starts from the file you are looking at. It shows
whether the agent changed that path, which change set contains it, which turn or
operation touched it, and the next command. It is an alias for
`agent file README.md`. Add `--patch` for the focused diff.

`agent explain README.md` answers why one file changed: the captured prompt,
turn, tools, checkpoint, and focused diff command. It is an alias for
`agent why README.md`.

`agent rewind-points latest` lists friendly recovery targets before you undo or
rewind. It is an alias for `agent checkpoints latest`.

`agent undo-last latest` is the everyday recovery command. It is an alias for
`agent undo latest` and rewinds to the state before the latest completed turn by
default; pass `--turn` or `--prompt` when you need a specific prompt-sized undo.

## Lane Workflow

Lanes work on isolated Trail refs instead of immediately changing `main`.
External coding agents such as Claude Code or Codex can work inside these lanes,
but `trail lane` itself is the lower-level workflow. Prefer `trail agent` for
day-to-day agent tasks.

```sh
trail lane spawn doc-bot --from main --materialize=true
trail lane status doc-bot
trail lane workdir doc-bot
```

The printed workdir path is where you edit files or run an external coding
agent. Trail keeps that work isolated until you record it into the lane.

Apply a structured patch directly to the lane branch:

```sh
trail lane apply-patch doc-bot --patch patch.json
```

Review and gate the work:

```sh
trail lane diff doc-bot --patch --show-line-ids
trail lane review doc-bot
trail lane contribution doc-bot
trail lane readiness doc-bot
trail lane handoff doc-bot
```

When a tool needs a filesystem checkout, create or sync a materialized workdir:

```sh
trail lane spawn doc-bot --from main --materialize=true
LANE_DIR="$(trail lane workdir doc-bot)"
cd "$LANE_DIR"
# Edit files or run external tooling here.

cd /path/to/project
trail lane record doc-bot -m "record workdir changes"
trail lane sync-workdir doc-bot
```

Merge only after review and readiness checks:

```sh
trail merge-lane doc-bot --into main --dry-run
trail merge-queue add doc-bot --into main
trail merge-queue run
```

If a merge opens conflicts, inspect and resolve them explicitly:

```sh
trail conflicts list
trail conflicts show <conflict-set-id>
```

## Integrations

Start the local HTTP daemon for editor and automation integrations:

```sh
trail daemon
```

The daemon defaults to `127.0.0.1:8765`. `GET /v1/health` is unauthenticated;
other routes require bearer auth or `x-trail-token` unless the daemon is started
with `--no-auth`. When no token is supplied, Trail creates or reads
`.trail/daemon.token`, and daemon discovery uses `.trail/daemon.json`.

Export the OpenAPI contract:

```sh
trail api openapi --output trail.openapi.json
```

Start the MCP stdio server for agent hosts:

```sh
trail mcp
```

MCP exposes tools, resources, resource templates, prompts, completions, and risk
annotations for host permission UX. The MCP and HTTP layers call the same
`Trail` methods and return the same report shapes used by CLI JSON output.
Editor hosts can call `trail.agent_ask` with plain-language questions such as
`what needs attention`, `what changed since I looked`,
`show editor panel data`,
`what did the agent do`, `where is the workdir`, `where did the agent edit`,
`which prompt changed README.md`, `what should I review`,
`what should I review first`, `what file should I review first`,
`what file should I open`, `where should I look first`, `open review`,
`review this task`, `last prompt`,
`what changed in the last prompt`,
`what changed in README.md in the last prompt`, `show transcript`,
`what tools were used`, `can I merge`, `why can't I apply`,
`what is blocking this task`, `why did it fail`, `what went wrong`,
`what did the agent change`, `what files did it touch`,
`which files are risky`,
`what should I put in the PR`, `give me a summary to share`,
`what commit message should I use`,
or `explain README.md`; it routes to the
matching read-only agent report and includes the routed tool name.
For side panels, call `trail.agent_review_data` or read
`trail://workspace/agent-tasks/latest/review-data`.
Agent-focused MCP prompts include `trail.review_agent`,
`trail.recover_agent`, and `trail.apply_agent` for editor hosts that want a
guided workflow instead of making users choose individual tools.

Rust callers can depend on the `trail` crate and use exported types such as
`Trail`, `InitImportMode`, IDs, `PatchDocument`, and report structs through the
library prelude.

## Repository Layout

```text
trail/   CLI, library API, HTTP daemon, MCP server, models, storage
prolly/   Ordered map storage used by roots and text indexes
docs/            User, operator, integration, reference, and design docs
scripts/         Local helper and benchmark scripts
```

## Documentation

Start with the docs home:

- [Trail documentation](docs/README.md)
- [Roadmap](ROADMAP.md)
- [Install and build](docs/getting-started/install-and-build.md)
- [Initialize a workspace](docs/getting-started/initialize-a-workspace.md)
- [First record and provenance query](docs/getting-started/first-record-and-query.md)
- [First lane workflow](docs/getting-started/first-lane-workflow.md)

Read by topic:

- [Core concepts](docs/concepts/operation-database.md)
- [Guides](docs/guides/record-worktree-changes.md)
- [Harden agent workflows](docs/guides/hardening-agent-workflows.md)
- [Use cases](docs/use-cases/local-code-history.md)
- [Lane workflows](docs/lanes/overview.md)
- [Lane work model](docs/lanes/work-model.md)
- [Integrations](docs/integrations/overview.md)
- [CLI reference](docs/reference/cli/global-options-and-env.md)
- [HTTP API reference](docs/reference/http-api.md)
- [MCP tools reference](docs/reference/mcp-tools.md)
- [Design notes](docs/design/architecture.md)
- [Code fact map](docs/_meta/code-fact-map.md)

## Development

Run the main test suite:

```sh
cargo test -p trail
```

Run the storage crate tests when changing prolly-tree behavior:

```sh
cargo test -p prolly
```

Useful validation commands while editing docs or examples:

```sh
cargo run -p trail -- --help
cargo run -p trail -- agent --help
cargo run -p trail -- api openapi --output /tmp/trail.openapi.json
```

## License

Trail is distributed under the terms in [LICENSE](LICENSE). The checked-in
license file contains the MIT License; the Rust workspace metadata currently
declares `MIT OR Apache-2.0`.

## Current Boundaries

Trail is local-first. It does not require a hosted service, and its HTTP daemon
is a loopback integration surface rather than a cloud API.

Trail complements Git rather than replacing it. Use Git for shared commit
history; use Trail for local operation history, provenance, agent coordination,
and review signals before or around commits.

Safety features are conservative but not magic. Keep secrets ignored, review
patches and diffs, treat guardrail decisions as preflight signals, and use human
approvals for sensitive agent actions.

# CrabDB

CrabDB is a local-first operation database for code and text worktrees. It records
the meaningful work that happens between Git commits: local edits, recorded
operations, branch movement, lane patches, review handoffs, merges, and
line-level provenance.

Git remains the source of shared repository history. CrabDB adds a local layer for
questions Git does not answer well by itself:

- What operation introduced this current line?
- What changed on this branch before it became a Git commit?
- What happened in a lane, which paths changed, and is it ready to merge?
- Which changes are blocked by conflicts, pending approvals, dirty workdirs, or
  missing test/eval gates?
- Can an editor, agent host, or local service query the same state through CLI,
  HTTP, MCP, or Rust?

CrabDB stores its local state under `.crabdb/`, uses `.crabignore` and Git ignore
files to avoid accidental private/generated captures, and can run fully through
the CLI without a background service. The HTTP daemon and MCP stdio server are
opt-in integration surfaces.

## CrabDB vs Git

CrabDB is not a Git replacement. It sits next to Git as a local operation,
provenance, and lane-coordination layer.

```text
  human edits, editor saves, lane turns, tool calls, patches
                         |
                         v
  +---------------------------------------------------------+
  | CrabDB                                                  |
  | record operations, preserve line identity, isolate      |
  | lane branches, run guardrails, check readiness,        |
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

Use Git for durable shared source-control history. Use CrabDB for the messy,
high-frequency, local work that happens before a commit is ready.

| Need | Git | CrabDB |
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
- CrabDB is the local operational memory and coordination layer.
- Git answers "what committed snapshot did the project accept?"
- CrabDB answers "what happened locally, why did it happen, which lane contains it,
  is it safe to accept, and what still blocks merge?"

## Why This Matters for AI Agents

AI coding agents produce more than final diffs. They create attempts,
intermediate patches, tool calls, test runs, review notes, approvals, and
handoffs. Treating all of that as either "unstaged changes" or "a Git commit"
loses important context.

CrabDB gives lane workflows a native coordination layer:

- Each external agent can work inside an isolated CrabDB lane without
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

In short: Git is where accepted code history lives. CrabDB is where local and
lane work becomes understandable, reviewable, and mergeable before it becomes
Git history.

## Who It Is For

CrabDB is useful for several overlapping audiences.

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
`CrabDb` API and exported model/report types.

## What CrabDB Provides

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

CrabDB is organized around one core library object, `CrabDb`. The CLI, HTTP
daemon, MCP server, tests, and Rust callers all route through that same core so
they can share behavior and report types.

![CrabDB architecture diagram](diagram/crabdb-architecture/crabdb-architecture@2x.png)

The same architecture in text form:

```text
                         entry points
  +-----------+   +---------------+   +--------------+   +-------------+
  | CLI       |   | HTTP daemon   |   | MCP stdio    |   | Rust API    |
  | crabdb    |   | /v1 JSON API  |   | tools/docs   |   | CrabDb      |
  +-----+-----+   +-------+-------+   +------+-------+   +------+------+
        |                 |                  |                  |
        +-----------------+------------------+------------------+
                                     |
                                     v
  +-------------------------------------------------------------------+
  | CrabDb core                                                       |
  | workspace policy, refs, objects, records, lanes, merges, reports |
  +-----------+------------------+--------------------+---------------+
              |                  |                    |
              v                  v                    v
  +-------------------+  +------------------+  +----------------------+
  | SQLite            |  | Prolly maps      |  | .crabdb sidecars     |
  | objects, refs,    |  | path maps, file  |  | config, HEAD, refs,  |
  | indexes, queues,  |  | indexes, text    |  | daemon files,        |
  | lane state        |  | and line order   |  | workdir manifests    |
  +-------------------+  +------------------+  +----------------------+
```

### Command Flow

Most commands run directly against the local database. Selected hot paths can
use the daemon when a daemon URL is supplied or when `.crabdb/daemon.json` is
auto-discovered.

```text
  user or host
      |
      v
  crabdb CLI
      |
      v
  parse command + build RuntimeContext
      |
      +--> daemon-capable command and daemon available?
      |        |
      |        +-- yes --> HTTP daemon --> CrabDb core --> report JSON
      |
      +-- no or fallback --> local CrabDb core --> report struct
                                      |
                                      v
                         human output, JSON, or NDJSON
```

The important design point is that daemon-backed and local paths return the same
report shapes. A script can use CLI JSON, the HTTP API, MCP tools, or Rust types
without learning separate data models for each surface.

### Durable History Model

CrabDB records operations as durable history. Refs point to operation/root pairs;
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
  | lanes             |        | lane_branches       |
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

CrabDB's safety checks sit between user, automation, or agent requests and workspace mutation.

```text
  request
     |
     v
  normalize paths
     |
     v
  block .crabdb/.git/private hardcoded paths
     |
     v
  apply .crabignore and .gitignore policy
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

These checks are deliberately local and explainable. They protect CrabDB/Git
internals, ignored/private paths, risky lane actions, dirty materialized
workdirs, stale refs, and conflicted merges before changes are accepted.

## Quick Start

CrabDB is a Rust workspace. The repository declares Rust 1.81 in `Cargo.toml`.
Build from source with the Makefile:

```sh
# Build the debug binary at target/debug/crabdb.
make build

# Print CLI help from the local debug binary.
target/debug/crabdb --help
```

Install a local optimized binary with the Makefile. By default this installs to
`$HOME/.cargo/bin/crabdb`:

```sh
# Build the release binary and install crabdb locally.
make install

# Verify the installed crabdb command is on your PATH.
crabdb --help
```

For a project-local install directory, override `PREFIX`:

```sh
# Install to ./.local/bin/crabdb instead of $HOME/.cargo/bin/crabdb.
make install PREFIX="$PWD/.local"
```

The equivalent direct Cargo build command is:

```sh
# Build the debug binary without using the Makefile.
cargo build -p crabdb

# Print CLI help from the Cargo-built debug binary.
target/debug/crabdb --help
```

Initialize a workspace from the current working tree:

```sh
# Import visible working tree files into .crabdb/.
crabdb init --working-tree
```

Inspect and record an edit:

```sh
# Show whether the current worktree differs from CrabDB's recorded root.
crabdb status

# Record the current edit as a named local operation.
crabdb record -m "record current edit"

# List recent recorded operations.
crabdb timeline --limit 10

# Inspect one recorded operation from the timeline output.
crabdb show <change-id>
```

Ask provenance questions:

```sh
# Explain what operation introduced the current README.md line 2.
crabdb why README.md:2

# Show recorded history for README.md.
crabdb history README.md

# Show the current unrecorded worktree diff as a patch.
crabdb diff --dirty --patch
```

Start a lane for task work:

```sh
# Create an isolated lane branch with its own materialized workdir.
crabdb lane spawn docs-lane --from main --materialize=true

# Print the path to that workdir, then edit there or point a coding agent there.
crabdb lane workdir docs-lane

# Record, review, and check readiness before merge.
crabdb lane record docs-lane -m "record docs update"
crabdb lane diff docs-lane --patch
crabdb lane readiness docs-lane
crabdb merge-lane docs-lane --into main --dry-run
```

Example CLI output from a tiny workspace looks like this. IDs, object hashes,
workspace IDs, and actor names will differ on your machine.

## Common ID prefixes:

| Prefix | Meaning | Example use |
| --- | --- | --- |
| `wk_` | Workspace ID derived when `.crabdb/` is initialized | Identifies one local CrabDB workspace |
| `ch_` | Change/operation ID allocated when CrabDB records an operation | Appears as `Head`, `Initial operation`, timeline entries, and `show` selectors |
| `obj_` | Content-addressed object ID | Identifies stored roots, operations, text objects, blobs, and other durable objects |
| `msg_` | Message ID | Used for durable operation, agent, or review messages |
| `anc_` | Anchor ID | Used for durable labels tied to file and line identity |
| `ch_...:<n>` | Stable file or line identity with an origin change and local sequence | Appears in `why` output as a `Line ID` |

## Example output

1. Initialize CrabDB from the visible files in the current working tree. The
   output shows the workspace ID, active branch, initial operation ID, and import
   summary.

```text
$ crabdb init --working-tree
Initialized CrabDB workspace
Workspace: wk_24ec99f68d1db8716f4df8a87580e3da
Branch: main
Initial operation: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Imported: 1 files (1 text, 0 opaque, 0 binary)
```

2. Check the recorded branch and worktree state. Immediately after
   initialization, the worktree is clean.

```text
$ crabdb status
Branch: main
Head: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Root: obj_46b1a72c6ff5e66a7b3026113243681493e79c2e659b6ef9658a2db57fdac431
Worktree: clean
```

3. After editing `README.md`, run `status` again. CrabDB reports the worktree as
   dirty and lists the modified path.

```text
$ crabdb status
Branch: main
Head: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Root: obj_46b1a72c6ff5e66a7b3026113243681493e79c2e659b6ef9658a2db57fdac431
Worktree: dirty
  Modified README.md
```

4. Record the edit as a named local operation. The output returns the new
   operation ID and the changed path summary.

```text
$ crabdb record -m "record current edit"
Recorded ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa
  Modified README.md
```

5. Read the recent operation timeline. The newest record appears first, followed
   by the initial import operation.

```text
$ crabdb timeline --limit 10
ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa ManualRecord main record current edit
ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6 GitImport main Initialize CrabDB workspace
```

6. Inspect one operation from the timeline. `show` expands the operation kind,
   actor, message, parent, before/after roots, and path-level summary.

```text
$ crabdb show ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa
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
$ crabdb why README.md:2
README.md:2 First recorded line
Line ID: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6:2
Introduced by: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
Last content change: ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6
```

8. Show file history. `history` lists operations that affected the selected
   file.

```text
$ crabdb history README.md
README.md
ch_5a44178a04acec35b4c27590303d665d462a229aa9bf627bb24e2c0f685fdcd6 Added README.md
ch_3d5a38ae49a7cd4b6873f003c97863f30ebc3efa61749b463c222d5d34809bfa Modified README.md
```

9. After making another edit without recording it, inspect the dirty diff. The
   patch shows what is currently in the worktree but not yet recorded by CrabDB.

```text
$ crabdb diff --dirty --patch
Diff main..dirty
  Modified README.md (+1 -0)
diff --crabdb a/README.md b/README.md
--- a/README.md
+++ b/README.md
 CrabDB sample
 First recorded line
 Second recorded line
+Unrecorded working tree line
```

Initialization creates `.crabdb/` state and a `.crabignore` file when needed.
Default ignore patterns protect CrabDB/Git internals, environment files, private
key/certificate files, dependency folders, build output, and coverage output.

Later examples use `crabdb` for readability. If the binary is not on your PATH,
install it with `make install` or replace `crabdb` with `target/debug/crabdb`.

## Common CLI Reference

| Command | Description |
| --- | --- |
| `crabdb init --working-tree` | Initialize `.crabdb/` from visible working tree files |
| `crabdb status` | Show branch head, root object, cleanliness, and changed paths |
| `crabdb record -m "<message>"` | Record current worktree changes as a named local operation |
| `crabdb record --paths <path>... -m "<message>"` | Record only selected paths |
| `crabdb watch --once` | Watch for changes and record once after debounce |
| `crabdb timeline --limit <n>` | List recent operations, newest first |
| `crabdb show <selector>` | Inspect an operation, message, ref, or object |
| `crabdb diff --dirty --patch` | Show unrecorded worktree changes as a patch |
| `crabdb why <path:line>` | Explain which operation introduced a current line |
| `crabdb history <path>` | Show recorded history for a file or selector |
| `crabdb code-from <selector>` | Find changed paths connected to a message, session, or agent |
| `crabdb branch` | List local CrabDB branches |
| `crabdb branch <name> --from <ref>` | Create a branch from another ref |
| `crabdb checkout <branch> --dry-run` | Preview checkout effects before changing the worktree |
| `crabdb merge <branch> --into <target> --dry-run` | Preview a branch merge and possible conflicts |
| `crabdb ignore check <path>` | Check whether ignore policy records or skips a path |
| `crabdb guardrails check --action <action>` | Preflight a risky action against workspace policy |
| `crabdb lane spawn <name> --from <ref>` | Create an isolated lane branch |
| `crabdb lane apply-patch <name> --patch <file>` | Apply a structured patch to a lane branch |
| `crabdb lane review <name>` | Produce a compact review packet for a lane branch |
| `crabdb lane readiness <name>` | Report blockers before merging a lane branch |
| `crabdb lane handoff <name>` | Produce a review and continuation packet for a lane |
| `crabdb merge-lane <name> --into <branch> --dry-run` | Preview merging a lane branch into a target branch |
| `crabdb merge-queue run` | Run queued lane merges with readiness and conflict checks |
| `crabdb daemon` | Start the loopback HTTP daemon for editor and automation integrations |
| `crabdb mcp` | Start the MCP stdio server for agent hosts |
| `crabdb doctor` | Run workspace and integration diagnostics |
| `crabdb backup create <output>` | Create a CrabDB workspace backup |
| `crabdb fsck` | Verify repository integrity |

## Core Local Workflows

Record the whole worktree:

```sh
crabdb record -m "describe the operation"
```

Record only selected paths:

```sh
crabdb record --paths README.md docs/ -m "record docs only"
```

Manage branches and merges:

```sh
crabdb branch
crabdb branch experiment --from main
crabdb checkout experiment --dry-run
crabdb merge experiment --into main --dry-run
```

Run safety and maintenance checks:

```sh
crabdb ignore check README.md
crabdb guardrails check --action shell.exec --summary "Run smoke tests"
crabdb doctor
crabdb fsck
```

Use `--json` or `--format json` on commands when a script, editor, or agent
needs machine-readable output.

## Lane Workflow

Lanes work on isolated CrabDB refs instead of immediately changing `main`.
External coding agents such as Claude Code or Codex can work inside these lanes,
but `crabdb lane` itself does not launch an AI agent.

```sh
crabdb lane spawn doc-bot --from main --materialize=true
crabdb lane status doc-bot
crabdb lane workdir doc-bot
```

The printed workdir path is where you edit files or run an external coding
agent. CrabDB keeps that work isolated until you record it into the lane.

Apply a structured patch directly to the lane branch:

```sh
crabdb lane apply-patch doc-bot --patch patch.json
```

Review and gate the work:

```sh
crabdb lane diff doc-bot --patch --show-line-ids
crabdb lane review doc-bot
crabdb lane contribution doc-bot
crabdb lane readiness doc-bot
crabdb lane handoff doc-bot
```

When a tool needs a filesystem checkout, create or sync a materialized workdir:

```sh
crabdb lane spawn doc-bot --from main --materialize=true
LANE_DIR="$(crabdb lane workdir doc-bot)"
cd "$LANE_DIR"
# Edit files or run external tooling here.

cd /path/to/project
crabdb lane record doc-bot -m "record workdir changes"
crabdb lane sync-workdir doc-bot
```

Merge only after review and readiness checks:

```sh
crabdb merge-lane doc-bot --into main --dry-run
crabdb merge-queue add doc-bot --into main
crabdb merge-queue run
```

If a merge opens conflicts, inspect and resolve them explicitly:

```sh
crabdb conflicts list
crabdb conflicts show <conflict-set-id>
```

## Integrations

Start the local HTTP daemon for editor and automation integrations:

```sh
crabdb daemon
```

The daemon defaults to `127.0.0.1:8765`. `GET /v1/health` is unauthenticated;
other routes require bearer auth or `x-crabdb-token` unless the daemon is started
with `--no-auth`. When no token is supplied, CrabDB creates or reads
`.crabdb/daemon.token`, and daemon discovery uses `.crabdb/daemon.json`.

Export the OpenAPI contract:

```sh
crabdb api openapi --output crabdb.openapi.json
```

Start the MCP stdio server for agent hosts:

```sh
crabdb mcp
```

MCP exposes tools, resources, resource templates, prompts, completions, and risk
annotations for host permission UX. The MCP and HTTP layers call the same
`CrabDb` methods and return the same report shapes used by CLI JSON output.

Rust callers can depend on the `crabdb` crate and use exported types such as
`CrabDb`, `InitImportMode`, IDs, `PatchDocument`, and report structs through the
library prelude.

## Repository Layout

```text
crates/crabdb/   CLI, library API, HTTP daemon, MCP server, models, storage
crates/prolly/   Ordered map storage used by roots and text indexes
docs/            User, operator, integration, reference, and design docs
scripts/         Local helper and benchmark scripts
```

## Documentation

Start with the docs home:

- [CrabDB documentation](docs/README.md)
- [Roadmap](ROADMAP.md)
- [Install and build](docs/getting-started/install-and-build.md)
- [Initialize a workspace](docs/getting-started/initialize-a-workspace.md)
- [First record and provenance query](docs/getting-started/first-record-and-query.md)
- [First lane workflow](docs/getting-started/first-lane-workflow.md)

Read by topic:

- [Core concepts](docs/concepts/operation-database.md)
- [Guides](docs/guides/record-worktree-changes.md)
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
cargo test -p crabdb
```

Run the storage crate tests when changing prolly-tree behavior:

```sh
cargo test -p prolly
```

Useful validation commands while editing docs or examples:

```sh
cargo run -p crabdb -- --help
cargo run -p crabdb -- agent --help
cargo run -p crabdb -- api openapi --output /tmp/crabdb.openapi.json
```

## License

CrabDB is distributed under the terms in [LICENSE](LICENSE). The checked-in
license file contains the MIT License; the Rust workspace metadata currently
declares `MIT OR Apache-2.0`.

## Current Boundaries

CrabDB is local-first. It does not require a hosted service, and its HTTP daemon
is a loopback integration surface rather than a cloud API.

CrabDB complements Git rather than replacing it. Use Git for shared commit
history; use CrabDB for local operation history, provenance, agent coordination,
and review signals before or around commits.

Safety features are conservative but not magic. Keep secrets ignored, review
patches and diffs, treat guardrail decisions as preflight signals, and use human
approvals for sensitive agent actions.

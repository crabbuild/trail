# CrabDB

CrabDB is a local-first operation database for code and text worktrees. It records
the meaningful work that happens between Git commits: local edits, recorded
operations, branch movement, agent patches, review handoffs, merges, and
line-level provenance.

Git remains the source of shared repository history. CrabDB adds a local layer for
questions Git does not answer well by itself:

- What operation introduced this current line?
- What changed on this branch before it became a Git commit?
- What did an agent do, which paths did it touch, and is it ready to merge?
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
provenance, and agent-coordination layer.

```text
  human edits, editor saves, agent turns, tool calls, patches
                         |
                         v
  +---------------------------------------------------------+
  | CrabDB                                                  |
  | record operations, preserve line identity, isolate      |
  | agent branches, run guardrails, check readiness,        |
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
| Agent isolation | Branches and worktrees, but no agent activity model | `refs/agents/<name>`, sessions, turns, traces, patches, gates, handoffs |
| Safety before mutation | Hooks and review conventions | Ignore policy, guardrails, approvals, leases, readiness blockers |
| Machine interfaces | Git CLI and plumbing | CLI JSON, HTTP/OpenAPI, MCP tools/resources/prompts, Rust reports |
| Merge readiness | Merge attempt plus conflicts | Readiness reports before merge: conflicts, dirty workdirs, approvals, gates |

The practical position is:

- Git is the publication and synchronization layer.
- CrabDB is the local operational memory and coordination layer.
- Git answers "what committed snapshot did the project accept?"
- CrabDB answers "what happened locally, why did it happen, which agent did it,
  is it safe to accept, and what still blocks merge?"

## Why This Matters for AI Agents

AI coding agents produce more than final diffs. They create attempts,
intermediate patches, tool calls, test runs, review notes, approvals, and
handoffs. Treating all of that as either "unstaged changes" or "a Git commit"
loses important context.

CrabDB gives agent workflows a native coordination layer:

- Each agent can work on an isolated CrabDB ref without immediately touching
  `main`.
- Structured patches can target stable file and line identity instead of fragile
  line numbers.
- Sessions, turns, messages, events, and trace spans preserve what the agent did
  and why.
- Guardrail checks and human approvals make risky actions explicit before they
  mutate the workspace or external systems.
- Test/eval gates, dirty-workdir checks, open conflicts, and pending approvals
  roll up into one readiness signal.
- Handoff and contribution reports give humans or other agents enough context to
  continue, review, or reject the work.
- Merge queues serialize accepted agent work so multiple agents do not silently
  overwrite each other.

In short: Git is where accepted code history lives. CrabDB is where local and
agent work becomes understandable, reviewable, and mergeable before it becomes
Git history.

## Who It Is For

CrabDB is useful for several overlapping audiences.

**Developers working locally:** record useful worktree operations before they
become commits, inspect branch history, ask why a line exists, and safely
checkout or merge local refs.

**Reviewers and maintainers:** inspect provenance, changed paths, operation
timelines, conflict sets, anchors, and diagnostics before accepting work.

**Coding-agent operators:** give each agent an isolated branch, durable sessions
and turns, structured patches, trace events/spans, guardrail checks, human
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
- Agent branches under `refs/agents/<name>` with optional materialized or sparse
  workdirs.
- Durable agent sessions, turns, messages, events, trace spans, paused run
  checkpoints, approvals, tests, evals, readiness, contribution, and handoff
  reports.
- Direct agent merges and merge queues with readiness checks, conflict sets, and
  manual conflict resolution.
- Git import/export mappings, backup/restore, fsck, index rebuild, garbage
  collection, and doctor diagnostics.
- CLI, HTTP daemon, OpenAPI, MCP, and Rust library integration surfaces backed by
  the same core implementation.

## Architecture at a Glance

CrabDB is organized around one core library object, `CrabDb`. The CLI, HTTP
daemon, MCP server, tests, and Rust callers all route through that same core so
they can share behavior and report types.

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
  | workspace policy, refs, objects, records, agents, merges, reports |
  +-----------+------------------+--------------------+---------------+
              |                  |                    |
              v                  v                    v
  +-------------------+  +------------------+  +----------------------+
  | SQLite            |  | Prolly maps      |  | .crabdb sidecars     |
  | objects, refs,    |  | path maps, file  |  | config, HEAD, refs,  |
  | indexes, queues,  |  | indexes, text    |  | daemon files,        |
  | agent state       |  | and line order   |  | workdir manifests    |
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
  refs/branches/main              refs/agents/doc-bot
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

### Agent Coordination Model

Agents are modeled as isolated branches plus durable activity records. A normal
agent workflow writes to `refs/agents/<name>`, reviews readiness, then merges
into a target branch only after checks pass.

```text
  +-------------------+        +----------------------+
  | agents            |        | agent_branches       |
  | identity, model,  |------->| ref, base/head root, |
  | provider, metadata|        | session, workdir,    |
  +-------------------+        | status               |
                               +----------+-----------+
                                          |
                                          v
                               refs/agents/<name>
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

Materialized workdirs are optional. Structured patches can update an agent branch
without checking out a full filesystem tree; materialized or sparse workdirs are
available when tools need real files and command execution.

### Safety Boundaries

CrabDB's safety checks sit between user/agent requests and workspace mutation.

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
internals, ignored/private paths, risky agent actions, dirty materialized
workdirs, stale refs, and conflicted merges before changes are accepted.

## Quick Start

CrabDB is a Rust workspace. The repository declares Rust 1.81 in `Cargo.toml`.

```sh
cargo build -p crabdb
target/debug/crabdb --help
```

Initialize a workspace from the current working tree:

```sh
target/debug/crabdb init --working-tree
```

Inspect and record an edit:

```sh
target/debug/crabdb status
target/debug/crabdb record -m "record current edit"
target/debug/crabdb timeline --limit 10
target/debug/crabdb show <change-id>
```

Ask provenance questions:

```sh
target/debug/crabdb why README.md:2
target/debug/crabdb history README.md
target/debug/crabdb diff --dirty --patch
```

Initialization creates `.crabdb/` state and a `.crabignore` file when needed.
Default ignore patterns protect CrabDB/Git internals, environment files, private
key/certificate files, dependency folders, build output, and coverage output.

Later examples use `crabdb` for readability. If the binary is not on your PATH,
replace `crabdb` with `target/debug/crabdb`.

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

Use `--json` or `--format json` on commands when a script, editor, or agent needs
machine-readable output.

## Agent Workflow

Agents work on isolated CrabDB refs instead of immediately changing `main`.

```sh
crabdb agent spawn doc-bot --from main
crabdb agent status doc-bot
```

Apply a structured patch directly to the agent branch:

```sh
crabdb agent apply-patch doc-bot --patch patch.json
```

Review and gate the work:

```sh
crabdb agent diff doc-bot --patch --show-line-ids
crabdb agent contribution doc-bot
crabdb agent readiness doc-bot
crabdb agent handoff doc-bot
```

When a tool needs a filesystem checkout, create or sync a materialized workdir:

```sh
crabdb agent spawn doc-bot --from main --materialize=true
crabdb agent workdir doc-bot
crabdb agent record doc-bot -m "record workdir changes"
crabdb agent sync-workdir doc-bot
```

Merge only after review and readiness checks:

```sh
crabdb merge-agent doc-bot --into main --dry-run
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
- [Install and build](docs/getting-started/install-and-build.md)
- [Initialize a workspace](docs/getting-started/initialize-a-workspace.md)
- [First record and provenance query](docs/getting-started/first-record-and-query.md)
- [First agent workflow](docs/getting-started/first-agent-workflow.md)

Read by topic:

- [Core concepts](docs/concepts/operation-database.md)
- [Guides](docs/guides/record-worktree-changes.md)
- [Use cases](docs/use-cases/local-code-history.md)
- [Agent workflows](docs/agents/overview.md)
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

## Current Boundaries

CrabDB is local-first. It does not require a hosted service, and its HTTP daemon
is a loopback integration surface rather than a cloud API.

CrabDB complements Git rather than replacing it. Use Git for shared commit
history; use CrabDB for local operation history, provenance, agent coordination,
and review signals before or around commits.

Safety features are conservative but not magic. Keep secrets ignored, review
patches and diffs, treat guardrail decisions as preflight signals, and use human
approvals for sensitive agent actions.

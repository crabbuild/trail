# CrabDB

CrabDB is a local-first, prolly-tree-backed operation database for code and text
worktrees. Git records committed snapshots; CrabDB records the work that happens
between commits: saves, patches, branches, agent edits, merges, and line-level
provenance.

## Quick Start

```bash
cargo build -p crabdb

# Initialize from the current working tree.
cargo run -p crabdb -- init --working-tree

# Inspect and record changes.
cargo run -p crabdb -- status
cargo run -p crabdb -- record -m "Describe this operation"
cargo run -p crabdb -- timeline
cargo run -p crabdb -- show ch_...

# Ask why a line exists.
cargo run -p crabdb -- why src/lib.rs:42

# Start the local API for editor and agent integrations.
# By default this creates .crabdb/daemon.token and requires bearer auth.
cargo run -p crabdb -- daemon

# Export the machine-readable local API contract.
cargo run -p crabdb -- api openapi --output crabdb.openapi.json

# Start the MCP stdio server for agent hosts.
cargo run -p crabdb -- mcp
```

After initialization, CrabDB stores local state in `.crabdb/` and uses
`.crabignore` plus Git ignore files to avoid recording secrets, build output,
dependencies, and CrabDB internals. Use `crabdb ignore list`,
`crabdb ignore add`, and `crabdb ignore check` to manage local privacy rules.

## Agent Workflow

```bash
crabdb agent spawn doc-bot --from main
crabdb agent turn start doc-bot --from main --title "Improve docs"
crabdb agent turn message turn_... --role user --text "Improve the docs"
crabdb agent turn event turn_... --event-type tool_call \
  --payload-json '{"tool":"editor.apply_patch","status":"started"}'
crabdb agent trace start turn_... --type tool_call --name editor.apply_patch
crabdb agent trace end span_... --status completed
crabdb agent claim doc-bot README.md
crabdb guardrails check --agent doc-bot --action shell.exec \
  --summary "Run smoke tests" --path README.md
crabdb config set guardrails.policy \
  "allow:action:shell.exec; block:keyword:production"
crabdb approvals request doc-bot --action shell.exec \
  --summary "Run smoke tests" --turn turn_...
crabdb agent turn apply-patch turn_... --patch patch.json
crabdb agent record doc-bot -m "Workdir changes"
crabdb agent sync-workdir doc-bot
crabdb agent test doc-bot -- cargo test
crabdb agent eval doc-bot -- ./scripts/eval-agent.sh
crabdb agent turn end turn_... --status completed
crabdb agent contribution doc-bot
crabdb agent readiness doc-bot
crabdb agent handoff doc-bot
crabdb agent diff doc-bot --patch --show-line-ids
crabdb merge-queue add doc-bot --into main
crabdb merge-queue run
```

Each agent gets an isolated `refs/agents/<name>` branch and, by default, a
materialized workdir under `.crabdb/worktrees/<name>/`. Merging back to `main`
is explicit, serialized through an optional merge queue, and conservative.

## Documentation

- [User Guide](docs/USER_GUIDE.md)
- [CLI Reference](docs/CLI_REFERENCE.md)
- [Agent Workflows](docs/AGENT_WORKFLOWS.md)
- [Design Documents](docs/)

## Current Scope

This repository now includes a production-oriented local v1 slice:

- Persistent SQLite object store, refs, operations, and query indexes.
- SQLite schema version metadata with a downgrade guard for newer workspaces.
- Prolly-backed worktree path maps and file identity maps.
- Prolly-backed text order maps and line identity maps.
- Stable `ChangeId`, `FileId`, and `LineId` provenance.
- Same-position rewrite handling that preserves line identity when no better
  move match exists.
- Safe checkout, branch, timeline, root/dirty diff, why, fsck, Git
  import-update, patch export, and agent commands.
- Checkout and workdir materialization reject path escapes, symlink write
  redirection, and case-insensitive path collisions before writing files.
- Selective recording with `--paths`, explicit record kinds, session
  attachment, and opt-in ignored fixture capture.
- Polling watcher records can attach to durable sessions for long-running local
  agent or review loops.
- Ignore management CLI plus a hardcoded denylist for secrets, private keys,
  certificates, dependency folders, build output, and CrabDB/Git internals.
- Built-in trace metadata redaction for common token/password/secret patterns
  in agent messages, events, approvals, and operation messages.
- Git mapping audit records for imports, including Git HEAD and dirty tracked
  state.
- Git export can emit review patches or create Git commit objects from CrabDB
  roots without moving `HEAD`.
- Typed `config` CLI and SDK helpers for validated workspace settings.
- HTTP and MCP config list/get/set surfaces for editor and agent-host
  integrations.
- Read-only `show`, `history`, and `code-from` inspection for operations,
  messages, refs, agents, files, lines, sessions, and agent output.
- HTTP and MCP provenance inspection for `why`, `history`, `code-from`, and
  durable review anchors.
- Scoped operation timelines for branches, agents, and sessions through CLI,
  HTTP, and MCP.
- Stable line anchors for durable review references across nearby edits.
- Read-only status checks, workspace write locking, and CAS-protected ref
  advancement for local direct mode.
- Agent patch message and event indexes for trace-style provenance.
- Agent patch privacy enforcement: ignored paths are rejected by default, with
  explicit `allow_ignored` opt-in and CrabDB/Git internals always blocked.
- Agent lifecycle commands for listing, status, messages, timelines, checkout,
  and removal.
- HTTP and MCP agent lifecycle tools for spawn, list, show/status, and safe
  removal of completed or abandoned branches.
- Agent contribution review bundles through CLI, HTTP, and MCP, combining
  branch status, changed paths, operations, sessions, events, approvals, and
  latest gates.
- Agent merge-readiness reports through CLI, HTTP, and MCP, turning conflicts,
  pending approvals, dirty workdirs, failed gates, and review warnings into one
  machine-readable merge signal.
- Agent handoff packets through CLI, HTTP, and MCP, combining readiness, active
  session context, recent operations, trace events, spans, and deterministic
  next steps for transfer between agents or reviewers.
- Configured and per-agent custom materialized workdir paths for deterministic
  local agent hosts.
- Local HTTP API endpoints for workspace status, ref/root/dirty diff, agent
  spawn/status/diff/patch, merge-agent, ignore controls, and durable turn
  workflows.
- Explicit CLI agent turn commands for start, trace event, inspection, and
  closeout workflows.
- First-class agent sessions and turns for durable run-level audit trails.
- Current-session discovery and explicit session lifecycle management through
  CLI, HTTP, and MCP.
- Bounded session context packets for long-running agents that need compact
  recent messages, trace events, turns, and operations with total counts.
- Read-only guardrail preflight through CLI, HTTP, and MCP for shell, network,
  deploy, destructive, policy, and ignored-path actions before agents mutate the
  workspace or external systems, with configurable local policy rules in
  `guardrails.policy`.
- Durable human approval gates for sensitive agent actions through CLI, HTTP,
  and MCP, with decisions linked back into agent/session traces and matching
  approved/rejected decisions reflected in later guardrail checks.
- Durable paused agent run checkpoints through CLI, HTTP, and MCP, with
  approval requests returning a linked resumable run state and rejected
  approvals blocking linked resumes.
- Local JSON HTTP API for agent turn, message, patch, and close workflows.
- MCP stdio tool server for model-controlled CrabDB status, agent turn, trace,
  patch, and diff workflows.
- HTTP and MCP surfaces for agent test gates and isolated workdir refresh.
- HTTP and MCP ignore preflight tools for integrations that need to respect
  CrabDB's local privacy policy.
- Turn-level trace events for tool calls, guardrails, handoffs, and custom
  runtime observations.
- Filtered trace event queries through CLI, HTTP, and MCP for production agent
  observability across agents, sessions, turns, and event types.
- Parentable trace spans for agent/tool/guardrail/evaluation work with trace
  IDs, span IDs, status, timing, attributes, and results exposed through CLI,
  HTTP, and MCP.
- Trace summary rollups for open spans, failed spans, status/type/trace counts,
  and sampled slow spans across CLI, HTTP, and MCP.
- Agent workdir recording for tools that edit isolated materialized worktrees.
- Agent workdir path and polling watcher commands for direct-edit agent loops.
- Agent workdir sync command with dirty-workdir refusal and force refresh.
- Agent test runner for materialized workdirs with durable pass/fail events and
  stdout/stderr output blobs.
- Agent eval runner for materialized workdirs with durable eval events,
  pass/fail status, stdout/stderr output blobs, and latest-eval status
  reporting.
- Dirty agent workdir detection and merge refusal before unrecorded edits can be
  skipped.
- Line-id structured patch edits with expected-text stale context guards.
- Low-level object, root, text, and prolly map inspection commands for operator
  debugging.
- Advisory path leases for multi-agent coordination.
- HTTP and MCP advisory lease tools for agent-host path coordination.
- Line-aware text merge for non-overlapping stable-line edits.
- Direct agent merges and serial merge queues with durable merge-result audit
  rows, structured conflict sets, and conflict inspection/resolution.
- HTTP and MCP merge queue and conflict tools for serialized multi-agent merge
  coordination.
- Manual conflict resolution through CLI, HTTP, and MCP with per-path resolved
  content, deletion support, stale-ref checks, and ordinary merge operation
  provenance.
- Read-only `doctor` diagnostics through CLI, HTTP, and MCP for workspace
  health, locks, fsck, approvals, leases, queues, conflicts, and agent
  workdirs.
- Read-only MCP resources for workspace status, doctor diagnostics, agents,
  merge queue, conflicts, OpenAPI, and bundled user/agent/CLI guides.
- MCP resource templates and completions for host navigation across agents,
  agent handoffs, sessions, turns, conflicts, approvals, run states, and trace
  spans.
- MCP tool annotations for every advertised tool, including read-only,
  destructive, idempotent, and open-world hints for host permission UX.
- MCP prompt templates for safe agent task execution, branch review, and
  structured conflict resolution workflows.
- Machine-readable OpenAPI 3.1 contract through `crabdb api openapi` and the
  authenticated `GET /v1/openapi.json` daemon endpoint.
- Verifiable backup bundles and restore commands for CrabDB metadata, objects,
  refs, `.crabignore`, and materialized agent worktrees.
- Rebuildable derived indexes and conservative object garbage collection.
- JSON output for automation and agent/editor integrations.
- Stable machine-readable CLI error envelopes in JSON output mode, including
  error codes and exit codes for agent runners.

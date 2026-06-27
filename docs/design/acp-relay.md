# ACP Relay Design

Status: initial implementation.

This design describes how CrabDB can become the neutral capture, branch, and
recovery layer for ACP-compatible coding agents without becoming an agent
runtime itself.

## Goal

Let a user keep working from an ACP-capable editor while CrabDB automatically:

- Creates or reuses an isolated CrabDB lane branch.
- Records prompts, assistant output, tool calls, approvals, and terminal or file
  events as CrabDB sessions, turns, messages, events, and spans.
- Captures code changes as CrabDB operations through structured patches when
  possible, or through materialized workdir recording when the upstream agent
  edits files directly.
- Exposes CrabDB MCP tools to the upstream coding agent.
- Preserves bad attempts and supports rewind without polluting the user's active
  Git branch.

## Current Foundation

CrabDB already has the primitives the relay needs:

- Lane branches: durable branch-like refs under `refs/lanes/<name>`.
- Sessions, turns, messages, events, spans, runs, approvals, gates, and evals.
- MCP tools/resources/prompts for agent hosts.
- HTTP/OpenAPI and Rust APIs over the same `CrabDb` core.
- Materialized lane workdirs and `lane record` for capturing filesystem edits.
- Structured patches through `lane apply-patch` and MCP `crabdb.apply_patch`.
- `lane rewind` for auditable recovery to a known-good change or root.

The missing piece is not a new storage model. It is a relay that sits between
ACP editors and real coding agents, mirrors the protocol activity into CrabDB,
and ensures the real agent can see CrabDB's MCP tools.

## Implementation Status

The first production slice is implemented in `crabdb acp relay`.

Implemented:

- New CLI group: `crabdb acp relay -- <upstream-command>`.
- Newline-delimited JSON-RPC stdio relay for local ACP agents.
- Upstream child process lifecycle and stderr isolation.
- `_meta.crabdb` initialization metadata.
- Lane creation/reuse for ACP sessions.
- Durable ACP-to-CrabDB session mapping in `lane_acp_sessions`.
- CrabDB MCP injection into `session/new`, `session/load`, and
  `session/resume`.
- Optional materialized lane workdir routing via `--materialize`.
- Prompt capture as CrabDB turns with user/assistant messages.
- `session/update` capture for plans, generic updates, tool spans, and
  assistant message chunks.
- Privacy-conscious ACP update filtering: internal agent thought chunks are not
  persisted, and available-command lists are summarized instead of storing full
  command descriptions.
- Relay-scoped writer-lock waiting so concurrent ACP relay processes do not
  drop capture events when another process is briefly writing the workspace.
- ACP permission requests mirrored into CrabDB approvals/run state.
- Workdir recording at prompt completion linked to the active prompt turn.
- Conservative structured ACP `diff` content capture as CrabDB `write` patch
  edits for non-materialized sessions.
- Guided setup and inspection commands: `crabdb acp install`, `crabdb acp list`,
  `crabdb acp doctor`, `crabdb acp sessions`, `crabdb transcript`, and top-level
  `crabdb turn show`.
- Bounded assistant/event capture with truncation events and relay EOF closeout
  for open turns.

Not yet implemented:

- Mutating editor config generation; `acp install` prints snippets only.
- Broad structured edit conversion beyond ACP `diff` content with `newText`.
- Remote ACP transports while the HTTP transport remains draft.
- Long-running assistant message checkpointing before prompt completion.

## Performance Validation

A release-build synthetic ACP benchmark was run on June 26, 2026 using real
`crabdb acp relay` processes and fake ACP agents. The relay uses an in-process
batched turn-event writer so high-volume ACP updates are buffered during a turn
and committed to CrabDB under one writer lock at flush points. Each materialized
turn emitted usage, tool, tool-update, and assistant-message updates, wrote one
file into the lane workdir, and let CrabDB record the prompt checkpoint.

Materialized edit checkpoint results after batched event capture:

| Concurrent relays | Turns | Wall time | Throughput | p50 prompt | p95 prompt | p99 prompt |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 16 | 320 | 2.48s | 129 turns/s | 9.2ms | 442ms | 994ms |

The 16-relay materialized run captured all 16 lanes, 16 ACP session mappings,
320 turns, 5,248 events, 321 operations, and 2,576 streamed updates with zero
relay warnings and zero persisted thought events.

A separate 16-relay capture-only run with `--no-materialize` captured 800 turns,
12,128 events, and 6,416 streamed updates in 5.36s, or 149 turns/s. Prompt
latency was p50 17ms, p95 426ms, p99 873ms, with zero relay warnings.

Compared with the pre-batching baseline, the 16-relay materialized run improved
from 102 to 129 turns/s and reduced p50 prompt latency from 20.5ms to 9.2ms.
The 16-relay capture-only run improved from 129 to 149 turns/s and reduced p95
prompt latency from 492ms to 426ms.

Interpretation:

- Aggregate throughput is acceptable for multi-agent coding workflows because
  model/tool latency will usually dominate these local capture costs.
- Tail latency grows under high write concurrency because CrabDB intentionally
  serializes workspace mutations through its writer lock.
- The remaining performance improvement is a daemon-backed cross-process writer
  that can coalesce writes from many relay processes while preserving CrabDB's
  single-writer correctness model.

## Protocol Assumptions

This design targets Agent Client Protocol, not IBM Agent Communication Protocol
or other ACP acronyms.

External protocol facts used by this design:

- ACP standardizes communication between editors/IDEs and coding agents and is
  suitable for local and remote scenarios.
- Local ACP agents normally run as editor subprocesses over JSON-RPC stdio.
- Remote ACP agents over HTTP or WebSocket are still developing.
- ACP initialization negotiates protocol version, capabilities, and
  authentication.
- `session/new` includes a working directory and a list of MCP servers the agent
  should connect to.
- Prompt turns use `session/prompt`, stream progress through `session/update`,
  and end with a prompt response.
- ACP content blocks reuse the same `ContentBlock` structure as MCP.
- ACP supports `_meta` fields for compatible extension metadata.

References:

- https://agentclientprotocol.com/get-started/introduction
- https://agentclientprotocol.com/protocol/v1/initialization
- https://agentclientprotocol.com/protocol/v1/session-setup
- https://agentclientprotocol.com/protocol/v1/prompt-turn
- https://agentclientprotocol.com/protocol/v1/content
- https://agentclientprotocol.com/protocol/v1/transports
- https://agentclientprotocol.com/protocol/v1/extensibility
- https://github.com/agentclientprotocol/agent-client-protocol
- https://modelcontextprotocol.io/specification/2025-06-18

## User Model

Without the relay:

```text
Editor -> real ACP agent
```

With CrabDB:

```text
Editor -> crabdb ACP relay -> real ACP agent
                                  |
                                  v
                              CrabDB MCP
```

The user should still pick an agent from the editor. The label may be
`Claude via CrabDB`, `Codex via CrabDB`, or `CrabDB Agent`. The relay launches
the selected real agent and mirrors what happens into CrabDB.

The user-facing interpretation is:

```text
The real agent writes code.
CrabDB remembers what happened.
CrabDB keeps each lane on its own branch-backed ref.
CrabDB lets me review, merge, or rewind.
```

## CLI Surface

Run a local ACP relay in front of a real ACP agent:

```sh
crabdb acp relay \
  --lane docs-bot \
  --from main \
  --provider anthropic \
  --model claude-code \
  --materialize \
  -- claude-acp-agent --stdio
```

Useful flags:

- `--lane <name>` pins the ACP session to a stable CrabDB lane.
- `--from <ref>` selects the base ref used when the lane is first created.
- `--materialize` routes the upstream ACP session `cwd` to a materialized lane
  workdir and records filesystem edits at prompt completion. This is the relay
  default because most coding agents edit files directly.
- `--no-materialize` leaves `cwd` untouched and relies on structured CrabDB MCP
  operations or later manual recording.
- `--provider` and `--model` annotate the lane, turns, and session mapping.
- `--no-mcp` disables dynamic CrabDB MCP injection.

Supporting setup commands are exposed through `crabdb acp`:

```sh
crabdb acp list
crabdb acp install --agent claude-code --print
crabdb acp doctor --agent claude-code
crabdb acp sessions
crabdb transcript <lane-or-session>
```

`acp install` prints editor-specific launch snippets only; it does not mutate
editor configuration.
The relay itself must be usable directly from any ACP editor that can launch a
local agent command.

## Architecture

```text
ACP editor/client
    |
    | JSON-RPC stdio
    v
crabdb acp relay
    |                          \
    | JSON-RPC stdio            \ Rust API / local daemon
    v                            v
real ACP agent                 CrabDB
    |
    | MCP server config injected by relay
    v
crabdb mcp
```

The relay has three roles:

1. ACP server to the editor.
2. ACP client to the upstream real agent.
3. CrabDB capture coordinator.

It should relay ACP traffic faithfully. CrabDB capture must not change what the
editor or upstream agent sees unless a safety policy explicitly blocks an
operation.

## Component Plan

### `acp` CLI Module

New files should live under `crates/crabdb/src/cli/command/acp_args.rs` and
`crates/crabdb/src/cli/command/handler/acp.rs`.

Responsibilities:

- Parse relay configuration.
- Resolve runtime context and workspace.
- Spawn the relay runtime.
- Report clear setup errors for missing upstream command, unsupported ACP
  version, or unavailable CrabDB workspace.

### ACP Transport Runtime

Add an internal transport module, likely under `crates/crabdb/src/acp`.

Responsibilities:

- Read/write JSON-RPC messages on stdin/stdout for the editor side.
- Spawn the upstream ACP agent as a child process.
- Read/write JSON-RPC messages to the upstream child process.
- Preserve request IDs and notification ordering.
- Route client-to-agent, agent-to-client, and relay-handled messages.
- Shut down child processes cleanly on EOF, cancel, or signal.

Prefer the official Rust ACP runtime crate when it is stable enough for the
needed relay role. Fall back to the lower-level schema crate or local typed
message structs if relay support is not mature enough.

### Session Mapper

The relay needs a durable mapping between ACP sessions and CrabDB sessions.

The initial implementation adds this table:

```sql
CREATE TABLE IF NOT EXISTS lane_acp_sessions (
    acp_session_id TEXT PRIMARY KEY,
    upstream_session_id TEXT,
    lane_id TEXT NOT NULL,
    crabdb_session_id TEXT NOT NULL,
    cwd TEXT NOT NULL,
    provider TEXT,
    model TEXT,
    upstream_command_json TEXT,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

The relay also keeps an in-memory cache for active process state, but the
durable table is the source of truth for `session/load`, `session/resume`,
crash recovery, and cross-editor handoff.

### Capture Coordinator

The capture coordinator maps ACP traffic to CrabDB calls.

It maintains per-session state:

- ACP session ID.
- Upstream ACP session ID if different.
- CrabDB lane name and lane ID.
- CrabDB session ID.
- Current turn ID, if a prompt is active.
- Buffered assistant messages by ACP message ID.
- Active tool spans by ACP tool call ID.
- Last known lane branch head.
- Materialized workdir path, when enabled.

## Message Mapping

### `initialize`

Flow:

1. Editor calls `initialize` on relay.
2. Relay calls `initialize` on upstream agent.
3. Relay returns upstream capabilities, with relay metadata added under
   `_meta.crabdb`.

Capture:

- Add no CrabDB turn.
- Optionally emit a process-level diagnostic event only after a CrabDB lane is
  known.

Failure handling:

- If ACP protocol versions are incompatible, return the upstream error.
- If the upstream agent requires authentication, pass through auth requirements.

### `session/new`

Flow:

1. Editor sends `cwd` and `mcpServers`.
2. Relay ensures a CrabDB lane exists.
3. Relay starts or reuses a CrabDB session for that agent.
4. Relay injects the CrabDB MCP server into `mcpServers`.
5. Relay forwards `session/new` to the upstream agent.
6. Relay records ACP and CrabDB session mapping.

Capture:

- `lane_spawn` if the lane does not exist.
- `session_started` event if a new CrabDB session is created.
- `acp_session_started` event with upstream command, cwd, provider, model, and
  ACP session ID.

MCP injection should look conceptually like:

```json
{
  "name": "crabdb",
  "command": "crabdb",
  "args": ["mcp"],
  "env": [
    {"name": "CRABDB_WORKSPACE", "value": "/repo"},
    {"name": "CRABDB_DIR", "value": "/repo/.crabdb"}
  ]
}
```

The exact command should use the current binary path when possible so editor
launch environments do not accidentally resolve a different `crabdb`.

### `session/load` and `session/resume`

Flow:

1. Look up ACP session mapping.
2. Reconnect or restore the upstream agent according to its advertised
   capabilities.
3. Reattach the CrabDB session.
4. Forward replayed `session/update` notifications to the editor.

Capture:

- Do not duplicate already-recorded messages during replay.
- Record `acp_session_loaded` or `acp_session_resumed` event.
- If the upstream agent replays messages that CrabDB has not seen, import them
  as replay events with `_meta.crabdb.replayed = true`.

### `session/prompt`

Flow:

1. Editor sends user prompt content blocks.
2. Relay starts a CrabDB turn.
3. Relay stores the user prompt with `crabdb.add_message`.
4. Relay starts a root trace span for the prompt turn.
5. Relay forwards `session/prompt` to upstream.
6. Relay mirrors all `session/update` notifications while the prompt runs.
7. On final response, the relay records status and ends the CrabDB turn.

Capture:

- Turn metadata:
  - ACP request ID.
  - ACP session ID.
  - provider and model.
  - cwd.
  - upstream command hash, not raw secret-bearing argv.
- User message:
  - Text content as message body.
  - Non-text blocks summarized into metadata or event payloads.
- Prompt response:
  - `completed`, `failed`, or `cancelled` turn status.
  - stop reason in event payload.

### `session/update`

The relay forwards every update to the editor and mirrors selected updates to
CrabDB.

Suggested mapping:

| ACP update | CrabDB capture |
| --- | --- |
| user message chunk replay | `add_message` only if not previously recorded |
| agent message chunk | buffer by message ID, flush to assistant message |
| plan update | `add_event(event_type = "plan_update")` |
| tool call pending | `span_start(span_type = "tool")` |
| tool call update | `add_event(event_type = "tool_call_update")` |
| tool call finished | `span_end(status = "...")` |
| file/diff update | `add_event(event_type = "file_update")`; maybe patch |
| permission request | CrabDB approval/run checkpoint |
| session info update | CrabDB session metadata event |
| current mode update | `add_event(event_type = "agent_mode_update")` |

Assistant message chunking needs a flush policy. MVP can flush once at prompt
completion. Later, it can checkpoint long messages periodically to survive
crashes.

### File Operations And Edits

There are three edit paths, in preferred order.

1. Native CrabDB patch path.
   If the upstream agent calls CrabDB MCP `crabdb.apply_patch`, the operation is
   already structured, linked to the turn, and branch-safe.

2. ACP diff/file update path.
   If ACP exposes a structured edit or diff update, the relay can convert it
   to a CrabDB patch and apply it to the lane branch.

3. Materialized workdir path.
   If the upstream agent edits files in the filesystem directly, the relay
   captures the resulting diff by calling CrabDB's workdir record path at turn
   end or at checkpoints.

MVP should use materialized workdirs because it works with the broadest set of
agents. Structured patch conversion should be a follow-up once real ACP agent
behavior is observed.

### Permission And Approval Requests

If the upstream agent asks the editor for permission through ACP, the relay
should:

1. Forward the request to the editor unchanged.
2. Record an approval request in CrabDB.
3. If the operation is blocked waiting for the user, create or update a CrabDB
   paused run checkpoint.
4. Record approval decision events after the editor responds.

This keeps the editor UX authoritative while giving CrabDB durable recovery
state.

### `session/cancel`

Flow:

1. Forward cancel to upstream.
2. End the active CrabDB span.
3. Mark the active turn as `cancelled` if the prompt is abandoned.
4. Optionally record workdir state if files were modified before cancellation.

### `session/close`

Flow:

1. Forward close to upstream if supported.
2. End any open CrabDB turn as `cancelled` or `failed`.
3. End the CrabDB session if configured to close with ACP.
4. Mark `lane_acp_sessions.status = 'closed'`.

## Lane Branch Strategy

Default strategy:

- One ACP session maps to one CrabDB lane branch unless the user explicitly
  pins a branch name.
- Lane names are user-friendly and stable:
  - explicit `--lane docs-bot`, or
  - generated `acp-<provider>-<short-session>`.
- `--from main` defines the base branch for first spawn.
- `--materialize` creates a workdir for agents that edit files directly.

The relay should not merge into `main` automatically. It should produce review
state:

```sh
crabdb lane review <lane>
crabdb lane diff <lane> --patch
crabdb merge-lane <lane> --into main --dry-run
crabdb merge-queue add <lane> --into main
```

## Multi-Agent Coordination

Multiple wrapped ACP sessions can run at once:

```text
Editor A -> crabdb acp relay --lane docs-bot  -> Claude ACP
Editor B -> crabdb acp relay --lane tests-bot -> Codex ACP
Editor C -> crabdb acp relay --lane ui-bot    -> Gemini ACP
```

CrabDB provides:

- Separate refs and heads for each lane.
- Advisory path claims/leases.
- Readiness checks.
- Conflict sets.
- Merge queue serialization.
- Rewind per lane branch.

The relay can improve coordination by:

- Claiming paths when the user prompt, plan, or tool update identifies an edit
  scope.
- Recording conflicts between active claims as warnings.
- Adding claim warnings to the turn event log.

Claims remain advisory. Merge readiness and conflict handling are the
authoritative safety checks.

## Recovery Model

If the upstream agent fails:

- Mark the active span and turn as `failed`.
- Record the upstream exit status or protocol error after redaction.
- Preserve any materialized workdir changes with `lane record` if configured.
- Leave the lane branch inspectable.

If the work is wrong:

```sh
crabdb lane rewind <lane> --to <known-good> --record-current --sync-workdir
```

This should be exposed in editor UX later as:

- Rewind to turn start.
- Rewind to session start.
- Preserve failed attempt.
- Open failed attempt diff.

## Security And Privacy

The relay sees sensitive data: prompts, file paths, tool calls, terminal
commands, and sometimes model output. Requirements:

- Local-first by default.
- Do not send CrabDB capture data to remote services.
- Redact sensitive payloads before storing events.
- Do not persist raw environment variables by default.
- Store upstream command as structured metadata with secret-like args redacted.
- Respect CrabDB ignore rules for workdir recording.
- Run CrabDB guardrail checks before relay-initiated shell, network,
  destructive, deploy, or ignored-path operations.
- Keep editor permission UX authoritative; CrabDB records decisions but should
  not silently bypass the editor.

## Data Model Additions

Required for MVP:

- No new domain object type.
- New operation kinds are not required unless the relay starts applying
  structured patches itself. Existing `LanePatch`, `LaneRecord`, and
  `LaneRewind` cover the branch mutations.

Likely needed for production:

- `lane_acp_sessions` table for ACP-to-CrabDB session mapping.
- Optional `lane_external_ids` table if other host protocols need the same
  mapping later.
- Event payload conventions for ACP metadata:

```json
{
  "protocol": "acp",
  "acp_session_id": "sess_...",
  "acp_request_id": 2,
  "upstream": {
    "provider": "anthropic",
    "model": "claude-code"
  }
}
```

Use `_meta.crabdb` in ACP traffic only for best-effort correlation. Do not
require editors or upstream agents to preserve it.

## Implementation Phases

### Phase 0: Finalize Protocol Spike

Deliverables:

- Confirm chosen ACP Rust crate or local schema strategy.
- Build a tiny fake ACP agent fixture for tests.
- Build a transcript fixture covering `initialize`, `session/new`,
  `session/prompt`, `session/update`, tool call updates, and final response.
- Document which ACP updates are observed from one real provider.

Acceptance criteria:

- A local test can relay JSON-RPC messages between a fake editor and fake agent.
- Request IDs, notifications, errors, and EOF are preserved.

### Phase 1: Pass-Through ACP Relay

Deliverables:

- `crabdb acp relay -- <upstream-command>`.
- Child process lifecycle management.
- Transparent ACP forwarding.
- Basic `initialize` and `session/new` pass-through.
- Clean shutdown.

Acceptance criteria:

- An ACP editor can talk to a real upstream ACP agent through the relay.
- No CrabDB capture is required yet beyond debug logs.

### Phase 2: Session And Prompt Capture

Deliverables:

- Ensure or spawn CrabDB lane branch.
- Create CrabDB session on `session/new`.
- Begin CrabDB turn on `session/prompt`.
- Store user prompt and assistant response messages.
- End turn with completed/failed/cancelled status.
- Record ACP session events.

Acceptance criteria:

- `crabdb lane turn show <turn-id>` shows the user prompt, assistant response,
  events, and session.
- `crabdb lane events <lane>` shows ACP session and prompt events.

### Phase 3: MCP Injection And Tool/Span Capture

Deliverables:

- Inject CrabDB MCP server into ACP `mcpServers`.
- Mirror ACP tool call updates into CrabDB spans.
- Mirror permission requests into CrabDB approvals/runs.
- Expose relay metadata in `_meta.crabdb`.

Acceptance criteria:

- Upstream agent can call CrabDB MCP tools.
- `crabdb span list --lane <lane>` shows tool spans.
- Approval waits can be resumed through CrabDB run state.

### Phase 4: Workdir Capture MVP

Deliverables:

- Support `--materialize`.
- Point upstream ACP session `cwd` at the materialized workdir when requested.
- Record workdir changes at prompt completion.
- Optional periodic checkpoints for long turns.
- Rewind integration for editor UX.

Acceptance criteria:

- A real ACP agent edits files normally.
- CrabDB records the resulting diff as a lane operation.
- `crabdb lane diff <lane>` shows the change.
- `crabdb lane rewind <lane> --record-current --sync-workdir` recovers the
  branch and workdir.

### Phase 5: Structured Edit Capture

Deliverables:

- Convert supported ACP diff/file updates to CrabDB patch documents.
- Prefer `crabdb.apply_patch` when a safe structured patch can be produced.
- Fall back to workdir record when conversion is incomplete.

Acceptance criteria:

- Supported edits produce precise CrabDB operations during the turn.
- Unsupported edits still capture correctly at checkpoint or turn end.

### Phase 6: Multi-Agent Product UX

Deliverables:

- `crabdb acp init <provider>` setup helpers.
- `crabdb acp list` and `crabdb acp doctor`.
- Recommended editor config snippets.
- Lane naming policy.
- Parallel-session docs.
- Review, merge, and rewind affordances for editors that support custom commands
  or links.

Acceptance criteria:

- A user can configure at least one editor and one upstream ACP agent using docs
  or generated config.
- Two ACP sessions can run concurrently against two CrabDB lane branches.
- Merge queue and conflict handling work with captured branches.

## Test Plan

Unit tests:

- JSON-RPC framing and ID preservation.
- ACP session mapping.
- MCP server injection.
- Message chunk buffering and flushing.
- Tool call update to span mapping.
- Redaction of event payloads and upstream command metadata.

Integration tests:

- Fake editor -> relay -> fake ACP agent.
- `session/new` creates CrabDB session mapping.
- `session/prompt` records a CrabDB turn and messages.
- `session/update` tool calls produce spans.
- Upstream file write plus turn completion records workdir diff.
- Cancelled prompt marks turn cancelled.
- Upstream crash marks turn failed and leaves workdir recoverable.
- Rewind preserves failed head and syncs workdir.

Manual compatibility tests:

- One ACP editor.
- One ACP-capable upstream coding agent.
- One non-ACP agent through a provider-specific adapter, if needed.

## Open Questions

- Which ACP Rust crate should be used for proxying: high-level runtime or schema
  crate plus local transport?
- Do target editors allow a local ACP command that itself launches another
  local process with arbitrary args?
- Which upstream agents report edits as structured ACP updates versus direct
  filesystem writes?
- How much assistant streaming should be persisted before prompt completion?
- Should `session/new` always create a new CrabDB session, or should users be
  able to attach to an existing CrabDB session by name?
- Should path claims be automatic, prompt-driven, or explicit only?
- How should remote ACP agents be supported once the remote transport stabilizes?

## Delivery Recommendation

Deliver the relay in this order:

1. Transparent ACP relay.
2. Session and prompt capture.
3. MCP injection.
4. Workdir capture.
5. Tool spans and approvals.
6. Structured edit capture.
7. Editor setup helpers.

This order gives users value early while keeping risk contained. A transparent
relay proves editor compatibility first. Session capture proves the audit log.
Workdir capture then gives broad compatibility with real agents, even before
structured ACP edit mapping is mature.

## Code Facts Used

- Lane lifecycle: `crates/crabdb/src/db/lane/lifecycle.rs`
- Lane branches and activity models: `crates/crabdb/src/model/lane`
- Sessions, turns, runs, approvals: `crates/crabdb/src/db/lane/control`
- Events and spans: `crates/crabdb/src/db/lane/control/traces`
- Lane patching: `crates/crabdb/src/db/lane/patching.rs`
- Lane workdir recording: `crates/crabdb/src/db/lane/workdir`
- Lane rewind: `crates/crabdb/src/db/lane/rewind.rs`
- MCP server: `crates/crabdb/src/mcp`

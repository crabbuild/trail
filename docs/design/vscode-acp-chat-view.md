# VS Code ACP Chat View Design

Status: proposed.

This document designs the Trail VS Code extension chat surface. The goal is a
single editor UI that can drive any Agent Client Protocol agent while Trail
remains the durable source of truth for agent tasks, transcripts, turns,
events, checkpoints, diffs, readiness, merge state, and recovery.

The extension is an ACP client. It should not become an agent runtime, a second
database, or a provider-specific transcript store.

## Product Goal

Let a user run Claude Code, Codex, Gemini, OpenCode, Goose, or another ACP
agent through one VS Code interface:

```text
VS Code Trail extension
    |
    | ACP JSON-RPC stdio
    v
trail agent acp run <provider>
    |
    | Trail ACP relay
    v
real ACP agent or provider adapter
```

In parallel, the extension talks to the Trail daemon:

```text
VS Code Trail extension
    |
    | local HTTP JSON
    v
trail daemon
```

The split is intentional:

- ACP is the live interaction stream.
- Trail HTTP is the persisted task model.
- Trail lanes are the safety boundary.
- The selected provider remains replaceable.

## User Model

Primary users:

- Developers who use several coding agents and do not want a different editor
  workflow for each provider.
- Developers who want agent work isolated until reviewed and applied.
- Teams evaluating agent output and needing transcripts, patches, readiness
  gates, and rewindable attempts.

Core vocabulary:

- **Task**: the user-facing unit of work in VS Code.
- **Lane**: the Trail branch-like ref backing a task.
- **Session**: the ACP and Trail conversation container.
- **Turn**: one user prompt and one agent response cycle.
- **Checkpoint**: recorded lane state after a turn.
- **Review**: diff, gates, transcript, changed paths, and apply plan.

Users should mostly see "task", "turn", "checkpoint", and "review". Lane and
ACP IDs should be visible in details, logs, and troubleshooting.

## Design Principles

1. Protocol-shaped, not provider-shaped.
   Render ACP content and update variants through a registry. Provider-specific
   fields are metadata, not top-level UI concepts.

2. Trail is the truth.
   The live stream can be dropped, replayed, or corrupted by a provider. The
   extension uses Trail for persisted history, checkpoints, review, readiness,
   and apply state.

3. The editor remains the control point.
   Permissions, cancellation, prompt composition, file opening, and review
   actions happen in VS Code. Trail records decisions but does not hide them.

4. Every task is reviewable before it touches the main worktree.
   Default to materialized lane workdirs and dry-run apply. Direct apply must be
   an explicit action.

5. Dense, calm, and operational.
   The UI is a work surface, not a landing page. It should prioritize scanning,
   comparison, and repeated action.

6. Progressive disclosure for noisy agent internals.
   Plans, file edits, approvals, and failed tools are prominent. Raw JSON,
   voluminous terminal output, and provider-specific details are collapsible.

7. Privacy first.
   Internal thought chunks are not persisted by default and should not be shown
   unless a provider explicitly exposes safe user-facing reasoning. Secrets,
   environment variables, and raw command args are redacted or hidden.

## UX Surface Map

The extension should contribute one view container, one webview panel, and a
small set of commands.

```text
Activity Bar
+-- Trail Agents
    +-- Tasks                 Tree view
    +-- Reviews               Tree view or filtered task view
    +-- Queue                 Tree view

Editor Area
+-- Agent Chat                Webview panel
    +-- Header
    +-- Transcript timeline
    +-- Turn input composer
    +-- Review drawer

Native Editors
+-- File diff editors
+-- Virtual lane file previews
+-- Terminal panes, when needed
```

Use Tree Views for lists because they fit VS Code conventions and scale to many
tasks. Use a webview for the chat because ACP content blocks, plans, tool
timelines, rich approval prompts, image/audio previews, and checkpoint rails
need richer layout than native Tree Items allow.

## Visual Direction

Subject: local operational memory for agent work.

Audience: developers repeatedly supervising autonomous code changes.

Single job: make every agent feel like the same supervised workflow, with the
current state and next safe action always visible.

### Visual System

Use VS Code theme variables as the base so the extension respects light, dark,
high contrast, and custom themes. Color should be restrained: neutral editor
surfaces do most of the work, semantic accents appear mainly as thin borders,
rail markers, and compact status text.

Semantic accents:

| Token | Hex fallback | Use |
| --- | --- | --- |
| `--trail-lane` | `var(--vscode-textLink-foreground, #4F8EA3)` | Active task, lane identity, current turn rail. |
| `--trail-checkpoint` | `var(--vscode-testing-iconPassed, #4D8D55)` | Recorded checkpoint, ready state, successful tool. |
| `--trail-review` | `var(--vscode-editorWarning-foreground, #9A6A23)` | Needs review, stale base, waiting state. |
| `--trail-risk` | `var(--vscode-errorForeground, #B0525C)` | Permission risk, blocked gate, destructive action. |
| `--trail-provider` | `var(--vscode-badge-background, #62628F)` | Provider badge, model identity. |
| `--trail-muted` | `var(--vscode-descriptionForeground)` | Metadata and secondary labels. |

Typography:

- Use VS Code's UI font for all controls and message chrome.
- Use VS Code's editor font for code, diffs, commands, paths, and raw payloads.
- Avoid viewport-scaled text. The webview should feel native beside the editor.

Signature element: the **checkpoint rail**.

Each turn has a vertical rail in the left gutter. User prompt, agent response,
tools, diff previews, approvals, and the final checkpoint attach to the same
rail. The rail makes it visually obvious which artifacts belong to the same
turn and whether the turn produced a durable Trail checkpoint.

```text
| user prompt
|
| agent response
|  +-- plan
|  +-- tool: search files
|  +-- tool: edit file
|  +-- diff: docs/integrations/acp.md
|
* checkpoint checkpoint_...  Ready for review
```

This is the one distinctive visual move. Everything else should be restrained:
thin dividers, compact rows, bordered labels instead of bright pills, stable
icon buttons, and native theme colors.

Interaction chrome should follow VS Code workbench conventions:

- Icon-only controls are transparent by default and use toolbar hover tokens.
- Primary actions use VS Code button tokens; destructive actions are outlined.
- Status, capability, provider, and attachment labels are bordered labels, not
  saturated pills.
- Touch-capable environments should retain at least 44px interactive targets
  without increasing desktop density.

## Primary Layout

### Task Tree

The sidebar task tree is for navigation and triage.

Groups:

- Running
- Waiting for permission
- Ready to review
- Blocked
- Applied
- Archived

Task row:

```text
<status icon> <task title>
  <provider> <lane> <changed paths count> <last activity>
```

Context actions:

- Open chat
- Open review
- Show diff
- Open lane workdir
- Rewind
- Archive

Do not use tree rows as hidden buttons. Selection opens the task details, while
explicit row actions perform mutations.

### Chat Header

The header is sticky and compact.

```text
Task title                         <provider> <mode> <context meter>
Lane: acp-claude-42  Base: main    [Review] [Apply dry-run] [Cancel]
```

Header responsibilities:

- Show provider, model, current mode, context usage, lane, base branch, and
  checkpoint status.
- Offer only the next useful commands.
- Display stale-base, dirty-workdir, and blocked-gate warnings inline.
- Use user-facing "session" language in the primary UI. ACP IDs remain
  available in review details, logs, and troubleshooting surfaces.

### Transcript Timeline

The timeline is append-only during a live turn and hydrated from Trail when a
task is reopened.

Default density:

- User prompts are visible in full unless long, then folded after a few lines.
- Assistant messages are visible in full with markdown rendering.
- Plans are compact checklists.
- Tool calls are one-line rows by default.
- Diffs show file path plus summary; full diff opens in a native VS Code diff
  editor or an expanded inline preview.
- Raw inputs and outputs are behind a "Details" disclosure.

### Input Composer

The composer sits at the bottom.

Controls:

- Provider selector.
- Mode selector, if the agent advertises session modes.
- Context attachments:
  - current file
  - selection
  - diagnostics
  - terminal output
  - changed files
  - Trail history or previous checkpoint
- Prompt field.
- Send and stop icon buttons with tooltips.

The composer must respect the agent's negotiated prompt capabilities. If an
agent does not support images, audio, or embedded context, those attachment
actions are disabled with a short reason.

### Review Drawer

The review drawer opens from the right side of the chat panel or as a separate
webview tab.

Sections:

- Summary: task title, provider, base, checkpoint, changed paths.
- Readiness: blockers, warnings, stale base, ignored paths, risky files.
- Tests and evals: last runs and actions to run again.
- Diffs: file list with additions/deletions and open-diff actions.
- Transcript: jump links to relevant turns.
- Actions: dry-run apply, queue merge, rewind, preserve failed attempt.

The review drawer should never duplicate the full transcript. It links back to
turns and files.

## Data Ownership

The extension has three state layers.

### `LiveAcpStore`

Ephemeral, in-memory state for the active ACP process.

Owns:

- request IDs
- streaming message chunks
- in-flight tool calls
- current permission request
- current mode/config update
- cancellation state

This store is disposable. Closing VS Code or losing the ACP process should not
destroy the durable task.

### `TrailStore`

Durable state loaded through Trail HTTP and CLI fallback.

Owns:

- tasks and lane mappings
- sessions and turns
- persisted messages and events
- checkpoints and change IDs
- diffs, reviews, readiness, merge queue state
- approvals and run state

This store is authoritative for task lists, reopen, resume, review, rewind, and
apply.

### `RenderStore`

A derived store that merges live ACP updates with durable Trail records.

Rules:

- Prefer Trail for completed turns.
- Prefer live ACP for the current in-flight turn.
- Correlate by `_meta.trail`, ACP session ID, Trail session ID, turn ID,
  message ID, tool call ID, and checkpoint change ID.
- Mark unpersisted live items as "streaming" or "pending checkpoint".
- Replace live provisional items when Trail confirms the durable record.

## Render Model

Use a renderer registry rather than hard-coded provider branches.

```ts
type RenderNode =
  | MessageNode
  | PlanNode
  | ToolNode
  | DiffNode
  | TerminalNode
  | ApprovalNode
  | CheckpointNode
  | UsageNode
  | ModeNode
  | ConfigNode
  | ResourceNode
  | UnknownNode;

interface RenderNodeBase {
  id: string;
  taskId: string;
  lane: string;
  turnId?: string;
  acpSessionId?: string;
  acpMessageId?: string;
  acpToolCallId?: string;
  provider?: string;
  source: "acp-live" | "trail" | "merged";
  status: "pending" | "in_progress" | "completed" | "failed" | "cancelled";
  createdAt?: string;
  updatedAt?: string;
  raw?: unknown;
}
```

Each ACP update becomes one or more render nodes. Each Trail event/message can
also become a render node. The registry decides how to display a node, not how
to persist it.

## ACP Update Mapping

### `user_message_chunk`

Render as a user message attached to the current turn rail.

Behavior:

- Aggregate by `messageId`.
- Render text as markdown.
- Render non-text content through the content block registry.
- When Trail has the persisted user prompt, replace the live aggregate.

### `agent_message_chunk`

Render as an assistant message.

Behavior:

- Aggregate by `messageId`.
- Keep the current streaming message anchored to the bottom unless the user has
  scrolled away.
- Render markdown incrementally, but debounce expensive syntax highlighting.
- Show a small streaming indicator in the message chrome, not inside the text.
- On prompt completion, wait for Trail confirmation and mark as checkpointed.

### `agent_thought_chunk`

Default behavior:

- Do not persist.
- Do not show in normal transcript.
- Show only a compact ephemeral "agent thinking" activity indicator if needed.

Optional developer mode:

- If enabled, show a redacted ephemeral panel labeled "Provider reasoning
  stream".
- Never include this panel in task export, review packets, or Trail records
  unless a future policy explicitly allows it.

### `plan`

Render as a complete-replacement plan block.

Behavior:

- Replace the visible plan entries on each update.
- Show statuses with icons and labels.
- Keep completed items visible but subdued.
- If the plan changes dramatically, keep only the latest plan in the main
  timeline and expose previous versions in details.

Plan row:

```text
[in progress] Inspect current ACP relay capture
[pending]      Add renderer registry
[completed]    Verify provider profile
```

### `tool_call`

Render as a tool row under the active turn.

Fields:

- title
- kind
- status
- locations
- content
- raw input/output, collapsed

Behavior:

- Choose icon by `ToolKind`: read, edit, delete, move, search, execute, think,
  fetch, switch_mode, other.
- Show affected file locations as clickable chips.
- Collapse successful low-risk tools by default.
- Expand failed, blocked, destructive, or approval-related tools by default.

### `tool_call_update`

Patch the existing `ToolNode` by `toolCallId`.

Behavior:

- Update status, title, locations, content, kind, raw input, and raw output.
- Preserve earlier content only when the protocol says the update is additive.
  If the field is defined as replacement, replace it.
- Move the row from in-progress to completed/failed on terminal status.

### `available_commands_update`

Do not render as transcript noise.

Behavior:

- Update a command menu in the composer.
- Show a brief header note only if commands appear or disappear while the user
  is actively composing.
- Store a summarized event through Trail if the relay already captures it.

### `current_mode_update`

Render in the header and mode selector.

Behavior:

- Update the selected mode.
- Add a subtle timeline row only when the mode change was initiated by the
  agent or materially changes permissions.

### `config_option_update`

Render in a configuration popover.

Behavior:

- Show changed config options in the header details.
- Do not spam the timeline.
- If an option affects prompt capabilities, update composer controls
  immediately.

### `session_info_update`

Render as task metadata.

Behavior:

- Use title updates to rename the task after user confirmation or with an
  undoable toast.
- Use timestamps to refresh task ordering.
- Preserve Trail identifiers as stable metadata even if the provider title
  changes.

### `usage_update`

Render as a context meter in the header.

Behavior:

- Show used/size and cost if available.
- Use color only for thresholds:
  - normal under 70 percent
  - review at 70 to 90 percent
  - risk over 90 percent
- Keep the full cost breakdown in a tooltip or details popover.

### `session/request_permission`

Render as a blocking approval panel.

Behavior:

- Pause the composer for that task.
- Show tool title, kind, affected files, command or edit summary, and risk.
- Show permission options exactly as protocol choices, with safer options first
  if the protocol order is not meaningful.
- Return the selected ACP response to the agent.
- Mirror the decision through Trail approval records.

Approval panel:

```text
Permission required
Run command in lane workdir

Command: cargo test -p trail
Scope: .trail/worktrees/acp-claude-42

[Allow once] [Reject] [Show details]
```

### Prompt completion response

Render a turn footer.

Behavior:

- Show stop reason.
- If completed, show pending checkpoint until Trail confirms the workdir
  record or structured patch.
- If cancelled, preserve partial transcript and show whether changes were
  recorded.
- If failed, show failure summary and recovery actions.

## Content Block Registry

### Text

Render markdown with:

- sanitized HTML disabled or strictly filtered
- fenced code blocks with language labels
- copy button
- open-in-editor action for large code blocks
- stable line wrapping

Do not run arbitrary scripts from markdown.

### Image

Render inline image previews with:

- max height
- click to open full preview
- alt/title from annotations or metadata when available
- file/resource URI link if present

If image data is too large, show a thumbnail and defer decoding until expanded.

### Audio

Render as a compact audio attachment.

Behavior:

- Show duration if known.
- Use the browser audio control only when allowed by webview policy.
- Provide "save/open externally" only when the resource is safe and local.
- If the provider also emits transcript text, link them.

### Resource Link

Render as a file/resource chip.

Behavior:

- `file://` and workspace-relative URIs open in VS Code.
- External URIs require confirmation before opening.
- Show name, title, mime type, and size when available.
- For unsupported URI schemes, show details and copy URI.

### Embedded Resource

Render as an expandable preview.

Text resources:

- syntax-highlight by mime type or file extension
- open as readonly virtual document
- show size and line count

Binary resources:

- image/audio preview if mime type is supported
- otherwise show metadata and copy/save actions

### Unknown Content

Render a compact unsupported-content row.

Behavior:

- Never crash the transcript.
- Show the discriminator, provider, and a copy-redacted-JSON action.
- Log enough detail for compatibility testing.

## Tool Content Registry

### Standard Content

Render using the content block registry.

### Diff

Render a diff node.

Behavior:

- Use `path`, `oldText`, and `newText` to build an inline summary.
- For small diffs, show a collapsed inline preview.
- For large diffs, show path, additions/deletions, and an "Open diff" action.
- Prefer native VS Code diff editors for full review.
- Correlate with Trail change IDs once persisted.

### Terminal

Render a terminal attachment.

Behavior:

- Show command, cwd, status, elapsed time, and exit code.
- Stream output in a fixed-height area with search and copy.
- Collapse successful terminal output after completion.
- Keep failed command output expanded.
- Offer "Open in Terminal" for long-running interactive commands.

## Trail Source of Truth Contract

The extension should use the daemon when available and CLI fallback when not.

Required reads:

- health and daemon auth
- task/session list
- lane status
- lane review
- lane readiness
- lane diff
- turn show
- events and spans
- approvals
- merge queue

Required mutations:

- create/start task through `trail agent acp run`
- cancel prompt through ACP
- approval decision through ACP and Trail mirror
- record/refresh lane workdir when explicitly requested
- run tests/evals
- dry-run apply
- queue merge
- rewind
- archive/remove task

The extension should not store durable transcripts in VS Code global state.
Global state may cache UI preferences, last selected provider, and recent task
IDs, but all task facts must reload from Trail.

## Reopen and Resume

When the user opens a task:

1. Load the task, lane, session, turns, events, and latest review from Trail.
2. Render persisted turns as completed nodes.
3. If an ACP session is still active, attach live stream state.
4. If the provider supports resume/load, offer "Resume".
5. If resume is unavailable, offer "Start follow-up from checkpoint".

The UI should distinguish:

- "resume same provider session"
- "start new turn from current lane checkpoint"
- "start same task with another provider"

Switching providers should be explicit because the new provider may not inherit
the old provider's private context. Trail can provide the durable transcript,
diffs, and checkpoint as context.

## Multi-Agent Coordination

The extension should make parallel work visible without making the chat noisy.

Task tree badges:

- active path claims
- stale base
- conflicting lane
- ready for merge
- blocked by approval

Chat warnings:

- show when another active task claims or edits the same path
- link to compare tasks
- suggest queueing or refreshing rather than directly applying

Review actions:

- compare two agent tasks
- show shared changed paths
- queue one task behind another
- open conflict set when Trail reports one
- render task-local buttons from `trail agent review-data --json`
  `actions[]`; use each action's stable id, label, safety class, enabled state,
  disabled reason, exact command, optional path/open path, MCP tool, and MCP
  arguments instead of parsing free-form suggestions or shell strings.

## Accessibility

Requirements:

- Full keyboard navigation through tasks, transcript landmarks, composer,
  approval options, and review actions.
- Visible focus rings using VS Code focus colors.
- Screen-reader labels for every icon button.
- No color-only status indicators. Pair color with icon and text.
- Respect reduced motion.
- Preserve scroll position when streaming unless the user is pinned to bottom.
- Announce blocking permission requests through VS Code notifications and
  webview ARIA live regions.
- Keep target sizes at least 28 CSS pixels in compact mode and 32 in default
  mode.

## Security

Webview:

- Use a strict content security policy.
- Load only extension-bundled scripts/styles through webview URIs.
- Sanitize markdown.
- Never execute scripts from agent content.
- Gate external links through VS Code `env.openExternal` confirmation.

Protocol:

- Redact raw environment variables and secret-like command args.
- Treat provider `_meta` as untrusted.
- Bound message size and rendering cost.
- Throttle streaming updates to avoid UI lockups.
- Keep thought chunks ephemeral by default.

Filesystem:

- Respect Trail ignore policy.
- Open lane files from materialized lane workdirs or virtual readonly docs.
- Require explicit action before applying lane changes to the main worktree.

## Implementation Architecture

Suggested TypeScript packages in the separate `trail-vscode` extension repository:

```text
trail-vscode/src/
  extension.ts
  acp/
    AcpClient.ts
    AcpProcess.ts
    AcpMessageRouter.ts
    ProviderRegistry.ts
  trail/
    TrailDaemonClient.ts
    TrailCliFallback.ts
    TaskRepository.ts
  state/
    LiveAcpStore.ts
    TrailStore.ts
    RenderStore.ts
    Correlator.ts
  views/
    TasksTreeProvider.ts
    ChatPanel.ts
    ReviewPanel.ts
  webview/
    app/
      components/
        Timeline.tsx
        MessageBlock.tsx
        PlanBlock.tsx
        ToolBlock.tsx
        DiffBlock.tsx
        ApprovalBlock.tsx
        Composer.tsx
        ReviewDrawer.tsx
      renderers/
        contentRegistry.ts
        updateRegistry.ts
        toolContentRegistry.ts
      styles/
        tokens.css
```

The extension host owns process management, daemon calls, native VS Code
commands, and file opening. The webview owns presentation and user gestures. All
mutations flow back through extension-host commands, not direct webview network
calls.

## Renderer Registry API

```ts
interface AcpUpdateRenderer<TUpdate> {
  match(update: unknown): update is TUpdate;
  reduce(update: TUpdate, context: RenderReduceContext): RenderPatch[];
}

interface ContentRenderer<TContent> {
  match(content: unknown): content is TContent;
  render(props: ContentRenderProps<TContent>): React.ReactNode;
}

interface RenderReduceContext {
  taskId: string;
  lane: string;
  acpSessionId: string;
  currentTurnId?: string;
  provider?: string;
  now(): string;
}
```

Benefits:

- New ACP variants can be added without rewriting the chat.
- Provider-specific extensions can register renderers behind feature flags.
- Unknown variants degrade to `UnknownNode`.

## Empty and Error States

No workspace:

```text
Open a folder to use Trail agents.
```

No Trail workspace:

```text
Initialize Trail to record agent tasks, checkpoints, and review state.
[Initialize workspace]
```

No provider:

```text
Add an ACP agent provider.
[Use Claude Code] [Add custom provider]
```

Daemon unavailable:

```text
Trail daemon is not running. The extension can start it for faster status and
review updates.
[Start daemon] [Use CLI fallback]
```

Provider crashed:

```text
The agent process exited before the turn completed. Partial transcript and
workdir changes remain in the task lane.
[Open review] [Start follow-up] [Show logs]
```

## Command Set

Command palette:

- `Trail: New Agent Task`
- `Trail: Open Agent Chat`
- `Trail: Open Latest Agent Review`
- `Trail: Apply Latest Agent Task Dry Run`
- `Trail: Queue Agent Task Merge`
- `Trail: Rewind Agent Task`
- `Trail: Compare Agent Tasks`
- `Trail: Start Daemon`
- `Trail: Doctor`
- `Trail: Add ACP Provider`

Inline editor commands:

- `Ask Agent About Selection`
- `Attach Selection to Agent Task`
- `Show Trail History for Line`
- `Show Agent Changes for File`

## Delivery Phases

### Phase 1: Read-Only Task Browser

Deliver:

- VS Code extension scaffold.
- Daemon discovery and CLI fallback.
- Task tree from Trail.
- Read-only transcript/review webview for existing ACP sessions.

Acceptance:

- A user can run an ACP task externally and inspect it in VS Code.
- The extension stores no durable transcript outside Trail.

### Phase 2: Native ACP Chat MVP

Deliver:

- Provider registry with `claude-code` and custom command.
- ACP stdio client over `trail agent acp run`.
- Chat panel with text messages, plan, tool rows, usage meter, and cancel.
- Basic permission prompt.

Acceptance:

- The user can start and complete a task from VS Code.
- Trail records the turn, transcript, tools, and checkpoint.
- Reopening VS Code hydrates the completed turn from Trail.

### Phase 3: Review and Apply

Deliver:

- Review drawer.
- Diff renderer and native diff editor integration.
- Readiness and gate display.
- `review-data` driven actions for open focus file, mark file reviewed, test
  plan, dry-run apply, apply, merge queue, and rewind.
- Rewind actions.

Acceptance:

- User can review and apply an agent task without leaving VS Code.
- Failed or unwanted work can be rewound from the UI.

### Phase 4: Full ACP Component Coverage

Deliver:

- Image, audio, resource, embedded resource renderers.
- Terminal renderer.
- Config and mode controls.
- Available commands menu.
- Unknown variant fallback and compatibility logging.

Acceptance:

- All ACP schema variants render or degrade gracefully.
- Providers with richer content do not break the unified UI.

### Phase 5: Multi-Agent Operations

Deliver:

- Parallel task dashboard.
- Path claim/conflict warnings.
- Compare tasks.
- Queue visualization.
- Provider switching from checkpoint.

Acceptance:

- Multiple providers can work concurrently in separate lanes.
- The user can compare, queue, apply, or rewind tasks without losing context.

## Test Plan

Unit tests:

- ACP update reducers for every `SessionUpdate` variant.
- Content block renderers for text, image, audio, resource link, embedded
  resource, and unknown content.
- Tool content renderers for standard content, diff, terminal, and unknown
  content.
- Correlation between ACP message IDs, tool call IDs, Trail turn IDs, and
  checkpoint IDs.
- Markdown sanitization.
- Permission response mapping.

Integration tests:

- Stub ACP provider streaming text, plan, tools, permission, diff, and prompt
  completion.
- Trail daemon fixture returning lanes, turns, events, diffs, and readiness.
- Reopen completed task from Trail only.
- Active turn live stream replaced by Trail persisted turn.
- Provider crash leaves recoverable task state.

Visual tests:

- Dark, light, and high contrast themes.
- Narrow sidebar and wide editor layouts.
- Long messages, long paths, long commands, and large diffs.
- Streaming transcript with user scrolled up.
- Permission prompt keyboard flow.

Manual compatibility tests:

- Claude Code through existing provider profile.
- One custom ACP command.
- One provider that emits direct diff content.
- One provider that edits only through materialized workdir capture.

## Open Questions

- Should the first extension release bundle an ACP TypeScript SDK dependency or
  keep a small local JSON-RPC client until SDK stability is proven?
- Which provider-specific metadata should be elevated into first-class UI?
- Should provider titles be allowed to rename Trail tasks automatically?
- Should terminal output live inside the webview, native VS Code terminals, or
  both?
- What is the right retention policy for raw ACP compatibility logs?
- How should remote ACP transports map to local Trail workspaces once the
  remote transport is stable?

## References

- ACP overview and JSON-RPC model: https://agentclientprotocol.com/protocol/v1/overview
- ACP prompt turn lifecycle: https://agentclientprotocol.com/protocol/v1/prompt-turn
- ACP schema and content/update variants: https://agentclientprotocol.com/protocol/v1/schema
- ACP extensibility and `_meta`: https://agentclientprotocol.com/protocol/v1/extensibility
- VS Code Webview API: https://code.visualstudio.com/api/extension-guides/webview
- VS Code view guidelines: https://code.visualstudio.com/api/ux-guidelines/views
- Trail ACP relay: [ACP relay design](./acp-relay.md)
- Trail HTTP routes: [HTTP API reference](../reference/http-api.md)

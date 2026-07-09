# Trail Agents for VS Code

Trail Agents is a VS Code extension that provides a unified Agent Client
Protocol chat and review workflow backed by Trail.

The extension treats ACP as the live agent protocol and Trail as the durable
source of truth for tasks, lanes, turns, checkpoints, diffs, readiness, and
recovery.

## Development

```sh
cd extensions/vscode
npm install
npm run compile
npm run check
```

Launch the extension from VS Code with an extension development host after
compiling.

For interactive debugging, open `extensions/vscode` as the VS Code workspace and
run **Run Trail Agents Extension** from the Run and Debug view.

## Current Production Slice

Implemented:

- Activity Bar container with task, review, and queue Tree Views.
- Contextual welcome states for no open folder, an uninitialized Trail
  workspace, empty task/review lists, and an empty merge queue, including a
  `Trail: Initialize Workspace` command.
- Task tree coordination badges derived from Trail readiness, including
  conflicts, pending approvals, queued merges, stale bases, dirty workdirs, and
  missing gates when the daemon is available.
- Command palette actions for workspace initialization, new task, open chat,
  refresh, doctor, daemon start, dry-run apply, rewind, preserve-and-rewind,
  remove task, lane test/eval, lane workdir opening, and editor-context entry
  points.
- Provider picking for new chats, with built-in Trail-backed Claude Code and
  configurable custom ACP commands.
- ACP JSON-RPC stdio client for `trail agent acp`.
- `initialize`, `session/new`, `session/prompt`, `session/cancel`, streamed
  `session/update`, and `session/request_permission` handling.
- Cancelling a turn also replies `cancelled` to any pending ACP permission
  requests and marks their approval cards as cancelled.
- ACP prompt capability negotiation from `initialize`, including conservative
  fallbacks for inline context, image, and audio support.
- ACP protocol version validation during `initialize`, with fail-fast errors
  when a provider negotiates an unsupported major version.
- ACP authentication handling for providers that advertise `authMethods` and
  return `auth_required` during session setup or active session requests. The
  chat flow prompts for the authentication method, calls `authenticate`, and
  retries the failed request once.
- Trail-backed chats preflight workspace initialization before launching the
  ACP relay, offering the `Trail: Initialize Workspace` command instead of
  surfacing a provider crash.
- Read-only ACP client filesystem support for `fs/read_text_file`, confined to
  the active workspace, including open editor buffers with unsaved changes, and
  bounded by line ranges/response size. Direct filesystem mutation and terminal
  execution remain disabled in the VS Code client surface.
- Multi-root VS Code workspaces are exposed through ACP
  `sessionCapabilities.additionalDirectories` when the provider advertises
  support, and `fs/read_text_file` enforces the same negotiated primary plus
  additional root set.
- ACP session controls for provider-advertised modes, session config options,
  and slash commands. Config options use `session/set_config_option`; modes use
  `session/set_mode`; slash commands insert protocol-standard `/command` prompt
  text.
- ACP session metadata updates surface provider-suggested titles in the chat
  header and review drawer without mutating the durable Trail task title.
- Reopen-aware ACP lifecycle handling: existing Trail ACP session IDs use
  `session/resume` or `session/load` only when the provider advertises support;
  otherwise the extension starts a checkpoint-based follow-up through Trail.
- Explicit provider switching inside the composer. Switching providers stops
  the active ACP process and starts the next prompt as a Trail checkpoint
  follow-up, so provider-private context is not assumed to transfer.
- Prompt completion footers that render ACP stop reasons (`end_turn`,
  `cancelled`, `max_tokens`, `max_turn_requests`, and `refusal`) separately
  from durable Trail checkpoints.
- Provider crash recovery state in the chat timeline. If the ACP process exits
  before a turn completes, the panel preserves the partial transcript, clears
  dead permission prompts, offers review/log actions, and starts the next
  prompt as a checkpoint follow-up. Exit details include code or signal plus
  recent redacted ACP stderr.
- Trail daemon discovery with daemon-first reads/mutations for stable HTTP
  routes, including lanes, review, diff, queue merge, rewind, lane test/eval
  gates, and lane workdir lookup; high-level agent transcript views retain CLI
  fallback until a matching HTTP task view is available.
- Provider-neutral render model and ACP update reducer registry.
- Streaming user, assistant, and thought chunks aggregate by ACP message id
  without storing provider-specific transcript state.
- Webview chat surface with checkpoint rail, message blocks, plans, tools,
  diffs, terminal/resource renderers, approvals, usage meter, composer, and
  review drawer.
- Keyboard shortcuts for sending and navigation: Enter sends from the composer,
  Shift+Enter inserts a newline, Ctrl/Cmd+Enter also sends, Escape closes
  drawers, and `Alt+1`, `Alt+2`, `Alt+3` focus transcript, composer, and
  review. Icon controls remain accessible for repeated chat, attachment,
  preview, and drawer actions.
- Approval panels normalize ACP permission requests into action, provider, scope,
  affected locations, option descriptions, bounded details, and redacted raw
  payloads.
- Text, diff, embedded-resource, and redacted raw-JSON previews share a
  bounded code viewer with copy and open-in-editor actions for compatibility
  testing and deeper inspection.
- Image/audio content and supported embedded binary resources render as bounded
  previews with a full-preview drawer, while oversized media degrades to
  redacted details.
- Terminal renderers show command, cwd, status, exit code, elapsed time, and
  bounded stdout/stderr/output previews with filtering, copy, and a safe
  open-terminal action for follow-up inspection.
- Current selection, current file, diagnostics, latest terminal output,
  changed-file list, and Trail history attachments sent as ACP content blocks
  from either editor commands or the chat composer.
- Attachment transport fallback when the agent does not support embedded
  context.
- Composer draft and transcript scroll position survive live streaming updates.
- Clickable tool locations, changed paths, and ACP resource links. Workspace
  files open directly; outside-workspace files and external URLs require
  confirmation from the extension host.
- Native VS Code diff documents for ACP diff content.
- Rehydration of persisted Trail transcript turns into the chat timeline.
- Review drawer with task summary, readiness, tests/evals, diff links,
  transcript jump links, Trail coordination signals, lane test/eval execution,
  lane workdir opening, task comparison, dry-run apply, queue merge, and rewind
  actions, plus preserve-and-rewind and confirmed task removal.
- Chat and review warnings for parallel agent tasks that change the same paths,
  with shared-path chips plus compare, refresh, and queue actions.
- In-chat compare dashboard for `agent compare` results, showing left/right
  task risk, shared changed paths, side-only changes, recommendation, next
  commands, and redacted raw details without leaving the chat review workflow.
- Conflict detail drawer for Trail conflict sets reported by readiness/review,
  showing source and target refs, affected paths, class/reason summaries,
  recommendations, next steps, and redacted raw details without leaving the chat
  review workflow.
- Daemon-enriched review/readiness data is merged into the review drawer when
  available, while the persisted transcript still reloads from Trail.
- Queue Tree View backed by Trail merge-queue entries, grouped by status with
  source, target, priority, explain, run, and remove actions.
- Redaction for sensitive keys, bearer tokens, CLI flags, raw ACP details, and
  Trail command output before they are shown in the webview or output channel.
- Bounded raw JSON, text, image, audio, and embedded-resource previews so large
  provider payloads degrade instead of locking the webview.
- Unit coverage for render reducers, hydration, attachment conversion,
  changed-file/history context attachments, ACP
  capability parsing, streamed chunk aggregation, session lifecycle
  negotiation, startup and active auth-required retry, exit signal/stderr
  diagnostics, permission cancellation, multi-root
  `additionalDirectories`, read-only ACP filesystem requests, session controls,
  prompt completion stop reasons, resource target safety, merge queue normalization,
  daemon discovery/auth requests,
  coordination summary
  normalization, daemon DELETE requests, shell command wrapping, redaction, and
  the JSON-RPC ACP client against a stub stdio agent.
- Extension-host smoke coverage for activation, commands, configuration, and a
  disposable `trail agent acp` ChatPanel run where a stub ACP provider modifies
  a materialized lane workdir and Trail records the transcript/checkpoint.
- Extension-host smoke coverage for `session/request_permission`: a stub ACP
  provider pauses on approval, the ChatPanel approves it, the provider writes to
  a materialized lane, and Trail records the approved transcript/checkpoint.
- CLI compatibility fallback for `Open diff`: when an older Trail CLI lacks
  `agent diff`, the extension falls back to `agent view` rather than dropping
  the review action.
- Strict CSP without inline styles, sanitized text rendering, throttled webview
  state posting, and no durable transcript storage in VS Code state.

Still planned:

- Full daemon-backed agent task HTTP routes when Trail exposes them.
- Real-provider smoke tests through `trail agent acp` once credentials and
  provider CLIs are available in CI.

## Architecture

- The extension host launches `trail agent acp --provider <provider>` and
  speaks ACP JSON-RPC over stdio.
- The extension host reads persisted task state through Trail CLI/daemon
  helpers.
- The webview renders ACP content through protocol-shaped renderer registries.
- Durable transcript and review state remain in Trail, not VS Code global
  storage.

## Custom Providers

Add custom ACP commands in `trail.customProviders`. To keep Trail as the
source of truth, configure the command as a Trail relay entrypoint, for
example:

```json
{
  "trail.customProviders": [
    {
      "id": "custom-acp",
      "label": "Custom ACP via Trail",
      "command": "${trailPath}",
      "args": [
        "--workspace",
        "${workspaceFolder}",
        "agent",
        "acp",
        "--provider",
        "claude-code"
      ]
    }
  ]
}
```

Raw ACP commands are allowed for experimentation, but the extension warns when a
provider is not recognized as Trail-backed because task history, checkpoints,
and review state may not be durable.

## Verification

```sh
npm run check
npm run compile
npm test
npm run test:extension
npm run package:vsix
```

Additional local ACP smoke checks used for this slice:

```sh
trail --workspace <temp-trail-workspace> acp doctor --agent claude-code --json
trail --workspace <temp-trail-workspace> acp doctor --agent fake --json
```

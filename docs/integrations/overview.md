# Integration Overview

Trail exposes five integration surfaces:

- CLI: primary human and scripting interface.
- HTTP daemon: JSON API for editor and automation integrations.
- MCP server: stdio server with tools, resources, prompts, and completions for agent hosts.
- ACP relay: stdio relay in front of ACP-capable coding agents.
- Rust library: public `Trail` API and exported model/report types.

Git interop is also available through the CLI and library.

The [ACP integration guide](./acp.md) and
[ACP Agent Usage Runbook](./acp-agent-usage.md) show how Trail sits between
ACP-capable editors and real coding agents while mirroring sessions, turns,
prompts, tool events, and edits into Trail.
The [VS Code ACP chat view design](../design/vscode-acp-chat-view.md) describes
how a VS Code extension can render ACP chat components while treating Trail as
the durable source of truth for tasks, turns, checkpoints, review, and recovery.
The initial extension implementation lives in
[`crabbuild/trail-vscode`](https://github.com/crabbuild/trail-vscode).

## Choose a Surface

Use the CLI when:

- A human is driving local workflows.
- A script can shell out.
- You want human-readable output by default.

Use the daemon when:

- Repeated status/diff/record calls need a warmed worktree cache.
- An editor or local service wants HTTP JSON.
- CLI hot commands should route to a long-running process.

Use MCP when:

- An agent host supports MCP tools and resources.
- You need guided prompts for agent tasks, review, or conflict resolution.

Use the ACP relay when:

- An editor speaks Agent Client Protocol.
- You want the editor to keep its normal agent UX while Trail records the run.
- The real coding agent should receive Trail MCP tools automatically.

Use the Rust library when:

- You are embedding Trail in another Rust process.
- You need direct typed access to `Trail` methods and reports.

## Code Facts Used

- CLI entrypoint: `trail/src/cli`
- HTTP server: `trail/src/server`
- MCP server: `trail/src/mcp`
- ACP usage: `docs/integrations/acp-agent-usage.md`
- Library exports: `trail/src/lib.rs`
- ACP relay proposal: `docs/design/acp-relay.md`
- VS Code ACP chat view: `docs/design/vscode-acp-chat-view.md`
- VS Code extension implementation: `https://github.com/crabbuild/trail-vscode`

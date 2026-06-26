# Integration Overview

CrabDB exposes four integration surfaces:

- CLI: primary human and scripting interface.
- HTTP daemon: JSON API for editor and automation integrations.
- MCP server: stdio server with tools, resources, prompts, and completions for agent hosts.
- Rust library: public `CrabDb` API and exported model/report types.

Git interop is also available through the CLI and library.

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

Use the Rust library when:

- You are embedding CrabDB in another Rust process.
- You need direct typed access to `CrabDb` methods and reports.

## Code Facts Used

- CLI entrypoint: `crates/crabdb/src/cli`
- HTTP server: `crates/crabdb/src/server`
- MCP server: `crates/crabdb/src/mcp`
- Library exports: `crates/crabdb/src/lib.rs`


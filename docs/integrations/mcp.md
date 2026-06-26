# MCP

CrabDB provides an MCP stdio server for agent hosts.

## Start

```sh
crabdb mcp
```

The server implements JSON-RPC methods for tools, resources, resource templates, prompts, and completion.

## Tool Groups

MCP tools mirror the main workflows:

- Core workspace and provenance tools.
- Agent branch and review tools.
- Session, approval, run, lease, and anchor tools.
- Merge queue and conflict tools.
- Turn, event, span, patch, test, eval, read, and sync tools.

See [MCP tools reference](../reference/mcp-tools.md) for the complete list.

## Resources

Static resources include status, doctor, agents, merge queue, conflicts, OpenAPI, and the three compatibility docs.

Resource templates expose agent details, status, review packets, contribution, gates, readiness, handoff, diff, sessions, turns, conflicts, approvals, run states, and trace spans.

## Prompts

Built-in prompts:

- `crabdb.agent_task`
- `crabdb.review_agent`
- `crabdb.resolve_conflict`

These guide hosts through safe agent tasks, review, and conflict resolution.

## Code Facts Used

- MCP server: `crates/crabdb/src/mcp`
- Tools: `crates/crabdb/src/mcp/tools`
- Resources: `crates/crabdb/src/mcp/capabilities/resources.rs`
- Prompts: `crates/crabdb/src/mcp/capabilities/prompts.rs`
- Tests: `mcp_stdio_tools_drive_agent_turn_workflow`, `local_api_and_mcp_drive_merge_queue_and_conflicts`

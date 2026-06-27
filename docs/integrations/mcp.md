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
- Lane branch and review tools.
- Session, approval, run, lease, and anchor tools.
- Merge queue and conflict tools.
- Turn, event, span, patch, test, eval, read, and sync tools.
- Rewind tools for returning a lane branch to a known-good root without losing
  the failed attempt.

See [MCP tools reference](../reference/mcp-tools.md) for the complete list.

## Resources

Static resources include status, doctor, agents, merge queue, conflicts, OpenAPI, and the three compatibility docs.

Resource templates expose agent details, status, review packets, contribution, gates, readiness, handoff, diff, sessions, turns, conflicts, approvals, run states, and trace spans.

## Prompts

Built-in prompts:

- `crabdb.lane_task`
- `crabdb.review_lane`
- `crabdb.resolve_conflict`

These guide hosts through safe agent tasks, review, and conflict resolution.

## Host Capture Contract

CrabDB cannot capture a coding agent transcript unless the host calls the MCP
tools. Hosts such as Codex, Claude, Cursor, or custom runners should wrap each
task with:

```text
crabdb.begin_turn -> crabdb.add_message -> crabdb.span_start/span_end or crabdb.add_event -> crabdb.apply_patch or crabdb.sync_workdir -> crabdb.end_turn
```

When a run pauses for approval or interruption, use `crabdb.run_pause` and later
`crabdb.run_resume`. If a branch goes sideways, use `crabdb.lane_rewind` with
`record_current=true` to preserve the failed head before returning to a
known-good state.

## Code Facts Used

- MCP server: `crates/crabdb/src/mcp`
- Tools: `crates/crabdb/src/mcp/tools`
- Resources: `crates/crabdb/src/mcp/capabilities/resources.rs`
- Prompts: `crates/crabdb/src/mcp/capabilities/prompts.rs`
- Tests: `mcp_stdio_tools_drive_lane_turn_workflow`, `local_api_and_mcp_drive_merge_queue_and_conflicts`

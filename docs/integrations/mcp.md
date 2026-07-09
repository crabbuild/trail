# MCP

Trail provides an MCP stdio server for agent hosts.

## Start

```sh
trail mcp
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

Static resources include status, doctor, agents, latest agent review-data, merge queue, conflicts, OpenAPI, and the three compatibility docs.

Resource templates expose agent details, review-data packets, status, review packets, contribution, gates, readiness, handoff, diff, sessions, turns, conflicts, approvals, run states, and trace spans.

## Prompts

Built-in prompts:

- `trail.lane_task`
- `trail.review_lane`
- `trail.resolve_conflict`

These guide hosts through safe agent tasks, review, and conflict resolution.

## Host Capture Contract

Trail cannot capture a coding agent transcript unless the host calls the MCP
tools. Hosts such as Codex, Claude, Cursor, or custom runners should wrap each
task with:

```text
trail.begin_turn -> trail.add_message -> trail.span_start/span_end or trail.add_event -> trail.apply_patch or trail.sync_workdir -> trail.end_turn
```

When a run pauses for approval or interruption, use `trail.run_pause` and later
`trail.run_resume`. If a branch goes sideways, use `trail.lane_rewind` with
`record_current=true` to preserve the failed head before returning to a
known-good state.

## Code Facts Used

- MCP server: `trail/src/mcp`
- Tools: `trail/src/mcp/tools`
- Resources: `trail/src/mcp/capabilities/resources.rs`
- Prompts: `trail/src/mcp/capabilities/prompts.rs`
- Tests: `mcp_stdio_tools_drive_lane_turn_workflow`, `local_api_and_mcp_drive_merge_queue_and_conflicts`

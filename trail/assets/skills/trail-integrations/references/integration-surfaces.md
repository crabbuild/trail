# Integration Surfaces

## Structured CLI

Use global flags before the command and pin the workspace:

```sh
trail --workspace <root> --format json status
trail --workspace <root> --format ndjson timeline --limit 20
```

Parse structured reports and stable error codes, not human terminal output.

## MCP

Start the stdio server with `trail mcp`. Prefer high-level `trail.agent_*` tools for managed tasks and low-level lane tools for direct lane control. Honor annotations that distinguish read-only, workspace-write, destructive, and open-world tools.

A host that wants durable causal capture must wrap real activity:

```text
trail.begin_turn
  -> trail.add_message
  -> trail.span_start/span_end or trail.add_event
  -> trail.apply_patch or trail.sync_workdir
  -> trail.add_message
  -> trail.end_turn
```

Use run pause/resume only for real interruptions.

## ACP and Native Hooks

Use `trail agent acp setup <provider> --editor <editor>` for editor configuration and `trail acp relay <provider>` for the relay. Use `trail agent start <provider>` for terminal-first tasks. Do not nest a relay or terminal task inside an already managed Trail task.

Native hook installation is provider-specific and must preserve unrelated provider configuration. Inspect the plan/status first and install only with the user's requested scope.

## HTTP Daemon

`trail daemon` defaults to loopback and token authentication. Pass bearer auth or `x-trail-token`; never log `.trail/daemon.token`. Keep host/origin checks and authentication enabled. Use `Idempotency-Key` for retried mutating requests and bound all bodies, frames, subprocess output, and pagination.

None of these surfaces implicitly publishes Git history. Use Trail readiness and preview reports before requesting a shared-ref merge or Git application.

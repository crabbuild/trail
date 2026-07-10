# Integrations

Choose the narrowest interface that fits the host.

## CLI Automation

Use human output interactively and JSON for machines:

```sh
trail --workspace <root> --json status
trail --workspace <root> --json agent review-data <task>
```

Do not parse terminal tables. Global `--json`, `--format json`, or `TRAIL_FORMAT=json` also produces structured errors. Use explicit workspace and task/lane selectors in unattended scripts.

## MCP

Start the stdio server with:

```sh
trail mcp
```

When Trail MCP tools are already registered, prefer them to shell parsing. Prefer high-level `trail.agent_*` tools for normal task UX and low-level lane tools only when direct control is required. Start ambiguous user questions with the read-only `trail.agent_ask`, `trail.agent_guide`, `trail.agent_dashboard`, or `trail.agent_review_data` tools.

Honor tool risk annotations:

- Read-only: status, reports, diff, readiness, diagnosis, resources.
- Workspace write: review markers and archive metadata.
- Destructive write: apply/finish, undo/rewind, merge-queue execution.
- Open-world write: test/eval commands.

Require confirmation appropriate to the risk. For non-dry-run apply/finish, call readiness and dry-run first.

### Host Capture Contract

Trail cannot infer a transcript from an unrelated host. A host that wants durable causal capture must wrap real activity:

```text
trail.begin_turn
  -> trail.add_message (actual user message)
  -> trail.span_start/span_end or trail.add_event (actual tool activity)
  -> trail.apply_patch or trail.sync_workdir (actual edits)
  -> trail.add_message (actual assistant result)
  -> trail.end_turn
```

Use `trail.run_pause`/`trail.run_resume` across real interruptions. Never fabricate messages, tool calls, gates, or approvals.

## ACP Relay and Terminal Providers

Use ACP when an editor should keep its normal agent UX while Trail records turns, events, edits, and checkpoints and injects Trail MCP tools:

```sh
trail agent setup --provider codex --editor vscode
trail acp relay codex
```

Use `trail agent start --provider <name>` for terminal-first agents. ACP relay is the richer streaming-capture path; terminal tasks universally isolate work and record the final checkpoint.

## HTTP Daemon

Use the daemon for local editors/services or warmed repeated status/diff/record operations:

```sh
trail daemon
```

It defaults to `127.0.0.1:8765`, writes `.trail/daemon.json`, and uses a token stored in `.trail/daemon.token` unless configured otherwise. Pass bearer auth or `x-trail-token`. Keep authentication enabled; `--no-auth` is loopback-only but still exposes mutation authority to every local process.

Route supported CLI hot commands with:

```sh
trail --daemon-url http://127.0.0.1:8765 --daemon-token "$TOKEN" status
```

Use `Idempotency-Key` for retried mutating HTTP requests. Do not log tokens or raw secret-bearing bodies.

## Git Boundary

MCP, ACP, HTTP, and CLI share the same Trail core, but none changes the product boundary: Trail stores local task/operation history; Git remains shared publication history. Prefer Trail review/readiness and dry-run reports before any integration requests a Git apply or shared-ref merge.

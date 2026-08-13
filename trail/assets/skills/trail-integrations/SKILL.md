---
name: trail-integrations
description: Connect agent hosts, editors, and local automation to Trail through MCP, ACP relay, native agent hooks, the authenticated HTTP daemon, OpenAPI, or structured CLI output. Use when configuring or implementing a Trail integration; selecting a capture surface; registering Trail tools with an agent; preserving real turns, messages, events, patches, and gates; or debugging integration framing, authentication, idempotency, and risk annotations.
---

# Trail Integrations

Choose the narrowest interface that fits the host. CLI, MCP, ACP, hooks, and HTTP share Trail's typed core, but capture semantics and trust boundaries differ.

## Select a Surface

- Use structured CLI output for scripts and one-shot local automation.
- Use MCP when the host needs Trail tools, resources, prompts, and typed risk annotations.
- Use ACP relay when an ACP editor should retain its normal UX while Trail captures streaming turns and checkpoints.
- Use native hooks when a provider exposes lifecycle callbacks but not ACP.
- Use the HTTP daemon for local editor/service integration or repeated warmed operations.

Read [integration-surfaces.md](references/integration-surfaces.md) before configuring capture, authentication, or mutations.

## Preserve Truthful Capture

Record only events the host actually observed. Never fabricate user messages, tool calls, approvals, gates, or assistant output. Use explicit workspace, lane, session, turn, and correlation IDs. Prefer JSON or typed tools over parsing terminal tables.

Keep daemon authentication enabled, redact tokens and secret-bearing payloads, use idempotency keys for retried mutations, and honor MCP risk annotations. Read-only inspection does not authorize open-world commands or destructive writes.

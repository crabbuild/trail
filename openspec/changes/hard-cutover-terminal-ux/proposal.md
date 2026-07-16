## Why

Trail exposes rich operational state, but its human output is inconsistent, metadata-first, and difficult to scan across status, review, readiness, merge, and recovery workflows. A hard cutover to one outcome-first terminal language will make the CLI feel trustworthy and immediately understandable while preserving structured machine output as a separate surface.

## What Changes

- **BREAKING**: Replace all existing human-readable command output with one new rendering contract; no legacy human style, compatibility switch, or deprecation period is provided.
- Introduce outcome-first layouts that present the current state, the important evidence, attention items, and one safest next action in that order.
- Introduce a shared semantic renderer for statuses, sections, key/value metadata, borderless tables, path changes, diagnostics, command hints, truncation, color, terminal width, and verbosity.
- Replace raw debug enum names and routine internal identifiers with stable user-facing language; keep full technical metadata available in verbose and structured output.
- Redesign status, record, timeline, diff, history, readiness, guardrail, doctor, approval, conflict, merge, lane, and agent-task views around the new grammar.
- Add responsive wide and narrow layouts, terminal-aware color, redirected-output behavior, safe text sanitization, and paging for long interactive content.
- Make failures actionable by pairing stable diagnostic codes with cause, consequence, and an exact safe recovery command.
- Define explicit human, plain, JSON, and NDJSON behavior. JSON/NDJSON report schemas and stable exit/error codes remain machine contracts, but human and plain rendering adopt the new style immediately.
- Add golden and semantic rendering tests across terminal widths, color policies, verbosity levels, empty states, large data sets, and redirected output.

## Capabilities

### New Capabilities

- `terminal-output-ux`: Defines Trail's unified human/plain terminal rendering contract, responsive presentation, diff and table behavior, actionable diagnostics, and command-specific UX expectations.

### Modified Capabilities

None. This repository has no existing OpenSpec capability specifications; the new capability establishes the terminal output contract.

## Impact

- Affects the CLI argument/runtime context, all modules under `trail/src/cli/command/render`, human error rendering, and selected report models where the renderer lacks data needed for summaries or recovery guidance.
- Replaces direct command-level `println!`/`eprintln!` formatting with shared semantic rendering primitives and buffered writers.
- Changes snapshots, README examples, CLI reference documentation, and downstream consumers that parse human output; those consumers must move to JSON or NDJSON.
- May add small terminal presentation dependencies for display width, wrapping, styling, and paging, subject to Trail's Rust and platform support constraints.
- Does not change Trail's storage format, operational semantics, HTTP/MCP contracts, or JSON/NDJSON report meaning unless a separately documented model addition is required to render missing evidence.

# Trail ACP v1 Compatibility

Trail is a complete, transparent compatibility layer between a local ACP v1
client/editor and a local ACP v1 agent over newline-delimited UTF-8 JSON-RPC
stdio. The relay preserves valid protocol behavior while adding Trail lane
isolation, durable capture, provenance, and optional Trail MCP injection.

## Normative contract

Trail pins ACP wire version `1` from the official Agent Client Protocol schema:

- upstream revision: `64cbd71ae520b89aac54164d8c1d364333c8ee5f`
- `schema/v1/schema.json` SHA-256:
  `92c1dfcda10dd47e99127500a3763da2b471f9ac61e12b9bf0430c32cf953796`
- `schema/v1/meta.json` SHA-256:
  `e0bf36f8123b2544b499174197fdc371ec49a1b4572a35114513d56492741599`
- stable methods: all 23 methods enumerated by that metadata
- transport: stdio

The offline conformance gate covers every stable method, request and response
direction, success and error response, notification, content/update union,
capability combination, string and numeric ID, concurrent correlation case,
unknown extension, and intentional transformation. Fault tests cover malformed
input, the 16 MiB frame boundary, backpressure, cancellation races, peer
termination, capture failure, and durable spill recovery. Interoperability is
tested both with a peer compiled from the official Rust ACP types and an
independent Python peer. A scheduled drift check compares the vendored v1
artifacts with the official repository.

## What the user gets

- An ACP-capable editor can launch Trail as its agent command; Trail can launch
  any conformant local ACP v1 agent.
- Editor-to-agent and agent-to-editor requests remain correlated even when IDs
  overlap or responses arrive out of order.
- Prompts, streamed output, tool activity, permissions, filesystem callbacks,
  terminal callbacks, cancellation, errors, and extensions remain usable.
- Agent work can run in an isolated Trail lane. Paths inside the workspace are
  mapped into the materialized lane; additional roots outside the workspace are
  preserved instead of being silently copied or rewritten.
- Capture runs off the forwarding path. Queue pressure spills to a durable
  journal, and capture degradation is surfaced without changing wire traffic.
- Trail-owned MCP identity is injected only when requested and is deduplicated
  from an equivalent editor-provided server.
- `trail transcript` and the lane/session views provide durable activity and
  provenance without making the editor or agent Trail-specific.

## Read the compatibility evidence

Run the doctor in a Trail workspace:

```sh
trail agent acp doctor codex
trail --json agent acp doctor codex
```

The report separates these facts:

- `conformance`: wire version, schema revision and digests, stdio transport,
  method count, build identifier, evidence status, and exclusions;
- `provider`, `relay`, and `launch`: whether the selected agent can be launched
  on this machine;
- `capture_journal`: whether the durable ingress journal is accessible;
- `path_mapping`: whether workspace isolation and preserved external roots pass
  the relay's mapping check;
- `capture_smoke`: intentionally skipped until a real editor/agent prompt has
  supplied environment-specific evidence.

An ordinary local build reports `evidence_status: "unverified"` and never prints
`ACP v1 conformant`. This prevents a modified build from inheriting a release
claim merely because its provider command is available. A release builder may
attest a tested source revision by compiling with both values equal:

```sh
revision=$(git rev-parse HEAD)
TRAIL_SOURCE_REVISION="$revision" \
TRAIL_ACP_V1_CONFORMANCE_VERIFIED="$revision" \
  cargo build --release -p trail
```

Those variables are a build attestation, not a substitute for running the
release gates. `verified` means the named source revision was built after the
schema, conformance, fault, interoperability, and benchmark gates passed. It
does not mean the selected external provider is installed, authenticated, or
healthy.

## Exact boundary

Protocol conformance is independent of setup convenience. The generic, VS Code,
and Zed setup commands only produce or install editor configuration; a missing
adapter does not reduce Trail's wire coverage. Similarly, Trail can diagnose
provider readiness but does not install user credentials, accept third-party
terms, repair a non-conformant peer, or guarantee an external service account.

Capture fidelity means every parsed ACP frame receives ordered durable evidence
and all stable ACP session activity is projected into Trail's typed views. The
wire remains authoritative: capture timeout, worker panic, journal pressure, or
projection failure cannot rewrite a forwarded frame. A later projection can be
recovered from the journal.

Lane isolation applies to workspace-owned roots selected for materialization.
Absolute roots outside the workspace remain external and retain their original
paths, as ACP clients expect. Trail does not claim to sandbox those external
roots or the agent process itself.

The compatibility claim explicitly excludes:

- ACP v2 semantics;
- the draft remote HTTP transport;
- an editor UI or guarantees about an editor's settings format;
- installation, licensing, authentication, uptime, or correctness of an
  external agent.

## Opt-in real-editor smoke evidence

Real programs are environment evidence, not part of the deterministic protocol
gate. If a required executable is absent, record the smoke as `skipped`; never
count the skip as passing conformance evidence.

### Zed with Codex

```sh
command -v zed >/dev/null || { echo "SKIP: Zed unavailable"; exit 0; }
command -v npx >/dev/null || { echo "SKIP: Codex ACP adapter unavailable"; exit 0; }
trail agent acp doctor codex
trail agent acp setup codex --editor zed --yes
zed .
```

In Zed, start the configured Trail/Codex agent, send a prompt that reads and
writes one workspace file, cancel a second prompt, then verify
`trail agent acp sessions` and `trail transcript <session-id>`.

### Zed with Claude Code (independent agent)

```sh
command -v zed >/dev/null || { echo "SKIP: Zed unavailable"; exit 0; }
command -v npx >/dev/null || { echo "SKIP: Claude ACP adapter unavailable"; exit 0; }
trail agent acp doctor claude-code
trail agent acp setup claude-code --editor zed --yes
zed .
```

Send a prompt that exercises a permission request and a file edit, then inspect
the session and transcript as above. Authentication failure is provider evidence
and must not be reported as a Trail protocol failure.

### Zed with Cursor's native ACP agent (second independent implementation)

```sh
command -v zed >/dev/null || { echo "SKIP: Zed unavailable"; exit 0; }
command -v agent >/dev/null || { echo "SKIP: Cursor agent unavailable"; exit 0; }
trail agent acp doctor cursor
trail agent acp setup cursor --editor zed --yes
zed .
```

Send one prompt that uses a terminal and one that reads an additional external
directory. Confirm both editor behavior and Trail's captured session. Record the
program versions, doctor JSON, prompt result, and transcript selector with the
release evidence.

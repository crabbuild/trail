# ACP v1 Full Conformance Design

Status: approved for implementation planning on 2026-07-12.

## Objective

Make Trail a complete, transparent, and verifiably conformant proxy between any
ACP v1 client/editor and any ACP v1 agent over the stable stdio transport. Trail
must preserve all valid ACP v1 behavior while adding lane isolation, durable
capture, and Trail MCP injection. Compatibility is a release gate, not a best-
effort claim.

## Normative Baseline

The implementation targets ACP wire protocol version `1` as defined by the
official Agent Client Protocol repository. The initial pinned conformance
baseline is upstream commit
`64cbd71ae520b89aac54164d8c1d364333c8ee5f`:

- `schema/v1/schema.json`, SHA-256
  `92c1dfcda10dd47e99127500a3763da2b471f9ac61e12b9bf0430c32cf953796`
- `schema/v1/meta.json`, SHA-256
  `e0bf36f8123b2544b499174197fdc371ec49a1b4572a35114513d56492741599`

The source of record is
<https://github.com/agentclientprotocol/agent-client-protocol>. Trail will
vendor the two v1 artifacts and their source metadata for deterministic,
offline tests. A separate drift check will compare the vendored artifacts with
the latest official stable v1 artifacts. Any upstream stable-v1 change must
fail that check until Trail's fixtures, implementation, and compatibility
evidence are deliberately updated.

ACP schema package versions and ACP wire versions are different. Trail's public
compatibility statement will name wire protocol version `1` and the pinned
schema artifact revision.

## Meaning of Full Compatibility

Trail may claim full ACP v1 compatibility only when all of the following are
true:

1. Every valid ACP v1 request, response, notification, capability combination,
   content variant, session update, error, and extension passes through Trail
   with ACP-defined behavior.
2. All 23 stable methods in the official v1 `meta.json` have executable
   conformance coverage in the correct direction:
   - `initialize`
   - `authenticate`
   - `logout`
   - `session/new`
   - `session/load`
   - `session/resume`
   - `session/close`
   - `session/list`
   - `session/delete`
   - `session/prompt`
   - `session/cancel`
   - `session/set_mode`
   - `session/set_config_option`
   - `session/update`
   - `session/request_permission`
   - `fs/read_text_file`
   - `fs/write_text_file`
   - `terminal/create`
   - `terminal/output`
   - `terminal/wait_for_exit`
   - `terminal/kill`
   - `terminal/release`
   - `$/cancel_request`
3. Trail preserves request IDs, direction-specific ID scopes, response
   correlation, notification ordering, error objects, `_meta`, and unknown
   extension data.
4. Trail's intentional transformations are enumerated, atomic, capability-
   aware, and validated against the official v1 schema before forwarding.
5. Multiple sessions and multiple in-flight bidirectional requests on one ACP
   connection work without cross-session or cross-direction correlation.
6. Trail-specific capture is causally complete for ACP-visible activity and
   cannot silently change wire behavior.
7. The conformance suite passes on every supported Trail platform.

Passing only a prompt smoke test, forwarding unknown JSON, or passing the
existing ACP-focused tests is insufficient evidence for this claim.

## Scope

### Included

- Stable ACP v1 over newline-delimited UTF-8 JSON-RPC stdio.
- Any conformant editor/client that can launch a local ACP agent command.
- Any conformant local ACP v1 agent, whether built in, registry-provided, or
  passed as a custom command.
- Protocol negotiation, authentication, sessions, prompts, streaming updates,
  permissions, configuration, modes, filesystem callbacks, terminal callbacks,
  cancellation, errors, and extensions.
- Trail lanes, materialized workdirs, prompt checkpoints, transcripts,
  approvals, traces, provenance, and MCP injection.
- Correct shutdown, malformed-peer behavior, backpressure, and recovery.

### Excluded

- ACP v2 semantics. Non-v1 negotiation must never be misrepresented as v1
  capture or compatibility.
- Streamable HTTP while it remains a draft rather than a stable ACP v1
  transport contract.
- Implementing an editor UI. Editor-specific setup adapters are convenience
  surfaces; protocol compatibility must not depend on one editor's settings
  format.
- Claiming that a non-conformant editor or agent can be repaired transparently.
  Trail must diagnose peer violations without inventing protocol data.

## Selected Architecture

Trail will use a schema-driven transparent proxy, not a full protocol
terminator and not an expanded set of ad hoc JSON match arms.

```text
ACP editor/client
        |
        | ACP v1 JSON-RPC stdio
        v
+---------------- Trail ACP proxy ----------------+
| frame codec                                      |
| raw envelope + direction-aware correlator        |
| negotiated-version and capability state          |
| atomic session transformer                       |
| ordered durable capture journal                  |
| semantic capture projector                       |
| conformance diagnostics                          |
+--------------------------------------------------+
        |
        | ACP v1 JSON-RPC stdio
        v
ACP agent
```

The proxy holds the original JSON value for forwarding and a typed view for
validation and observation. Parsing a typed view must not discard unknown
fields or reserialize untouched messages. Known messages are checked with the
official ACP v1 types or vendored schema; extension messages and extension
fields remain in the raw envelope.

## Component Boundaries

The current `trail/src/acp.rs` combines provider discovery, transport, session
transformation, capture, and tests. The implementation will split only the ACP
responsibilities needed for conformance:

- `trail/src/acp.rs`: public provider and relay entry points plus module wiring.
- `trail/src/acp/transport.rs`: stdio child lifecycle, frame reading/writing,
  pumps, shutdown, and stderr forwarding.
- `trail/src/acp/protocol.rs`: raw JSON-RPC envelope classification,
  direction-aware request correlation, negotiated v1 state, method catalog,
  and schema validation hooks.
- `trail/src/acp/transform.rs`: atomic initialization metadata, MCP injection,
  cwd/additional-directory mapping, and validation of transformed requests.
- `trail/src/acp/capture.rs`: ordered journal ingestion and semantic projection
  into Trail sessions, turns, messages, events, spans, approvals, and
  checkpoints.
- `trail/src/acp/registry.rs`: official registry resolution and package/binary
  launch, retained as a separate concern.
- `trail/src/acp/setup.rs`: editor configuration planning and application,
  retained as a separate concern.
- `trail/tests/acp_conformance.rs`: black-box protocol conformance harness.
- `trail/tests/fixtures/acp/v1/`: pinned official schema, method metadata,
  source manifest, and method/variant fixtures.

No module may depend on terminal rendering. Transport must not depend on Trail
database report types. Capture may consume protocol observations but may not
write to either protocol stream.

## Protocol Data Flow

### Framing

The frame codec accepts exactly one UTF-8 JSON-RPC object per non-empty line.
Messages are delimited by `\n` and serialized without embedded raw newlines,
matching ACP v1 stdio. Standard JSON string escapes remain valid. Blank lines
may be ignored for resilience but are never emitted.

Malformed JSON, a non-object top-level value, an invalid JSON-RPC version, or
an invalid request envelope is a peer protocol violation. If an ID can be
recovered safely, Trail emits the appropriate JSON-RPC error to the sender;
otherwise it emits a parse/invalid-request error with a null ID. Trail records
the diagnostic, closes affected in-flight capture state as interrupted, and
shuts down cleanly when continuing would make framing ambiguous.

### Initialization and version negotiation

Trail forwards the editor's `initialize` request unchanged. It must not reject
an editor merely because the editor offers a later protocol version: ACP
negotiation allows the downstream agent to select version `1`.

Trail forwards the agent's initialize response after optionally adding only the
documented `_meta.trail` extension. The modified response is validated before
it replaces the raw response. Trail records the selected version and both
capability sets.

- If the selected version is `1`, Trail enables v1 validation, transformation,
  and semantic capture.
- If the selected version is not `1`, Trail forwards the response so the client
  can follow ACP negotiation rules, disables all v1 transformations, reports
  that the connection is outside Trail's supported protocol, and does not claim
  capture compatibility.
- Session methods received before successful initialization are forwarded only
  as raw peer behavior and produce a diagnostic; Trail must not create a lane
  from an unnegotiated session.

Trail never adds or removes advertised editor or agent capabilities.

### Requests, notifications, and responses

Request IDs may be strings or numbers. Client-to-agent and agent-to-client
requests have independent ID spaces. Correlation keys therefore include both
direction and the exact JSON ID value. Responses may arrive out of order.

The proxy forwards requests, notifications, responses, and errors in arrival
order within each input stream. It must not wait for an earlier request's
response before forwarding later traffic. Capture observations carry a
monotonic relay sequence so database projection can reproduce causal ordering
across both directions.

`$/cancel_request` is correlated with the request in the same sender's request
space. `session/cancel` remains a distinct session notification. Neither may be
translated into the other.

Unknown methods are forwarded unchanged. Methods beginning with `_` are treated
as implementation extensions. Unknown non-extension methods are also preserved
because stable v1 may gain optional methods without a wire-version change; the
schema-drift gate ensures Trail deliberately learns their semantics before a
new full-compatibility release.

## Intentional Transformations

Trail may perform only these protocol mutations:

1. Add `_meta.trail` to a successful `initialize` response without replacing
   other `_meta` keys.
2. Add one Trail stdio MCP server to `mcpServers` for `session/new`,
   `session/load`, and `session/resume` when MCP injection is enabled and an
   equivalent Trail server is not already present.
3. Replace session workspace paths with their effective lane paths when
   materialization is enabled.

Each transformation follows clone-transform-validate-commit:

1. Clone the raw JSON value.
2. Apply the complete mutation to the clone.
3. Validate the clone as the corresponding ACP v1 request or response.
4. Replace the forwarded value only after validation succeeds.

Transformation failure must never leak a partially modified message. A failed
session transformation returns a JSON-RPC error to the editor and does not
forward the request or create an active upstream session.

### MCP injection

The injected MCP descriptor uses the ACP v1 stdio server shape with an absolute
Trail executable, `args: ["mcp"]`, and environment entries for the explicit
workspace and Trail directory. Existing servers and their order are preserved;
Trail is appended once. Equivalence checks use the Trail-owned identity rather
than deleting or replacing an editor-provided server with the same display
name.

### Workspace path mapping

For a session whose requested `cwd` is the Trail workspace root, the effective
cwd is the lane workdir root. For a requested cwd below the workspace root,
Trail preserves the relative suffix below the lane workdir and verifies that
the mapped directory exists or can be created safely. A path outside the Trail
workspace is preserved unchanged because rewriting it would change ACP
semantics and Trail does not own that tree.

Every `additionalDirectories` entry is handled independently:

- A path at or below the Trail workspace root maps to the corresponding lane
  workdir path with the same relative suffix.
- An external path is preserved exactly and recorded as a non-isolated external
  session root.
- Canonicalization, symlink checks, and platform path rules prevent a mapped
  path from escaping its intended root.

The exact requested/effective root mapping is persisted with the ACP session
and reused by load/resume. Agent-to-editor filesystem and terminal callbacks
already contain paths in the effective session namespace and therefore remain
unchanged. Trail validates that path-bearing callbacks are consistent with the
negotiated effective roots before capture; it does not rewrite editor callback
paths behind either peer's back.

## Complete Semantic Capture

Wire compatibility and Trail capture are separate responsibilities. Transport
forwards the protocol message; capture projects a redacted observation.

The capture projector handles every stable v1 method:

- Initialization records negotiated versions, implementation information, and
  capabilities without recording secrets.
- Authentication/logout record lifecycle and outcome metadata; credentials and
  secret-bearing fields are redacted before persistence.
- New/load/resume/close/list/delete maintain durable ACP-to-Trail session state.
- Session load replay reconstructs user and assistant transcript messages,
  message IDs, plans, tools, modes, configuration, session information, and
  usage without inventing a prompt turn.
- Prompt creates one active Trail turn. All content blocks remain represented;
  binary/image/audio data use bounded metadata and digests rather than raw
  unbounded attachments.
- Every stable `session/update` variant is projected with its ACP identity.
  Agent thought content remains intentionally excluded by privacy policy, but a
  redacted fact that an excluded update occurred may be recorded.
- Permission requests and responses preserve option identity and final outcome
  in Trail approvals.
- Mode/config requests and updates maintain the current session projection.
- Filesystem requests/responses record the operation, path, result, and linkage
  to the active turn. File contents use Trail's existing redaction and bounded-
  attachment policies.
- Terminal create/output/wait/kill/release maintain terminal lifecycle,
  truncated-output state, exit status, and tool/turn linkage.
- Both cancellation mechanisms record the exact target and eventual outcome.
- Unknown updates and extension methods remain ordered redacted events with raw
  digests so they are not silently discarded.

Capture state is keyed by ACP session and direction-aware request identity. It
supports concurrent prompts in different sessions on one connection. A peer
attempting overlapping prompts for the same session is handled according to the
downstream agent's response and must not merge two Trail turns.

## Durable Capture Journal and Backpressure

Database writes must not occur inline with high-volume protocol forwarding.
The relay writes bounded redacted observations to an ordered local journal and
an independent projector batches them into Trail under the workspace writer
lock.

The journal is opened before Trail accepts the first session request. If a
session requires Trail capture but the journal cannot be opened, Trail returns
a session-creation error instead of pretending the session is captured. Once a
session is active:

- a journal append is bounded in size and time;
- database writer contention does not block protocol forwarding;
- a temporary projector failure leaves journal entries replayable;
- sequence numbers and idempotency keys prevent loss, duplication, or reorder;
- shutdown drains or durably leaves the journal for recovery;
- permanent journal failure marks capture degraded visibly, closes Trail turn
  state as interrupted, and continues forwarding already-active ACP traffic so
  Trail does not corrupt the editor-agent conversation.

This policy distinguishes protocol correctness from local evidence
availability while never silently claiming evidence that was not persisted.

## Errors, Shutdown, and Recovery

- Upstream spawn failure is reported before protocol startup.
- Editor EOF closes upstream stdin, gives the agent a bounded graceful-exit
  window, then terminates the child if necessary.
- Agent EOF or exit closes open requests and turns with an outcome derived from
  the real exit status; a clean idle exit is not mislabeled as a failed prompt.
- SIGINT/SIGTERM propagate to the child, close capture state, and leave a
  replayable journal.
- Malformed peer traffic cannot panic the relay or poison later workspace use.
- Capture or transformation diagnostics go to stderr only; stdout remains ACP
  JSON-RPC exclusively.
- Secret values are redacted before journal persistence, diagnostic rendering,
  command recording, or Trail events.
- No failure path substitutes `--force`, bypasses ignore/guardrail policy, or
  mutates Git.

## Conformance Test Architecture

### Schema validation

Every official v1 request, notification, response, and error fixture validates
against the pinned schema. Every transformed message validates again after
mutation. Invalid fixtures prove that validation rejects wrong method shapes,
missing required fields, invalid enums, wrong ID types, and malformed `_meta`.

### Method matrix

The black-box relay harness drives all 23 methods through a real Trail relay
process with independent editor and agent fixtures. Each method covers:

- correct direction;
- success response;
- JSON-RPC error response where applicable;
- string and numeric IDs where applicable;
- preservation of unknown fields and `_meta`;
- ordering with unrelated in-flight traffic;
- semantic capture evidence;
- relay shutdown with the method in flight.

### Variant matrix

The suite covers every stable v1 capability and union variant, including:

- all prompt content blocks and advertised prompt capabilities;
- stdio, HTTP, and SSE MCP descriptors accepted by v1 capabilities;
- every stable session update variant;
- every tool-call content/status/kind shape;
- every permission option and outcome;
- modes, select/boolean config options, commands, plan entries, usage, and
  session information;
- all filesystem optional ranges and responses;
- terminal output truncation, running/exited states, signals, kill, wait, and
  release;
- session pagination, load replay, resume, close, delete, and additional roots;
- authentication methods, authentication failures, and logout;
- request and session cancellation races.

### Concurrency and fault matrix

Tests cover multiple sessions on one connection, bidirectional requests with
identical JSON IDs, out-of-order responses, concurrent permission/filesystem/
terminal calls, editor EOF, agent EOF, crashes, malformed messages, slow peers,
blocked Trail database writers, journal replay, duplicate observations, and
process interruption.

### Interoperability

The release gate includes:

- the black-box reference client/reference agent harness built from official ACP
  v1 types;
- Trail's complete unit and E2E suite;
- existing real-agent smoke coverage for supported built-in adapters when
  credentials and executables are available;
- documented opt-in runs for Zed and at least two independent ACP agents;
- macOS, Linux, and Windows CI for platform-neutral protocol behavior.

Real-provider tests supplement but do not replace the deterministic conformance
suite.

### Performance

A release benchmark verifies semantic identity and zero message loss while
measuring forwarding overhead. Uncontended local relay overhead must remain
small relative to agent/model latency, and the benchmark records p50/p95/p99
rather than hiding tail latency. Performance results cannot waive any
correctness assertion.

## Public Compatibility Report

`trail agent acp doctor` will report:

- supported ACP wire version;
- pinned schema source revision and digests;
- transport support;
- provider launch readiness;
- journal and capture readiness;
- materialization/path-mapping readiness;
- conformance build identifier;
- explicit unsupported items, including v2 and draft remote transport.

Documentation will distinguish:

- protocol conformance;
- editor setup convenience;
- agent availability/authentication;
- capture fidelity;
- lane isolation, including preserved external additional roots.

Trail may print “ACP v1 conformant” only when built from a revision whose full
conformance gate passed. Development or locally modified builds report their
evidence as unverified unless the gate is rerun.

## Acceptance Gates

The change is complete only when all gates pass together:

1. The pinned official schema and metadata digests match the source manifest.
2. The schema drift check reports no unreviewed stable-v1 change.
3. All 23 method-matrix entries pass success, error, preservation, capture, and
   shutdown assertions.
4. Every stable capability and union variant has an executable fixture and
   passes schema validation and relay round-trip tests.
5. Multiple sessions, direction-scoped IDs, out-of-order responses, and all
   cancellation races pass without cross-correlation.
6. Cwd subdirectories and additional directories preserve ACP semantics under
   materialization on every supported platform.
7. Capture journal fault/replay tests prove no silent evidence loss,
   duplication, or reorder.
8. Full Trail unit, integration, E2E, formatting, lint, and documentation tests
   pass from a clean checkout.
9. Reference-client/reference-agent interoperability passes.
10. Available real-editor/real-agent smoke tests pass and unavailable external
    dependencies are reported as skipped evidence, never counted as conformance
    proof.
11. Documentation and doctor output state the exact verified boundary without
    claiming ACP v2 or draft transport support.

No subset of these gates is sufficient for completion.

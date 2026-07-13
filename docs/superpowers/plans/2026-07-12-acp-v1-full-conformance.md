# ACP v1 Full Conformance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Trail a transparent, durable, and exhaustively tested ACP wire-protocol v1 proxy between any conformant stdio client/editor and agent, with complete Trail semantic capture and no known stable-v1 gaps.

**Architecture:** Keep the relay a transparent byte-preserving proxy. Parse each newline-delimited frame only for classification, negotiation, intentional atomic transformations, and capture. Pin the official ACP v1 schema and method metadata for validation and offline conformance; use `jsonschema` rather than the official Rust schema crate because that crate requires Rust 1.88 and Trail's declared MSRV is 1.81. Move persistence off the forwarding path by appending direction-ordered ACP receipts to Trail's existing durable agent-capture ingress and replaying them asynchronously.

**Tech Stack:** Rust 2021/MSRV 1.81, `serde`, `serde_json`, `jsonschema` 0.29.1 with default features disabled, `sha2`, stdio JSON-RPC 2.0, SQLite/WAL, Trail's existing agent-receipt and lane-capture models, Cargo unit/integration/E2E tests.

## Global Constraints

- The normative source is official ACP commit `64cbd71ae520b89aac54164d8c1d364333c8ee5f`.
- The vendored `schema/v1/schema.json` digest must be `92c1dfcda10dd47e99127500a3763da2b471f9ac61e12b9bf0430c32cf953796`.
- The vendored `schema/v1/meta.json` digest must be `e0bf36f8123b2544b499174197fdc371ec49a1b4572a35114513d56492741599`.
- Preserve a valid input frame byte-for-byte, including property order and insignificant whitespace, unless `transform.rs` performs one of the three approved transformations: `_meta.trail`, Trail MCP injection, or workspace-path mapping.
- Treat editor-to-agent and agent-to-editor request IDs as separate correlation namespaces.
- Enable v1 validation, transformations, and semantic capture only after the upstream `initialize` response selects `protocolVersion: 1`; forward other negotiated versions without claiming conformance.
- Never wait for the Trail database writer lock on a protocol forwarding thread.
- Redact credentials, tokens, environment secrets, and secret-shaped metadata before durable persistence or stderr diagnostics.
- Keep every new production dependency compatible with Rust 1.81 and all five release targets.
- Commit only files named by the active task; preserve unrelated worktree changes.

---

## Task 1: Pin the Official ACP v1 Contract

**Files:**
- Modify: `Cargo.toml`
- Modify: `trail/Cargo.toml`
- Create: `trail/tests/fixtures/acp/v1/schema.json`
- Create: `trail/tests/fixtures/acp/v1/meta.json`
- Create: `trail/tests/fixtures/acp/v1/source.json`
- Create: `trail/src/acp/schema.rs`
- Modify: `trail/src/acp.rs`

- [ ] **Step 1: Add the failing artifact-integrity unit test**

Add this test to `trail/src/acp/schema.rs` before defining its helpers:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_v1_artifacts_match_manifest_and_compile() {
        let contract = AcpV1Contract::load().unwrap();
        assert_eq!(contract.wire_version(), 1);
        assert_eq!(contract.method_names().len(), 23);
        assert_eq!(contract.schema_sha256(), ACP_V1_SCHEMA_SHA256);
        assert_eq!(contract.meta_sha256(), ACP_V1_META_SHA256);
        assert!(contract.validator().is_valid(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": 1, "clientCapabilities": {}}
        })));
    }
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run: `cargo test -p trail acp::schema::tests::pinned_v1_artifacts_match_manifest_and_compile -- --exact`

Expected: FAIL because `acp::schema` and `AcpV1Contract` do not exist.

- [ ] **Step 3: Add the MSRV-compatible validator dependency**

Add to `[workspace.dependencies]` in `Cargo.toml`:

```toml
jsonschema = { version = "0.29.1", default-features = false }
```

Add to `[dependencies]` in `trail/Cargo.toml`:

```toml
jsonschema.workspace = true
```

- [ ] **Step 4: Vendor and identify the exact official artifacts**

Fetch the two files from the pinned upstream revision and verify both digests:

```bash
mkdir -p trail/tests/fixtures/acp/v1
curl --fail --location --silent --show-error https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/64cbd71ae520b89aac54164d8c1d364333c8ee5f/schema/v1/schema.json --output trail/tests/fixtures/acp/v1/schema.json
curl --fail --location --silent --show-error https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/64cbd71ae520b89aac54164d8c1d364333c8ee5f/schema/v1/meta.json --output trail/tests/fixtures/acp/v1/meta.json
shasum -a 256 trail/tests/fixtures/acp/v1/schema.json trail/tests/fixtures/acp/v1/meta.json
```

Expected: the output matches the two Global Constraints digests. Create `source.json` with this exact content:

```json
{
  "repository": "https://github.com/agentclientprotocol/agent-client-protocol",
  "commit": "64cbd71ae520b89aac54164d8c1d364333c8ee5f",
  "wireVersion": 1,
  "schemaSha256": "92c1dfcda10dd47e99127500a3763da2b471f9ac61e12b9bf0430c32cf953796",
  "metaSha256": "e0bf36f8123b2544b499174197fdc371ec49a1b4572a35114513d56492741599"
}
```

- [ ] **Step 5: Implement the immutable contract loader**

Define this public-in-crate API in `schema.rs` and register `mod schema;` in `acp.rs`:

```rust
pub(crate) const ACP_V1_SCHEMA_SHA256: &str = "92c1dfcda10dd47e99127500a3763da2b471f9ac61e12b9bf0430c32cf953796";
pub(crate) const ACP_V1_META_SHA256: &str = "e0bf36f8123b2544b499174197fdc371ec49a1b4572a35114513d56492741599";

pub(crate) struct AcpV1Contract {
    schema_sha256: String,
    meta_sha256: String,
    method_names: std::collections::BTreeSet<String>,
    validator: jsonschema::Validator,
}

impl AcpV1Contract {
    pub(crate) fn load() -> crate::Result<Self>;
    pub(crate) fn wire_version(&self) -> u16 { 1 }
    pub(crate) fn schema_sha256(&self) -> &str;
    pub(crate) fn meta_sha256(&self) -> &str;
    pub(crate) fn method_names(&self) -> &std::collections::BTreeSet<String>;
    pub(crate) fn validator(&self) -> &jsonschema::Validator;
    pub(crate) fn validate(&self, message: &serde_json::Value) -> crate::Result<()>;
}
```

Load fixture bytes with `include_bytes!`, hash the exact bytes, parse `agentMethods`, `clientMethods`, and `protocolMethods`, assert 23 unique values, and compile the schema with `jsonschema::validator_for`.

- [ ] **Step 6: Run integrity tests and the MSRV check**

Run: `cargo test -p trail acp::schema::tests::pinned_v1_artifacts_match_manifest_and_compile -- --exact`

Expected: PASS.

Run: `cargo +1.81.0 check -p trail --all-targets`

Expected: PASS, including the new dependency graph.

If the toolchain is absent, first run `rustup toolchain install 1.81.0 --profile minimal` and repeat the check.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock trail/Cargo.toml trail/src/acp.rs trail/src/acp/schema.rs trail/tests/fixtures/acp/v1
git commit -m "test: pin official ACP v1 contract"
```

## Task 2: Introduce a Direction-Safe Raw JSON-RPC Model

**Files:**
- Create: `trail/src/acp/protocol.rs`
- Modify: `trail/src/acp.rs`
- Create: `trail/tests/fixtures/acp/v1/messages.jsonl`

- [ ] **Step 1: Write failing classification and preservation tests**

Cover requests, notifications, success responses, error responses, string IDs, integer IDs, null IDs, unknown extension methods, and same-valued IDs in opposite directions. The core assertion is:

```rust
#[test]
fn frame_preserves_raw_bytes_and_scopes_ids_by_direction() {
    let client = Frame::parse(Direction::ClientToAgent, br#" { "id":7, "jsonrpc":"2.0", "method":"session/list", "params":{} }\n"#.to_vec()).unwrap();
    let agent = Frame::parse(Direction::AgentToClient, br#" { "id":7, "jsonrpc":"2.0", "method":"fs/read_text_file", "params":{"sessionId":"s","path":"a"} }\n"#.to_vec()).unwrap();
    assert_eq!(client.forward_bytes(), client.raw_bytes());
    assert_eq!(agent.forward_bytes(), agent.raw_bytes());
    assert_ne!(client.correlation_key(), agent.correlation_key());
}
```

- [ ] **Step 2: Run and verify failure**

Run: `cargo test -p trail acp::protocol::tests -- --nocapture`

Expected: FAIL because the protocol model is absent.

- [ ] **Step 3: Implement the protocol model**

Use these exact boundary types:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Direction { ClientToAgent, AgentToClient }

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RequestId { Null, Number(i64), String(String) }

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CorrelationKey { pub direction: Direction, pub id: RequestId }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnvelopeKind { Request, Notification, SuccessResponse, ErrorResponse }

pub(crate) struct Frame {
    direction: Direction,
    raw: Vec<u8>,
    parsed: serde_json::Value,
    kind: EnvelopeKind,
    method: Option<String>,
    id: Option<RequestId>,
    transformed: Option<Vec<u8>>,
}

impl Frame {
    pub(crate) fn parse(direction: Direction, raw: Vec<u8>) -> std::io::Result<Self>;
    pub(crate) fn raw_bytes(&self) -> &[u8];
    pub(crate) fn forward_bytes(&self) -> &[u8];
    pub(crate) fn value(&self) -> &serde_json::Value;
    pub(crate) fn value_mut_for_transform(&mut self) -> &mut serde_json::Value;
    pub(crate) fn commit_transform(&mut self) -> crate::Result<()>;
    pub(crate) fn correlation_key(&self) -> Option<CorrelationKey>;
}
```

Reject invalid UTF-8, non-object top-level values, wrong/missing `jsonrpc`, fractional IDs, envelopes with both `result` and `error`, and request envelopes without a string `method`. Do not reject unknown methods or unknown fields.

- [ ] **Step 4: Run focused and regression tests**

Run: `cargo test -p trail acp::protocol::tests`

Expected: PASS.

Run: `cargo test -p trail acp::tests`

Expected: existing ACP unit tests remain PASS.

- [ ] **Step 5: Commit**

```bash
git add trail/src/acp.rs trail/src/acp/protocol.rs trail/tests/fixtures/acp/v1/messages.jsonl
git commit -m "refactor: model ACP JSON-RPC frames"
```

## Task 3: Make Transport Byte-Preserving and Concurrent

**Files:**
- Create: `trail/src/acp/transport.rs`
- Modify: `trail/src/acp.rs`
- Create: `trail/tests/acp_transport.rs`

- [ ] **Step 1: Add black-box failing transport tests**

Build a fake client and fake agent using pipes. Assert byte-exact forwarding for unmodified frames, concurrent bidirectional requests with the same ID, out-of-order responses, blank-line behavior, CRLF preservation, stderr isolation, editor EOF, agent EOF, non-zero child exit, and malformed input. Use a 250 ms assertion window to prove forwarding does not wait on capture.

- [ ] **Step 2: Run and verify the byte-preservation test fails**

Run: `cargo test -p trail --test acp_transport forwards_unmodified_frames_byte_exactly -- --exact`

Expected: FAIL because current code parses and reserializes every frame.

- [ ] **Step 3: Extract the relay transport**

Define:

```rust
pub(crate) trait FrameObserver: Send + Sync + 'static {
    fn observe(&self, frame: &mut Frame) -> crate::Result<()>;
    fn finish(&self, reason: RelayFinishReason);
}

pub(crate) struct StdioRelay<O: FrameObserver> { observer: std::sync::Arc<O> }

impl<O: FrameObserver> StdioRelay<O> {
    pub(crate) fn run(self, options: &AcpRelayOptions) -> crate::Result<()>;
}
```

Replace `read_json_line`/`write_json_line` with raw `read_until(b'\n', ...)` framing. Parse a copy into `Frame`, let the observer transform it atomically, then write `frame.forward_bytes()` in one direction-local writer loop. Keep upstream stderr copied only to Trail stderr.

- [ ] **Step 4: Implement bounded shutdown**

On editor EOF, close child stdin and wait up to two seconds; on agent EOF, stop the editor pump; on timeout, kill and reap the child. Ensure every path calls `FrameObserver::finish` exactly once and joins both pump threads.

- [ ] **Step 5: Run the transport suite**

Run: `cargo test -p trail --test acp_transport`

Expected: PASS with no hangs and no stdout diagnostics.

- [ ] **Step 6: Commit**

```bash
git add trail/src/acp.rs trail/src/acp/transport.rs trail/tests/acp_transport.rs
git commit -m "fix: preserve ACP transport semantics"
```

## Task 4: Negotiate ACP v1 and Apply Only Atomic Validated Transformations

**Files:**
- Create: `trail/src/acp/transform.rs`
- Modify: `trail/src/acp.rs`
- Modify: `trail/src/acp/protocol.rs`
- Create: `trail/tests/acp_transform.rs`

- [ ] **Step 1: Add failing negotiation tests**

Test forwarded initialize requests offering `1`, offering `[1, 2]` through extension-shaped data, upstream selection of `1`, upstream selection of `2`, initialize error, duplicate initialize, and a session request before negotiation. Assert Trail never rewrites the offered version and activates transformations only after an upstream response selects exactly `1`.

- [ ] **Step 2: Add failing transformation rollback tests**

For `_meta.trail`, MCP injection, and path mapping, assert a transformed frame validates against the pinned schema. Force an invalid candidate and assert `forward_bytes()` remains the original raw bytes, with a diagnostic sent to stderr/capture health rather than a partial mutation.

- [ ] **Step 3: Implement negotiation state**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NegotiationState { AwaitingInitialize, InitializePending, V1, Other(u16), Failed }

pub(crate) struct TransformPipeline {
    state: NegotiationState,
    initialize_id: Option<RequestId>,
    contract: std::sync::Arc<AcpV1Contract>,
    options: TransformOptions,
}

impl TransformPipeline {
    pub(crate) fn apply(&mut self, frame: &mut Frame) -> crate::Result<TransformOutcome>;
    pub(crate) fn state(&self) -> NegotiationState;
}
```

Correlate the initialize response only in the client-to-agent request namespace. `Other(version)` forwards everything but disables v1 schema validation, intentional transformations, and v1 semantic capture.

- [ ] **Step 4: Implement atomic candidate validation**

Clone the parsed `Value`, apply all enabled mutations to the clone, validate the full candidate with `AcpV1Contract`, serialize once with a newline matching the original framing, then commit. Preserve existing `_meta` keys and existing MCP servers; inject one stable Trail server identity only when capabilities allow it.

- [ ] **Step 5: Run focused tests**

Run: `cargo test -p trail --test acp_transform`

Expected: PASS for negotiation, preservation, deduplication, and rollback.

- [ ] **Step 6: Commit**

```bash
git add trail/src/acp.rs trail/src/acp/protocol.rs trail/src/acp/transform.rs trail/tests/acp_transform.rs
git commit -m "feat: validate ACP v1 transformations"
```

## Task 5: Correct Workspace and Additional-Directory Mapping

**Files:**
- Modify: `trail/src/acp/transform.rs`
- Modify: `trail/src/model/lane/activity.rs`
- Modify: `trail/src/db/storage/schema/ddl.rs`
- Modify: `trail/src/db/lane/control/acp_sessions.rs`
- Create: `trail/tests/acp_workspace_mapping.rs`

- [ ] **Step 1: Write failing mapping matrix tests**

Cover workspace root, nested cwd, normalized `.`/`..`, symlink escape, external cwd, repeated roots, overlapping additional directories, Windows drive-letter paths, and UNC paths. Assert descendants map to the same relative location inside the materialized lane and external roots remain unchanged with `isolated: false` evidence.

- [ ] **Step 2: Run and verify nested-cwd failure**

Run: `cargo test -p trail --test acp_workspace_mapping maps_nested_cwd_without_collapsing_to_lane_root -- --exact`

Expected: FAIL against the current cwd replacement logic.

- [ ] **Step 3: Implement the mapper boundary**

```rust
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PathMapping {
    pub original: std::path::PathBuf,
    pub effective: std::path::PathBuf,
    pub isolated: bool,
}

pub(crate) struct WorkspaceMapper {
    workspace_root: std::path::PathBuf,
    materialized_root: std::path::PathBuf,
}

impl WorkspaceMapper {
    pub(crate) fn map(&self, path: &std::path::Path) -> crate::Result<PathMapping>;
    pub(crate) fn map_session_params(&self, params: &mut serde_json::Map<String, serde_json::Value>) -> crate::Result<Vec<PathMapping>>;
}
```

Canonicalize existing ancestors, reject a relative result that escapes with `..`, retain the exact descendant suffix, deduplicate effective additional roots without reordering the first occurrence, and preserve external paths unchanged.

- [ ] **Step 4: Persist mappings with ACP sessions**

Add `path_mappings_json TEXT NOT NULL DEFAULT '[]'` to `lane_acp_sessions`, migrate existing databases idempotently, add `path_mappings: Vec<AcpPathMapping>` to `LaneAcpSession`, and round-trip it through create/get/list/update queries.

- [ ] **Step 5: Run mapping and database tests**

Run: `cargo test -p trail --test acp_workspace_mapping`

Run: `cargo test -p trail db::lane::control::acp_sessions`

Expected: PASS on the host platform; platform-string unit cases pass everywhere.

- [ ] **Step 6: Commit**

```bash
git add trail/src/acp/transform.rs trail/src/model/lane/activity.rs trail/src/db/storage/schema/ddl.rs trail/src/db/lane/control/acp_sessions.rs trail/tests/acp_workspace_mapping.rs
git commit -m "fix: preserve ACP workspace path semantics"
```

## Task 6: Put Durable Capture Behind a Non-Blocking Ordered Ingress

**Files:**
- Create: `trail/src/acp/capture.rs`
- Modify: `trail/src/acp.rs`
- Modify: `trail/src/model/agent_capture.rs`
- Modify: `trail/src/db/lane/control/agent_capture.rs`
- Modify: `trail/src/db/storage/schema/agent_capture.rs`
- Create: `trail/tests/acp_capture_journal.rs`

- [ ] **Step 1: Add failing lock-contention and replay tests**

Hold Trail's database writer lock, forward 1,000 ACP frames, and assert the final frame reaches the peer within 250 ms. Then release the lock and assert all 1,000 receipts project in strict per-connection sequence. Restart after 500 durable appends but before projection and assert replay produces 1,000 unique ordered events after the remaining frames arrive.

- [ ] **Step 2: Run and verify current synchronous capture fails**

Run: `cargo test -p trail --test acp_capture_journal forwarding_never_waits_for_database_writer -- --exact`

Expected: FAIL because `capture_step` can wait up to 30 seconds.

- [ ] **Step 3: Extend receipt identity for ACP ordering**

Add `connection_id TEXT`, `direction TEXT`, and `connection_sequence INTEGER` columns plus a unique index over `(connection_id, direction, connection_sequence)`. Add matching optional fields to `AgentHookReceiptInput` and `AgentHookReceipt`. Retain existing native-hook behavior when they are absent.

- [ ] **Step 4: Implement the ingress worker**

```rust
pub(crate) struct CaptureIngress {
    tx: std::sync::mpsc::SyncSender<CaptureCommand>,
    health: std::sync::Arc<CaptureHealth>,
    worker: Option<std::thread::JoinHandle<()>>,
}

pub(crate) enum CaptureCommand {
    Frame(CapturedFrame),
    Finish(RelayFinishReason),
}

pub(crate) struct CapturedFrame {
    pub connection_id: String,
    pub direction: Direction,
    pub sequence: u64,
    pub received_at: i64,
    pub redacted_message: serde_json::Value,
}

impl CaptureIngress {
    pub(crate) fn append(&self, frame: CapturedFrame) -> crate::Result<()>;
    pub(crate) fn shutdown(mut self, timeout: std::time::Duration) -> CaptureShutdownReport;
}
```

The forwarding observer performs bounded redaction and enqueue only. The worker persists through `persist_agent_hook_receipt`, assigns deterministic dedupe keys `acp:{connection}:{direction}:{sequence}`, and calls the existing replay pipeline. A full in-memory queue must spill a redacted JSONL record with `sync_data` into `options.db_dir.join("acp-ingress").join(format!("{connection_id}.jsonl"))`; it must never drop a record silently.

- [ ] **Step 5: Implement recovery and health**

On startup, replay spill files and pending ACP receipts before accepting new projections. Track `healthy`, `degraded`, `last_error`, `queued`, `spilled`, and `last_projected_sequence`. A permanent append failure marks capture degraded, emits one stderr warning, and preserves the spill file for `trail agent acp doctor`.

- [ ] **Step 6: Run journal tests**

Run: `cargo test -p trail --test acp_capture_journal`

Expected: PASS for contention, crash recovery, idempotency, ordering, bounded shutdown, and queue overflow.

- [ ] **Step 7: Commit**

```bash
git add trail/src/acp.rs trail/src/acp/capture.rs trail/src/model/agent_capture.rs trail/src/db/lane/control/agent_capture.rs trail/src/db/storage/schema/agent_capture.rs trail/tests/acp_capture_journal.rs
git commit -m "feat: journal ACP capture off the relay path"
```

## Task 7: Capture All Agent-Side Lifecycle and Session Methods

**Files:**
- Modify: `trail/src/acp/capture.rs`
- Modify: `trail/src/model/agent_capture.rs`
- Modify: `trail/src/db/lane/control/agent_capture.rs`
- Create: `trail/tests/acp_session_semantics.rs`

- [ ] **Step 1: Add failing lifecycle method tests**

Cover success and JSON-RPC error outcomes for `initialize`, `authenticate`, `logout`, `session/new`, `session/load`, `session/resume`, `session/close`, `session/list`, and `session/delete`. Assert auth credentials are absent from receipts, pagination tokens remain correlated, and session mappings transition only after successful responses.

- [ ] **Step 2: Add failing replay reconstruction test**

Send a `session/load` response followed by its replayed `session/update` notifications containing user, assistant, tool, plan, and usage history. Assert Trail reconstructs ordered transcript turns rather than creating a single synthetic load event.

- [ ] **Step 3: Implement the pending-request registry**

```rust
pub(crate) enum PendingOperation {
    Initialize { requested_version: serde_json::Value },
    Authenticate { method_id: String },
    Logout,
    SessionNew { cwd: PathMapping, additional: Vec<PathMapping> },
    SessionLoad { session_id: String, replay: ReplayAccumulator },
    SessionResume { session_id: String },
    SessionClose { session_id: String },
    SessionList { cursor: Option<String> },
    SessionDelete { session_id: String },
    Prompt { session_id: String },
    SetMode { session_id: String, mode_id: String },
    SetConfig { session_id: String, config_id: String, value: serde_json::Value },
    ClientCallback(ClientCallbackOperation),
}
```

Store it by `CorrelationKey`, reject duplicate in-flight keys in the same direction as peer violations, and pair responses without removing same-ID operations in the opposite direction.

- [ ] **Step 4: Project lifecycle semantics**

Map every request, success, error, and notification to a redacted `AgentLifecycleEvent` with original ACP method, request ID, direction, connection sequence, session ID, outcome, and error code. Reuse `lane_acp_sessions` for durable ACP-to-Trail mapping and the existing agent capture state machine for session/turn transitions.

- [ ] **Step 5: Run focused tests**

Run: `cargo test -p trail --test acp_session_semantics`

Expected: PASS for all nine lifecycle/session methods and replay reconstruction.

- [ ] **Step 6: Commit**

```bash
git add trail/src/acp/capture.rs trail/src/model/agent_capture.rs trail/src/db/lane/control/agent_capture.rs trail/tests/acp_session_semantics.rs
git commit -m "feat: capture ACP session lifecycle"
```

## Task 8: Capture Prompts, Configuration, Modes, and Both Cancellation Forms

**Files:**
- Modify: `trail/src/acp/capture.rs`
- Modify: `trail/src/model/agent_capture.rs`
- Create: `trail/tests/acp_turn_semantics.rs`

- [ ] **Step 1: Add failing method tests**

Cover `session/prompt`, `session/cancel`, `session/set_mode`, `session/set_config_option`, and `$/cancel_request` from each legal direction. Include cancel-before-response, cancel-after-response, cancel racing prompt completion, and cancellation error code `-32800`.

- [ ] **Step 2: Add all prompt content fixtures**

Exercise stable text, image, audio, resource-link, and embedded-resource content blocks with `_meta` and unknown extension keys. Assert the Trail transcript has a lossless redacted structured payload plus the readable text projection.

- [ ] **Step 3: Implement turn/config state**

Track one active prompt per ACP session, while allowing prompts in separate sessions concurrently. Persist current mode and every select/boolean config option value after successful responses. Resolve `$/cancel_request.params.requestId` within the sender's request namespace and record the eventual original-request outcome.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p trail --test acp_turn_semantics`

Expected: PASS for all five methods, all stable content blocks, and cancellation races.

- [ ] **Step 5: Commit**

```bash
git add trail/src/acp/capture.rs trail/src/model/agent_capture.rs trail/tests/acp_turn_semantics.rs
git commit -m "feat: capture ACP turns and cancellation"
```

## Task 9: Capture Every Session Update Variant

**Files:**
- Modify: `trail/src/acp/capture.rs`
- Modify: `trail/src/model/agent_capture.rs`
- Create: `trail/tests/fixtures/acp/v1/session_updates.jsonl`
- Create: `trail/tests/acp_update_semantics.rs`

- [ ] **Step 1: Generate the fixture inventory from the pinned schema**

Create one schema-valid example for every stable `SessionUpdate` branch: user message chunk, agent message chunk, agent thought chunk, tool call, tool call update, plan, available commands update, current mode update, config option update, and usage update. Include every stable tool-call content/status/kind shape and plan priority/status value.

- [ ] **Step 2: Add a failing exhaustiveness test**

```rust
#[test]
fn every_pinned_session_update_variant_has_semantic_projection() {
    let cases = fixture_cases("fixtures/acp/v1/session_updates.jsonl");
    assert_eq!(cases.variant_names(), pinned_session_update_variant_names());
    for case in cases {
        assert_schema_valid(&case.message);
        let events = project(case.message);
        assert!(events.iter().any(|event| event.payload["acpVariant"] == case.name));
    }
}
```

- [ ] **Step 3: Implement explicit variant projection**

Use a closed `AcpV1SessionUpdateKind` enum whose `ALL` constant drives the test. Preserve raw redacted structured content on every event, continue the existing human-readable transcript/tool/plan/usage projections, and send unrecognized extension updates to `Extension` without dropping or reordering them.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p trail --test acp_update_semantics`

Expected: PASS with fixture names exactly matching the stable schema inventory.

- [ ] **Step 5: Commit**

```bash
git add trail/src/acp/capture.rs trail/src/model/agent_capture.rs trail/tests/fixtures/acp/v1/session_updates.jsonl trail/tests/acp_update_semantics.rs
git commit -m "feat: capture every ACP session update"
```

## Task 10: Capture Permission, Filesystem, and Terminal Callbacks

**Files:**
- Modify: `trail/src/acp/capture.rs`
- Modify: `trail/src/model/agent_capture.rs`
- Create: `trail/tests/acp_client_callbacks.rs`

- [ ] **Step 1: Add failing callback method matrix**

Cover `session/request_permission`, `fs/read_text_file`, `fs/write_text_file`, `terminal/create`, `terminal/output`, `terminal/wait_for_exit`, `terminal/kill`, and `terminal/release`. For each, assert request, success, JSON-RPC error, string/numeric ID, unrelated interleaving, and shutdown-in-flight evidence.

- [ ] **Step 2: Add callback variant cases**

Cover all permission option kinds and outcomes; read ranges with optional line/limit fields; write content and errors; terminal environment, cwd, dimensions, output truncation, running/exited state, exit code, signal, kill-before-exit, wait-before-exit, and release-after-exit.

- [ ] **Step 3: Implement client callback state**

```rust
pub(crate) enum ClientCallbackOperation {
    Permission { session_id: String, tool_call_id: String, options: std::collections::BTreeMap<String, String> },
    ReadFile { session_id: String, path: String, line: Option<u64>, limit: Option<u64> },
    WriteFile { session_id: String, path: String, content_sha256: String, byte_len: u64 },
    TerminalCreate { session_id: String, command: Vec<String>, cwd: Option<String> },
    TerminalOutput { session_id: String, terminal_id: String },
    TerminalWait { session_id: String, terminal_id: String },
    TerminalKill { session_id: String, terminal_id: String },
    TerminalRelease { session_id: String, terminal_id: String },
}
```

Persist terminal identity and lifecycle until release. Store written content only through Trail's content-addressed/redacted artifact path; the receipt contains digest and byte length. Record filesystem path mapping identity and terminal output truncation markers.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p trail --test acp_client_callbacks`

Expected: PASS for all eight methods and all callback variants.

- [ ] **Step 5: Commit**

```bash
git add trail/src/acp/capture.rs trail/src/model/agent_capture.rs trail/tests/acp_client_callbacks.rs
git commit -m "feat: capture ACP client callbacks"
```

## Task 11: Build the Exhaustive 23-Method Conformance Harness

**Files:**
- Create: `trail/tests/acp_conformance.rs`
- Create: `trail/tests/support/acp_harness.rs`
- Create: `trail/tests/fixtures/acp/v1/method_cases.json`
- Create: `trail/tests/fixtures/acp/v1/variant_cases.json`
- Modify: `trail/tests/e2e.rs`

- [ ] **Step 1: Define the exact method inventory fixture**

The fixture must contain these 23 unique names with side and envelope kind sourced from `meta.json`: `initialize`, `authenticate`, `logout`, `session/new`, `session/load`, `session/resume`, `session/close`, `session/list`, `session/delete`, `session/prompt`, `session/cancel`, `session/set_mode`, `session/set_config_option`, `session/update`, `session/request_permission`, `fs/read_text_file`, `fs/write_text_file`, `terminal/create`, `terminal/output`, `terminal/wait_for_exit`, `terminal/kill`, `terminal/release`, and `$/cancel_request`.

- [ ] **Step 2: Write a failing inventory equality test**

Compare fixture method names with the union of all values in pinned `meta.json`; neither set may contain a name absent from the other.

- [ ] **Step 3: Define and test the stable variant inventory**

Make `variant_cases.json` name a schema-valid fixture for every branch of these protocol-domain unions: `ContentBlock`, `EmbeddedResourceResource`, `McpServer`, `PermissionOptionKind`, `RequestPermissionOutcome`, `SessionConfigOption`, `SessionUpdate`, `TerminalExitStatus`, `ToolCallContent`, `ToolCallStatus`, `ToolKind`, `PlanEntryPriority`, `PlanEntryStatus`, `Role`, and `StopReason`. Add an inventory test that reads discriminator constants and enum values from the pinned schema and requires exact equality with the fixture manifest.

Generate and validate every capability combination: all eight `PromptCapabilities` boolean combinations; all four `McpCapabilities` combinations; all eight client filesystem/terminal combinations; omitted, null, and object forms for client session capabilities; and the complete three-state Cartesian product (omitted, null, object) of the five `SessionCapabilities` fields `list`, `delete`, `additionalDirectories`, `resume`, and `close`. For each combination, round-trip initialize through the relay and assert no capability field changes.

- [ ] **Step 4: Implement the black-box reference harness**

The harness launches the real `trail acp relay --` binary with the platform-specific command returned by `fixture_agent_command(scenario: &str) -> Vec<String>` and drives both halves over stdio. On Unix that helper writes an executable shell fixture; on Windows it writes a PowerShell fixture. Each method case must assert correct direction, schema-valid success, schema-valid error where a response exists, string/numeric IDs, unknown-field and `_meta` preservation, interleaving, semantic receipt evidence, and deterministic shutdown while in flight.

- [ ] **Step 5: Run the method and variant matrices**

Run: `cargo test -p trail --test acp_conformance -- --nocapture`

Expected: 23 method cases PASS; output names every method once, every stable domain variant once, and every generated capability combination.

- [ ] **Step 6: Retire overlapping ad hoc E2E helpers**

Move reusable ACP fake-peer logic from `e2e.rs` into `tests/support/acp_harness.rs`; keep existing user-facing CLI assertions in `e2e.rs`. Do not reduce existing assertions.

- [ ] **Step 7: Run all ACP integration tests**

Run: `cargo test -p trail --test acp_conformance --test acp_transport --test acp_transform --test acp_workspace_mapping --test acp_capture_journal --test acp_session_semantics --test acp_turn_semantics --test acp_update_semantics --test acp_client_callbacks`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add trail/tests/acp_conformance.rs trail/tests/support/acp_harness.rs trail/tests/fixtures/acp/v1/method_cases.json trail/tests/fixtures/acp/v1/variant_cases.json trail/tests/e2e.rs
git commit -m "test: enforce the ACP v1 method matrix"
```

## Task 12: Add Fault, Concurrency, and Interoperability Gates

**Files:**
- Create: `trail/tests/acp_faults.rs`
- Create: `trail/tests/acp_interop.rs`
- Create: `trail/benches/acp_relay_bench.rs`
- Create: `tools/acp-v1-reference-peer/Cargo.toml`
- Create: `tools/acp-v1-reference-peer/src/main.rs`
- Create: `scripts/check-acp-v1-schema-drift.sh`
- Create: `scripts/test-acp-v1-reference-interop.sh`
- Modify: `trail/Cargo.toml`
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add failing fault and concurrency cases**

Test multiple sessions, 100 concurrent requests per direction, same ID in opposite directions, out-of-order responses, slow reader/writer backpressure, malformed UTF-8, invalid JSON-RPC envelopes, oversized frames at the configured limit and one byte above it, capture worker panic simulation, spill write failure, child crash, editor crash, and cancellation races.

- [ ] **Step 2: Implement explicit frame and queue limits**

Add named constants and diagnostics: `ACP_MAX_FRAME_BYTES = 16 * 1024 * 1024`, `ACP_CAPTURE_QUEUE_CAPACITY = 4096`, and `ACP_SHUTDOWN_TIMEOUT = Duration::from_secs(2)`. A frame above the limit terminates that peer direction with an invalid-data error; it must not allocate unbounded memory or emit a JSON-RPC response on behalf of the peer.

- [ ] **Step 3: Add schema drift enforcement**

The script downloads the two files from the manifest repository's current stable-v1 revision, prints old/new commit and digests, and exits non-zero on a difference. CI runs the offline digest/conformance tests on every build; the network drift job runs on schedule and manual dispatch so pull requests are not dependent on external availability.

- [ ] **Step 4: Add the official-type reference peers**

Create a standalone Cargo package with its own empty `[workspace]` table and exact dependency `agent-client-protocol-schema = "=1.4.0"`. Keep it outside CrabDB's workspace members so Trail retains MSRV 1.81; compile the reference peer with Rust 1.88 or newer in the interoperability job. Its `client` and `agent` modes must serialize and deserialize through official `agent_client_protocol_schema::v1` request, response, notification, content, update, and error types rather than hand-written JSON.

The script builds the standalone peer into `target/acp-reference`, exports its absolute path as `TRAIL_ACP_REFERENCE_PEER`, and runs `acp_interop`. Exercise official client → Trail → official agent and official client → Trail → fixture agent scenarios. Verify initialize, auth, new/load/resume, prompt streaming, permission, filesystem, terminal, cancellation, close/delete, and clean shutdown.

- [ ] **Step 5: Add the correctness-preserving relay benchmark**

Register a harness-free `acp_relay_bench` in `trail/Cargo.toml`. Relay 10,000 mixed request/notification/response frames through in-memory transport with capture enabled, assert byte identity for every untransformed frame and exactly 10,000 unique receipt sequences, then print p50/p95/p99 forwarding latency in microseconds. The benchmark fails on message loss, duplication, reorder, or semantic mismatch; it does not impose a machine-specific absolute latency threshold.

- [ ] **Step 6: Run fault and interoperability suites**

Run: `cargo test -p trail --test acp_faults -- --nocapture`

Run: `scripts/test-acp-v1-reference-interop.sh`

Expected: PASS without deadlock, dropped capture evidence, or cross-correlation.

- [ ] **Step 7: Run the benchmark**

Run: `cargo bench -p trail --bench acp_relay_bench`

Expected: PASS correctness assertions and print p50/p95/p99.

- [ ] **Step 8: Run schema drift locally**

Run: `scripts/check-acp-v1-schema-drift.sh`

Expected: PASS with both pinned digests unchanged.

- [ ] **Step 9: Commit**

```bash
git add trail/tests/acp_faults.rs trail/tests/acp_interop.rs trail/benches/acp_relay_bench.rs tools/acp-v1-reference-peer scripts/check-acp-v1-schema-drift.sh scripts/test-acp-v1-reference-interop.sh trail/Cargo.toml .github/workflows/ci.yml
git commit -m "test: gate ACP v1 faults and interoperability"
```

## Task 13: Publish Verifiable Compatibility Evidence

**Files:**
- Modify: `trail/src/model/lane/activity.rs`
- Modify: `trail/src/cli/command/handler/acp.rs`
- Modify: `trail/src/cli/command/render/acp.rs`
- Modify: `trail/tests/e2e.rs`
- Modify: `README.md`
- Create: `docs/acp-v1-compatibility.md`

- [ ] **Step 1: Add failing doctor-report assertions**

Assert JSON and human output include wire version, schema revision/digests, stdio transport, provider readiness, capture journal health, path-mapping health, conformance evidence status, and explicit exclusions for ACP v2 and draft remote transport. A locally modified or unverified build must not print `ACP v1 conformant`.

- [ ] **Step 2: Extend the report model**

```rust
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AcpConformanceEvidence {
    pub wire_version: u16,
    pub schema_commit: String,
    pub schema_sha256: String,
    pub meta_sha256: String,
    pub transport: String,
    pub method_count: u16,
    pub evidence_status: String,
    pub build_identifier: String,
    pub exclusions: Vec<String>,
}
```

Add `conformance: AcpConformanceEvidence` and capture/path health checks to `AcpDoctorReport`. Generate `build_identifier` from the package version plus compile-time source revision when available.

- [ ] **Step 3: Document the precise boundary**

Document protocol conformance separately from editor setup convenience, agent installation/authentication, capture fidelity, lane isolation, preserved external roots, ACP v2, and draft HTTP transport. Include opt-in smoke commands for Zed and at least two independent ACP agents, with unavailable programs reported as skipped evidence.

- [ ] **Step 4: Run report and documentation tests**

Run: `cargo test -p trail --test e2e agent_acp_doctor -- --nocapture`

Expected: PASS for JSON and human renderers.

Run: `cargo test -p trail --doc`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add trail/src/model/lane/activity.rs trail/src/cli/command/handler/acp.rs trail/src/cli/command/render/acp.rs trail/tests/e2e.rs README.md docs/acp-v1-compatibility.md
git commit -m "docs: publish ACP v1 compatibility evidence"
```

## Task 14: Run the Release Acceptance Gate

**Files:**
- Modify only files required by failures attributable to this ACP change

- [ ] **Step 1: Format and inspect the diff**

Run: `cargo fmt --all -- --check`

Run: `git diff --check`

Expected: both PASS.

- [ ] **Step 2: Run lint and MSRV checks**

Run: `cargo clippy -p trail --all-targets -- -D warnings`

Run: `cargo +1.81.0 check -p trail --all-targets`

Expected: PASS.

- [ ] **Step 3: Run the complete Trail suite**

Run: `cargo test -p trail --all-targets`

Expected: PASS, including all ACP unit, integration, E2E, fault, and interoperability tests.

- [ ] **Step 4: Run the offline conformance gate explicitly**

Run: `cargo test -p trail --test acp_conformance --test acp_faults -- --nocapture`

Expected: all 23 methods, every stable union/capability fixture, concurrency cases, and fault cases PASS.

Run: `scripts/test-acp-v1-reference-interop.sh`

Expected: PASS against the peer compiled from official ACP v1 Rust types.

Run: `cargo bench -p trail --bench acp_relay_bench`

Expected: byte/capture correctness PASS and p50/p95/p99 printed.

- [ ] **Step 5: Verify the compatibility claim from a clean build**

Build the release binary, run `target/release/trail agent acp doctor codex --json`, and confirm the report contains the exact pinned revision/digests and `evidence_status: "verified"` only when the conformance build identifier matches. Codex availability is a separate provider-readiness check. Run configured real-agent smoke tests; absent external agents remain skips and do not fail or satisfy the conformance gate.

- [ ] **Step 6: Review acceptance evidence against the design**

Check every item in `docs/superpowers/specs/2026-07-12-acp-v1-full-conformance-design.md` under “Acceptance Gates.” Record the command outputs and platform CI links in the final change/PR description. Do not declare completion if any gate is missing, skipped when required, or flaky.

- [ ] **Step 7: Commit verification-only corrections if present**

Stage each file corrected during verification by its explicit path, inspect `git diff --cached`, then run `git commit -m "fix: satisfy ACP v1 release gates"`. If verification required no code or documentation correction, do not create an empty commit.

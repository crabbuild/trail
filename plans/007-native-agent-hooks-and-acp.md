# Plan 007: Native Agent Hooks and ACP Integration

Status: DONE

Priority: P0

Size: XXL

Depends on: existing lane activity model and ACP relay

Design:

- [Native Agent Hooks and ACP Integration](../docs/design/native-agent-hooks-and-acp.md)

## Outcome

Trail records agent sessions through native provider hooks, ACP, or both without
duplicating lifecycle records. Codex, Claude Code, Pi, OpenCode, Cursor, Gemini CLI,
GitHub Copilot CLI, and Grok Build share one durable capture coordinator while keeping
provider-specific installation and parsing in narrow adapters.

The delivered system preserves provider-native evidence, creates exact turn and session
change manifests, correlates events to managed runs and Git history, survives retries and
crashes, and exposes the same state through Rust, CLI, HTTP/OpenAPI, and MCP surfaces.

## Execution rules

1. Read the design and this ledger before editing.
2. Keep provider adapters passive: they discover, install, parse, locate artifacts, and
   render responses; the shared coordinator owns lifecycle and checkpoint policy.
3. Make every ingress receipt durable and idempotent before semantic dispatch.
4. Treat ACP and native hooks as transports over one domain model.
5. Preserve canonical provider transcripts or exports whenever available.
6. Fail open for recording failures unless the user explicitly chooses strict capture.
7. Never overwrite or remove configuration Trail cannot prove it owns.
8. Update task status only with the acceptance evidence named below.

Status values are `TODO`, `IN PROGRESS`, `DONE`, `BLOCKED — reason`, and
`REJECTED — rationale`.

## Milestones

| ID | Deliverable | Depends on | Status |
| --- | --- | --- | --- |
| NAH-000 | Baseline, drift inventory, and task ledger | - | DONE |
| NAH-100 | Shared lifecycle contract and pure state machine | NAH-000 | DONE |
| NAH-200 | Durable capture schema and repository APIs | NAH-100 | DONE |
| NAH-300 | Receipt ingress, replay, runs, ownership, and recovery | NAH-200 | DONE |
| NAH-400 | Safe installation framework and adapter manifests | NAH-100 | DONE |
| NAH-500 | Eight production provider adapters and fixtures | NAH-300, NAH-400 | DONE |
| NAH-600 | Transcript, artifact, evidence, and checkpoint pipeline | NAH-300 | DONE |
| NAH-700 | Provenance, attestations, learnings, Git links, export | NAH-600 | DONE |
| NAH-800 | CLI, Rust, HTTP/OpenAPI, MCP, and diagnostics parity | NAH-300, NAH-500 | DONE |
| NAH-900 | Security, recovery, performance, compatibility, docs | all prior | DONE |

## NAH-000: Baseline and delivery map

- [x] NAH-001 Inspect Trail's ACP relay, lane activity storage, CLI, HTTP, MCP, schema,
  and test conventions.
- [x] NAH-002 Clone and inspect Entire for adapter inversion, native transcript capture,
  safe hook installation, snapshot/delta behavior, and Git association ideas.
- [x] NAH-003 Clone and inspect Atomic for its lifecycle state machine, managed-run
  correlation, evidence manifests, causal graph, attestations, and portable export.
- [x] NAH-004 Write the comprehensive design and link it from the documentation index.
- [x] NAH-005 Create this requirement-to-evidence implementation ledger.

Acceptance evidence:

- the design names every shared component, provider, storage table, public surface,
  recovery path, rollout phase, and release criterion;
- the reference repositories remain nested, clean clones and are not linked into the
  Trail build graph;
- every later task has an explicit dependency and test obligation.

## NAH-100: Shared lifecycle contract

### Domain model

- [x] NAH-101 Add versioned normalized event envelopes, provider/transport identity,
  native correlation identifiers, confidence, usage, content references, and bounded
  provider payload metadata.
- [x] NAH-102 Add the complete normalized vocabulary for sessions, turns, messages,
  plans, tools, approvals, subagents, compaction, usage/model changes, workspace
  changes, context injection, and diagnostics.
- [x] NAH-103 Add strict identifier, timestamp, size, and forward-version validation at
  mutation boundaries while retaining unknown provider events as inert evidence.
- [x] NAH-104 Define capture states, bounded transition context, ordered side-effect
  actions, and terminal outcome semantics.

### Pure transition function

- [x] NAH-111 Implement the pure `state + event + context -> state + actions`
  transition function with no database or filesystem access.
- [x] NAH-112 Cover missing start events, duplicate starts/ends, implicit turns, resume,
  cancellation, failure, late events, compaction, subagents, and unknown events.
- [x] NAH-113 Make terminal receipts enter `finalizing`; only a successful durable
  finalization completion can enter `ended` or `interrupted`.
- [x] NAH-114 Add an exhaustive state/event matrix test and property-style idempotency
  tests for every terminal transition.

### Coordinator boundary

- [x] NAH-121 Define the adapter-neutral capture coordinator interface and action
  executor boundary.
- [x] NAH-122 Refactor ACP capture to emit the shared lifecycle contract without
  regressing its existing streamed fidelity.
- [x] NAH-123 Add source-precedence and monotonic-enrichment rules for ACP, native
  hooks, transcripts, canonical exports, and reconstructed evidence.

Acceptance evidence:

- domain serialization golden tests are stable and versioned;
- all state/event pairs are tested and the transition module has no I/O imports;
- existing ACP unit and E2E tests pass unchanged or with explicitly reviewed richer
  assertions;
- replaying an identical normalized event produces no duplicate mutation action.

## NAH-200: Storage and repository APIs

### Schema

- [x] NAH-201 Add `agent_hook_installations` with ownership markers, config digests,
  provider versions, capability probes, scope, and last-success diagnostics.
- [x] NAH-202 Add `lane_agent_sessions` mapping provider-native sessions to Trail
  sessions, lanes, capture owner, managed run, lifecycle state, and finalization lease.
- [x] NAH-203 Add `agent_hook_receipts` with idempotency key, receive sequence, bounded
  raw payload reference, processing status, retry state, and error diagnostics.
- [x] NAH-204 Add `lane_artifacts` and `lane_turn_evidence_manifests` with immutable
  digest, trust, redaction, coverage, and exact file/change membership.
- [x] NAH-205 Add `agent_capture_runs` with renewable lease, canonical workdir scope,
  executor/provider constraints, and stamped output membership.
- [x] NAH-206 Add provenance nodes/edges, session attestations, revocation state, and
  learnings with factual/derived separation.
- [x] NAH-207 Add exact Git association records for Trail-created and externally
  observed commits without hidden branches or ambient-range attribution.
- [x] NAH-208 Provide transactional schema migration, rollback tests, completeness
  checks, indexes, foreign keys, and bounded retention queries.

### Repository layer

- [x] NAH-211 Add typed create/read/update APIs for installations, mappings, receipts,
  runs, artifacts, manifests, provenance, attestations, learnings, and Git links.
- [x] NAH-212 Enforce compare-and-set lifecycle transitions and renewable finalization
  leases under concurrent hook processes.
- [x] NAH-213 Allocate a monotonic receive sequence per native session and reject
  cross-workspace identity collisions.
- [x] NAH-214 Provide paginated, deterministic reports shared by CLI, HTTP, and MCP.

Acceptance evidence:

- migrations succeed from every supported schema version and rollback cleanly on
  injected failure;
- foreign-key and duplicate-key tests prove idempotency and exact ownership;
- concurrent writers cannot produce two active mappings or two finalizers;
- a database reopen preserves every in-flight receipt and lease.

## NAH-300: Ingress, replay, ownership, and recovery

### Durable ingress

- [x] NAH-301 Implement `trail agent hook receive PROVIDER EVENT` with bounded stdin,
  timeout, installation identity validation, and provider-compatible output.
- [x] NAH-302 Implement the matching daemon HTTP ingress with identical validation and
  response semantics.
- [x] NAH-303 Persist or securely spool the raw receipt before parsing or dispatch; use
  restrictive permissions, atomic publication, quotas, and deterministic filenames.
- [x] NAH-304 Add replay workers, retry/backoff, poison-receipt quarantine, operator
  inspection, and explicit retry/discard operations.
- [x] NAH-305 Retain unmapped provider events as redacted inert events.

### Managed runs and ownership

- [x] NAH-311 Implement managed-run begin/renew/end APIs and longest-canonical-workdir
  matching with provider/executor constraints.
- [x] NAH-312 Stamp only newly created native-session mappings; never retroactively
  capture an unrelated pre-existing session.
- [x] NAH-313 Implement owner selection among ACP, native hooks, terminal, and hybrid
  capture with stable correlation fallbacks and ambiguity diagnostics.
- [x] NAH-314 Implement native-session, turn, message, tool, and checkpoint idempotency
  keys plus source-precedence enrichment.

### Recovery

- [x] NAH-321 Implement finalization leases, crash takeover, transcript retry, artifact
  retry, workdir sync retry, and checkpoint retry.
- [x] NAH-322 Reconstruct incomplete turns from durable receipts plus native artifacts
  after daemon or plugin-buffer loss.
- [x] NAH-323 Reconcile expired managed runs and interrupted provider processes without
  silently marking incomplete sessions cleanly ended.

Acceptance evidence:

- killing Trail between receipt persistence and dispatch loses no event;
- 100 concurrent duplicate receipts create one semantic record;
- nested managed runs select the longest valid workdir match and ambiguity fails closed;
- hooks still return the provider success contract during bounded Trail degradation.

## NAH-400: Installation framework

- [x] NAH-401 Define the provider registry, stable canonical names/aliases, capabilities,
  discovery, version probing, transcript location, and response rendering traits.
- [x] NAH-402 Define a constrained, versioned adapter manifest for provider event names,
  merge fragments, generated assets, commands, response contracts, and supported
  provider versions.
- [x] NAH-403 Implement structural configuration merge preserving unknown fields,
  comments where the format permits, unrelated hooks/plugins, and user ordering.
- [x] NAH-404 Add ownership markers and before/after digests; removal may delete only
  exact Trail-owned entries or files.
- [x] NAH-405 Add repository and user scopes, canonical path validation, symlink/race
  defenses, restrictive file modes, atomic writes, rollback, and crash recovery.
- [x] NAH-406 Implement add/remove/list/status/doctor/events flows with dry-run and JSON.
- [x] NAH-407 Add manifest compatibility probes so unsupported provider versions report
  `partial`, `unavailable`, or `unknown` rather than guessing.

Acceptance evidence:

- golden merge/unmerge fixtures preserve unrelated configuration byte-for-byte where
  the format permits;
- install failure rolls back all Trail-owned changes;
- removal refuses modified ownership markers and reports the exact conflict;
- malicious paths, oversized config, symlinks, and concurrent installers fail safely.

## NAH-500: Provider adapters

Every provider must complete discovery, version probe, installation, parsing,
transcript/export collection, response rendering, capabilities, diagnostics, golden
fixtures, and coexistence tests. Unsupported native events remain inert evidence.

- [x] NAH-501 OpenAI Codex: notifications/native hooks where supported, transcript
  collection, managed launch fallback, and ACP correlation.
- [x] NAH-502 Anthropic Claude Code: settings merge, complete hook mapping, transcript
  offsets/snapshots, context injection, and hook response contract.
- [x] NAH-503 Pi: extension installation, lifecycle/tool/usage events, transcript and
  context integration.
- [x] NAH-504 OpenCode: plugin installation, fragment aggregation, todos/parts/tool
  mapping, durable acknowledgements, and transcript export.
- [x] NAH-505 Cursor: project/user hook configuration, available lifecycle/tool events,
  transcript capability detection, and explicit fidelity gaps.
- [x] NAH-506 Gemini CLI: settings/hooks merge, session/turn/tool/compaction mapping,
  transcript discovery, and context support.
- [x] NAH-507 GitHub Copilot CLI: supported hook/plugin wiring, lifecycle/tool mapping,
  transcript/export discovery, and version-gated capability reporting.
- [x] NAH-508 Grok Build: native contract probe and adapter where available plus a
  managed-wrapper first-class path with honest transcript/resume limitations.
- [x] NAH-509 Cross-provider alias, mixed-installation, upgrade, downgrade, removal, and
  hybrid ACP/native test matrix.

Acceptance evidence for each provider:

- provider-native golden receipts map to the expected normalized sequence;
- exact install/uninstall fixtures cover repository and user scopes;
- version probes identify the verified range and fidelity level;
- duplicate, late, missing, malformed, and oversized events are covered;
- canonical transcript/export digest tests pass when the provider exposes one.

## NAH-600: Evidence and checkpoint pipeline

- [x] NAH-601 Implement native artifact records for transcript, export, tool output,
  structured patch, plan, context, and provider metadata with trust/confidence.
- [x] NAH-602 Prefer canonical export, then stable native transcript, then explicitly
  marked reconstructed transcript.
- [x] NAH-603 Implement pre-turn offsets, immutable snapshots/deltas, digest validation,
  truncation detection, resume chaining, and bounded parser failures.
- [x] NAH-604 Separate immutable factual envelopes from redactable attachments and
  derived interpretations; never mutate a factual digest during redaction.
- [x] NAH-605 Capture exact turn-start workdir basis, structured/observed changes, and
  final workdir sync through existing lane patch/sync paths.
- [x] NAH-606 Create deterministic turn evidence manifests with exact file/change,
  receipt, message, tool, usage, artifact, and span membership.
- [x] NAH-607 Create session-end checkpoints only after durable finalization actions
  succeed; support partial/interrupted outcome and retry.

Acceptance evidence:

- transcript append, rewrite, truncation, resume, and missing-file fixtures are covered;
- redaction can remove attachment access without invalidating factual envelope hashes;
- exact file manifest tests exclude unrelated concurrent lane changes;
- a failed checkpoint remains retryable and does not transition to `ended`.

## NAH-700: Provenance and portable history

- [x] NAH-701 Persist the causal graph node and edge vocabulary from the design with
  exact source identifiers and deterministic ordering.
- [x] NAH-702 Implement rule-based activity classification as derived data linked to
  source nodes; never claim hidden reasoning or ambient attribution.
- [x] NAH-703 Build content-addressed session attestations over precisely enumerated
  evidence coverage, principal identity, previous attestation, and capture policy.
- [x] NAH-704 Implement local signing, verification, key rotation/revocation status,
  unsigned mode, and tamper diagnostics.
- [x] NAH-705 Implement explicit, reviewable learnings with source coverage, confidence,
  supersession, expiry, and opt-in context injection.
- [x] NAH-706 Implement portable versioned trace export/import with canonical ordering,
  digests, bounded attachments, compatibility validation, and no hidden Git branch.
- [x] NAH-707 Link Trail-created commits exactly and observe external commits only when
  exact checkpoint/tree/change identity proves the association.

Acceptance evidence:

- graph reconstruction is deterministic after shuffled receipt replay;
- tampering with any covered item fails attestation verification;
- revoked keys are reported without destroying historic evidence;
- export-import-export is byte-stable for the same supported version;
- concurrent unrelated commits are never attributed through time ranges alone.

## NAH-800: Public surfaces and diagnostics

### CLI and Rust

- [x] NAH-801 Implement `trail agent hooks add|remove|list|status|doctor|events` and the
  internal singular `trail agent hook receive` command.
- [x] NAH-802 Implement capture run, receipt replay, session/artifact/attestation,
  provenance, learning, Git-link, and portable export commands from the design.
- [x] NAH-803 Expose stable Rust request/report types backed by the same repository
  operations used by the CLI.

### HTTP/OpenAPI and MCP

- [x] NAH-811 Add HTTP endpoints for ingress and every inspect/mutate operation with
  pagination, authorization, bounded bodies, and consistent errors.
- [x] NAH-812 Update checked-in OpenAPI schemas/examples and contract tests.
- [x] NAH-813 Add MCP tools/resources with the same capability and report model; keep
  host-agent capture calls aligned with `begin_turn -> message -> span/event -> patch or
  sync -> assistant message -> end_turn`.

### Diagnostics

- [x] NAH-821 Report installation drift, provider compatibility, transcript support,
  capture owner, last receipt, spool pressure, replay failures, expired leases, partial
  finalizations, and fidelity gaps.
- [x] NAH-822 Make diagnostic output actionable and redact secrets in text and JSON.

Acceptance evidence:

- CLI/Rust/HTTP/MCP parity tests compare the same canonical reports;
- OpenAPI validation and MCP schema snapshots pass;
- all list surfaces are deterministic and paginated;
- no diagnostic renders raw secrets or unbounded provider payloads.

## NAH-900: Release hardening and completion audit

- [x] NAH-901 Run unit, integration, E2E, golden fixture, concurrency, crash/reopen,
  security, compatibility, and optional real-provider suites.
- [x] NAH-902 Add ingress latency, replay throughput, transcript size, database growth,
  and 100-concurrent-hook performance gates with documented budgets.
- [x] NAH-903 Add operator and user documentation for setup, capture modes, privacy,
  retention, troubleshooting, provider limitations, export, and recovery.
- [x] NAH-904 Add schema and adapter compatibility policy, manifest signing/distribution
  policy, deprecation windows, and upgrade/downgrade diagnostics.
- [x] NAH-905 Resolve every design open question in an ADR or mark it as an explicit
  versioned policy choice with compatibility behavior.
- [x] NAH-906 Perform a line-by-line design acceptance audit and link each requirement
  to code plus automated evidence in the completion matrix below.

## Completion matrix

This matrix is the final stop gate. No row may be marked complete solely because an API
or stub exists.

| Requirement | Implementation | Automated evidence | Status |
| --- | --- | --- | --- |
| Native capture works without ACP | receipt replay into Trail session/turn/message/span/checkpoint | durable lifecycle replay unit and native-hook CLI E2E | DONE |
| ACP retains current fidelity | typed lifecycle envelopes alongside streamed ACP events; Grok ACP profile | 12 ACP unit tests plus ACP relay E2E | DONE |
| Hybrid capture does not duplicate | exact ACP/native correlation with ACP lifecycle ownership and monotonic enrichment | hybrid native/ACP idempotency unit test | DONE |
| All nine providers are supported honestly | checked-in declarative adapters; unknown versions report `unknown` | 27 fixture cases plus nine-provider install matrix | DONE |
| Receipts survive retry and crash | durable object journal, bounded spool, retry/backoff/quarantine/discard | reopen/idempotency, stale-recovery, and spool-recovery tests | DONE |
| Exact turn/session file coverage | turn-start basis, observed/structured changes, immutable evidence membership | evidence-manifest and ACP workdir E2E tests | DONE |
| Canonical transcripts are preserved | export > native transcript > reconstructed fallback, with offsets and rewrite/truncation flags | canonical/rewrite/truncate/reconstruct test | DONE |
| Install/remove preserves user config | structural merge, exact markers, owned files, atomic rollback | nine-provider scope/idempotency, drift, symlink, and CLI tests | DONE |
| Finalization is leased and recoverable | renewable compare-and-set owner lease and stale takeover | competing-finalizer, expired-run, and replay tests | DONE |
| Facts, redactions, and derivations are separate | immutable manifest plus separately redactable attachments and derived nodes | digest, artifact-redaction, and classifier tests | DONE |
| Provenance is causal and exact | deterministic session-scoped source/activity nodes and `derived_from` edges | idempotent evidence/provenance test | DONE |
| Attestations cover enumerated evidence | content-addressed incremental chain over exact turn manifests | chain/sign/verify/revoke/tamper test | DONE |
| Learnings are explicit and reviewable | proposed/accepted/rejected records with anchors and opt-in injection | redaction and review-state test | DONE |
| Git association has no hidden branch | exact commit/change/session link records | full-object-ID and idempotency test | DONE |
| Portable trace round-trips | bounded canonical v1 export and verified import parser | export-import-export byte-stability test and CLI E2E | DONE |
| CLI/Rust/HTTP/MCP are consistent | shared report types, paginated routes/tools/resources, strict OpenAPI | receipt-ID and offset parity E2E plus OpenAPI/MCP contract tests | DONE |
| Security and resource bounds hold | auth, input/config/spool/artifact bounds, redaction, symlink and lock defenses | malformed/oversize/symlink/drift/auth/read-only tests | DONE |
| Upgrade and recovery are documented | operator guide, v1 policy choices, compatibility and recovery procedures | documentation and completion audit | DONE |

### Design acceptance audit

| # | Design acceptance criterion | Implementation and automated evidence | Result |
| --- | --- | --- | --- |
| 1 | Install every stable provider without ACP | `agent_hooks` registry/install planner and nine-provider project/user asset test | PASS |
| 2 | Direct sessions create searchable sessions/turns | native receipt replay repository test and Codex CLI E2E | PASS |
| 3 | Tools become correctly nested spans | root/tool/subagent/compaction span replay assertions | PASS |
| 4 | At most one checkpoint per changed turn | pure finalization actions plus duplicate-terminal and durable replay tests | PASS |
| 5 | Preserve transcript/export evidence | canonical precedence, offset, rewrite, truncation, and reconstruction test | PASS |
| 6 | Hybrid ACP/native capture deduplicates | exact correlation, ACP lifecycle ownership, message/event idempotency test | PASS |
| 7 | Hook failure is fail-open and diagnosable | database-outage spool/replay E2E and doctor diagnostics | PASS |
| 8 | Preserve foreign provider configuration | structural merge, drift refusal, idempotent uninstall, CLI E2E | PASS |
| 9 | Crash recovery does not duplicate semantics | durable receipt, stale processing, replay, and 100-writer tests | PASS |
| 10 | Enforce payload/path/ID/artifact security | hard size bounds, central redaction, symlink/outside-root/auth tests | PASS |
| 11 | Report capability/version limits honestly | compatibility probe and unknown-version provider reports | PASS |
| 12 | Preserve ACP and terminal compatibility | complete ACP unit/E2E suite and existing package regression suite | PASS |
| 13 | Query exact Git associations without hidden refs | exact full-object-ID Git link repository/CLI/HTTP/MCP surfaces | PASS |
| 14 | Managed runs stamp only governed sessions | longest-workdir, owner/executor, ambiguity, and expiry tests | PASS |
| 15 | Immutable turn manifest is separate from attachments | evidence manifest, artifact redaction, and digest-stability tests | PASS |
| 16 | Causal provenance links to factual sources | deterministic rule classifier and `derived_from` graph test | PASS |
| 17 | Session attestations exactly cover and chain turns | create/sign/verify/revoke/tamper/previous-attestation test | PASS |
| 18 | Redaction does not change factual/checkpoint identity | attachment redaction retains manifest and attestation identity | PASS |
| 19 | Adapters are previewable, gated, and reversible | dry-run plans, ownership manifests, compatibility diagnostics; remote manifests disabled in v1 | PASS |
| 20 | Portable trace is an export, not a second truth | canonical bounded export, verified import parser, byte-stable roundtrip | PASS |

## Shared validation gate

Before this plan can be marked `DONE`, run and record successful results for:

```sh
make fmt-check
cargo check -p trail
cargo test -p trail
make bench-cli-scale-smoke
```

Also run every provider fixture suite, schema migration suite, receipt crash/replay suite,
hybrid ACP/native E2E, HTTP/OpenAPI contract suite, MCP parity suite, and security test
listed above. Platform-specific real-provider evidence may be explicitly documented as
unavailable, but the corresponding adapter contract fixtures and honest capability
diagnostics are mandatory on every release platform.

### Latest implementation evidence

- `make fmt-check` and scoped `git diff --check`: passed.
- `cargo check -p trail`: passed in the shared repository.
- `cargo test -p trail` executed 279 library tests, 12 binary tests, and 183 E2E tests.
  The library and binary suites passed; 181 E2E tests passed in the aggregate run. The
  two failures were stale concurrent schema-v15 expectations (`14` instead of `15`),
  and both corrected migration tests passed exactly afterward. Combined result: all
  474 tests pass.
- `cargo test -p trail agent_capture --lib`: 29 tests passed, including lifecycle
  cross-product/idempotency, 100 concurrent writers under the 15-second/32-MiB budget,
  hybrid ownership, recovery, transcript rewrite/truncation, payload bounds, and
  transcript symlink/outside-root rejection.
- `cargo test -p trail agent_hooks --lib`: 14 tests passed, including the checked-in
  27-case provider fixture matrix and all-nine-provider project/user installation.
- ACP, evidence, OpenAPI, authenticated HTTP ingress, pagination parity, MCP read-only
  annotation, native spool recovery, schema-v12, and portable-trace E2E tests passed.
- `make bench-cli-scale-smoke`: passed at 1,000 files; results were written under
  `/tmp/trail-cli-scale-ci-smoke/1000`.
- Real-provider smoke tests were not run because they require provider installations,
  authentication, possible network access, and spend. Version compatibility therefore
  remains `unknown` unless proved by a checked-in adapter fixture/range, as required by
  the v1 policy.

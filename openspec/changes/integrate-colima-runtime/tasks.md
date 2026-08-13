## 1. Configuration and public model

- [x] 1.1 Add default-compatible runtime provider configuration and validated config list/get/set behavior.
- [x] 1.2 Add the typed runtime provider status/setup report and wire-compatible serialization tests.

## 2. Colima provider lifecycle

- [x] 2.1 Refactor OCI CLI execution to support explicit provider arguments and removal of ambient Docker endpoint variables.
- [x] 2.2 Implement workspace-derived Colima profile/context selection, prerequisite detection, contained startup, readiness verification, and bounded diagnostics.
- [x] 2.3 Route configured runtime reconciliation and cleanup through the selected provider while preserving ownership and digest checks.
- [x] 2.4 Reject Colima host-file secret mounts before container creation and leave the VM profile outside Trail's implicit cleanup authority.

## 3. CLI and contracts

- [x] 3.1 Add `trail env runtime provider status` and `setup colima` parsing, dispatch, and typed rendering.
- [x] 3.2 Add CLI and library regression tests for compatibility defaults, explicit context isolation, setup failure rollback, safe startup argv, and secret rejection.
- [x] 3.3 Align OpenAPI schemas or document why the workspace-only setup surface is intentionally CLI/Rust-only.

## 4. Documentation and verification

- [x] 4.1 Update runtime reference/design documentation, README guidance where appropriate, and the changelog with setup, rollback, security limits, and prerequisites.
- [x] 4.2 Run formatting, targeted runtime/config/CLI tests, workspace checks, and inspect the final diff without disturbing unrelated user changes.

## 5. Trail-managed Colima distribution

- [x] 5.1 Add platform-pinned Colima, Lima, and Docker CLI artifact manifests with immutable URLs, SHA-256 digests, size limits, and third-party notices.
- [x] 5.2 Implement safe global cache/data paths, bounded download and archive validation, executable verification, receipts, atomic publication, and concurrent setup convergence.
- [x] 5.3 Resolve a complete system toolchain first, fall back to an existing managed toolchain without network access, and provision only during explicit setup.
- [x] 5.4 Isolate managed Colima/Docker state, select macOS `vz`, and report system versus Trail-managed toolchain identity.
- [x] 5.5 Add failure, corruption, concurrency, fallback, and CLI contract tests without contacting upstream services.
- [x] 5.6 Update documentation and changelog for zero-install setup, supported platforms, cache/state lifecycle, licenses, and offline behavior.
- [x] 5.7 Validate OpenSpec, run formatting, targeted tests, workspace check/test/Clippy gates, and update the existing PR as a single coherent change.

## 6. Managed-execution configuration and model

- [x] 6.1 Add the default-compatible `host`/`colima` execution-backend configuration, validation, setup persistence, and rollback behavior.
- [x] 6.2 Extend typed runtime, managed-execution, and containment reports with backend, profile/instance, projection, checkpoint, and cleanup identity without exposing secrets.
- [x] 6.3 Add CLI parsing, config list/get/set behavior, JSON fixtures, and compatibility tests for backend selection and Colima setup preflight.

## 7. No-mount guest execution protocol

- [x] 7.1 Resolve the configured Colima profile to its explicit Lima instance and invoke only the verified `limactl` with isolated `LIMA_HOME` and bounded subprocess handling.
- [x] 7.2 Build deterministic, bounded lane-source projections that exclude private, ignored, reserved, and unsupported filesystem state while preserving accepted portable modes and symlinks.
- [x] 7.3 Create execution-scoped guest namespaces, stream and verify projections without host mounts, run argv without shell interpolation, and address the lane working directory explicitly.
- [x] 7.4 Export guest candidate state into host staging and validate entry count, byte limits, normalized paths, collisions, file kinds, modes, symlink targets, and containment before lane mutation.
- [x] 7.5 Compute and apply the validated candidate delta through the existing lane materialization barrier and checkpoint path, including unchanged and non-zero-exit cases.

## 8. Managed lifecycle, services, and limits

- [x] 8.1 Add a reusable execution-backend boundary beneath managed execution while preserving byte-for-byte compatible host behavior by default.
- [x] 8.2 Translate provider-neutral runtime service allocations and allowed environment-generation bindings into verified guest-local bindings without passing host paths or the Docker socket.
- [x] 8.3 Enforce bounded duration, stdout/stderr, projection size, file size, entry count, and concurrency with distinct timeout, cancellation, command-exit, validation, and infrastructure results.
- [x] 8.4 Integrate guest projection, execution, export, import, checkpoint, disposal, and cleanup into the existing managed-execution failure and finalization state machine.

## 9. Agent, gate, and public-interface integration

- [x] 9.1 Route library and CLI lane execution through the selected backend and expose deterministic human, JSON, and NDJSON output.
- [x] 9.2 Align HTTP/OpenAPI and MCP `trail.lane_exec` request, response, cancellation, backend, lifecycle, and error semantics with the shared library operation.
- [x] 9.3 Route readiness and verification gate commands through the same backend and attach their evidence to the gate and lane provenance records.
- [x] 9.4 Expose the guest managed-command capability to terminal and ACP agent workflows, associate session/turn/trace provenance, and report host control-plane versus guest data-plane containment honestly.

## 10. Provenance, cleanup, and recovery

- [x] 10.1 Extend preparation/finalization phases and receipts with toolchain, profile/instance, namespace, source/candidate digests, limits, service identities, exit classification, checkpoint, and cleanup fields.
- [x] 10.2 Make guest process and namespace cleanup bounded, ownership-checked, and idempotent across every success and failure edge without stopping the Colima profile.
- [x] 10.3 Add doctor/recovery reconciliation for interrupted projection, execution, export, import, checkpoint, and cleanup, failing closed on ambiguous or live guest ownership.
- [x] 10.4 Add adversarial redaction, path traversal, symlink escape, archive bomb, collision, secret, cancellation, crash, retry, and unrelated-profile preservation tests.

## 11. Documentation and verification

- [x] 11.1 Update the lane work-model, managed execution, environment runtime, CLI, HTTP, MCP, agent, security, recovery, and changelog documentation with setup, use cases, guarantees, boundaries, and rollback.
- [x] 11.2 Add host-compatibility and Colima guest protocol unit tests plus library, CLI, HTTP, MCP, gate, agent, checkpoint, recovery, and fault-injection integration coverage using fake local tools only.
- [x] 11.3 Validate OpenSpec, run formatting, targeted lane/runtime/managed-execution gates, workspace check/test/Clippy baselines, inspect the final diff, and update the existing single PR with verified and skipped evidence.

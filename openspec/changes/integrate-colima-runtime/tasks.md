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

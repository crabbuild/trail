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

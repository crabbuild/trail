## Why

Trail can already reconcile lane-private OCI services and prepare managed lane executions, but project commands still execute on the host. On macOS, Trail needs one contained, installation-free Colima boundary that can host both lane services and untrusted command execution while Trail remains authoritative for lane state, checkpoints, provenance, and recovery.

## What Changes

- Add first-class runtime-provider configuration for `auto`, `docker`, `podman`, and `colima`.
- Add a one-command Colima setup flow that selects a dedicated profile, starts it when requested, and verifies its Docker endpoint without activating it globally.
- Make setup installation-free on supported macOS hosts by downloading a Trail-pinned Colima, Lima, and Docker CLI toolchain when a complete compatible system toolchain is unavailable.
- Verify every managed artifact against a compile-time SHA-256 allowlist, publish it atomically into Trail's global cache, and retain third-party license notices alongside it.
- Run every Colima-backed OCI operation through the profile's explicit Docker context rather than ambient `DOCKER_HOST` or Docker context state.
- Create/start Trail's dedicated Colima profile with no host filesystem mounts, no SSH-agent forwarding, and no automatic Docker/Kubernetes context activation.
- Report the selected provider and Colima profile/context through typed Rust and CLI JSON output.
- Fail closed for Colima-backed file-secret mounts until a VM-safe secret broker exists.
- Preserve the current Docker/Podman auto-detection behavior for existing workspaces.
- Add an explicit managed-execution backend with compatibility-preserving `host` and opt-in `colima` values.
- Project only the lane-visible workspace into an execution-scoped directory inside the no-mount Colima VM, execute there, and import validated results through Trail's existing checkpoint path.
- Route CLI, HTTP, MCP, readiness-gate, and agent-managed project commands through the same backend contract rather than duplicating lane behavior at each surface.
- Make Colima-hosted lane services reachable from guest executions through backend-specific bindings without exposing the Docker socket or host loopback assumptions.
- Persist deterministic execution receipts covering the backend, profile, input/output identity, command result, limits, checkpoint, cleanup, and recovery outcome.
- Recover or fail closed on interrupted projection, execution, import, checkpoint, and guest-cleanup phases.

## Capabilities

### New Capabilities

- `colima-runtime-provider`: Safe setup, selection, lifecycle, reporting, lane-private OCI resources, and no-mount managed execution through a dedicated Colima/Lima profile.

### Modified Capabilities

None.

## Impact

- Affects workspace configuration, lane runtime reconciliation, managed execution, lane command and gate runners, agent containment reporting, CLI/HTTP/MCP contracts, typed reports, recovery, tests, reference documentation, and the changelog.
- Adds no linked Rust dependency and does not store third-party binaries in the repository or `trail` executable. Supported macOS users need no separate installation: Trail fetches pinned upstream release artifacts on explicit setup, while complete compatible system installations remain reusable.
- Existing configurations and runtime behavior remain compatible because the default provider remains `auto` and the default execution backend remains `host`; selecting `colima` makes the stronger execution boundary explicit.
- The agent provider process remains a contained host-side control plane unless a provider has a separately managed guest distribution. Its project command, test, service, and gate data plane can use the Colima backend through Trail's CLI, HTTP, or MCP interfaces without transferring broad host credentials into the VM.

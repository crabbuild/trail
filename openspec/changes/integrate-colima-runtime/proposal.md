## Why

Trail can already reconcile lane-private OCI services through Docker or Podman, but it depends on whichever daemon the user's ambient CLI happens to address. On macOS this makes a common Colima setup fragile and can accidentally target the wrong Docker context; Trail needs an explicit, contained provider lifecycle that works without global context changes.

## What Changes

- Add first-class runtime-provider configuration for `auto`, `docker`, `podman`, and `colima`.
- Add a one-command Colima setup flow that selects a dedicated profile, starts it when requested, and verifies its Docker endpoint without activating it globally.
- Run every Colima-backed OCI operation through the profile's explicit Docker context rather than ambient `DOCKER_HOST` or Docker context state.
- Create/start Trail's dedicated Colima profile with no host filesystem mounts, no SSH-agent forwarding, and no automatic Docker/Kubernetes context activation.
- Report the selected provider and Colima profile/context through typed Rust and CLI JSON output.
- Fail closed for Colima-backed file-secret mounts until a VM-safe secret broker exists.
- Preserve the current Docker/Podman auto-detection behavior for existing workspaces.

## Capabilities

### New Capabilities

- `colima-runtime-provider`: Safe setup, selection, lifecycle, reporting, and use of a dedicated Colima Docker profile for lane-private OCI runtime resources.

### Modified Capabilities

None.

## Impact

- Affects workspace configuration, lane runtime reconciliation, CLI environment-runtime commands, typed reports, OpenAPI schemas, tests, reference documentation, and the changelog.
- Adds no linked Rust dependency and does not vendor Lima or Colima. The external `colima` and `docker` executables remain separately installed, versioned, and invoked with bounded argument vectors.
- Existing configurations and runtime behavior remain compatible because the default provider remains `auto`.

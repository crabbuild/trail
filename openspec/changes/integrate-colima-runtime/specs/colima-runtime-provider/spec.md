## ADDED Requirements

### Requirement: Explicit runtime provider selection
Trail SHALL support workspace runtime provider values `auto`, `docker`, `podman`, and `colima`, and SHALL preserve `auto` as the default for workspaces without explicit runtime configuration.

#### Scenario: Existing workspace opens
- **WHEN** a workspace configuration has no runtime section
- **THEN** Trail selects `auto` and retains ambient Docker-then-Podman detection behavior

#### Scenario: Invalid provider is configured
- **WHEN** a caller attempts to configure an unsupported runtime provider value
- **THEN** Trail rejects the value without changing the active configuration

### Requirement: One-command Colima setup
Trail SHALL provide a workspace command that configures a Colima runtime profile, optionally starts it, verifies the profile-specific Docker endpoint, and returns a typed provider report.

#### Scenario: Setup starts a missing profile
- **WHEN** the user runs Colima setup with startup enabled and the workspace profile is not running
- **THEN** Trail starts the profile with the contained Trail flags, verifies its explicit Docker context, and persists the provider configuration

#### Scenario: Setup preflight fails
- **WHEN** Colima, Docker, profile startup, or Docker endpoint verification fails
- **THEN** Trail reports bounded actionable diagnostics and does not publish the requested provider configuration

#### Scenario: Setup without startup
- **WHEN** the user configures Colima with startup disabled
- **THEN** Trail verifies the required executables, persists autostart as disabled, and reports that the profile is not yet verified as running

### Requirement: Ambient-context independence
Trail SHALL execute every Colima-backed OCI operation through the configured profile's explicit Docker context and SHALL NOT change the user's active Docker context.

#### Scenario: Another Docker context is active
- **WHEN** Trail reconciles a Colima-backed runtime while another Docker context or `DOCKER_HOST` is active
- **THEN** all inspection, image, network, volume, container, start, stop, and remove operations target only the configured Colima context

### Requirement: Contained Colima startup
Trail SHALL start a managed Colima profile without host filesystem mounts, SSH-agent forwarding, host SSH configuration, Kubernetes, reachable bridged addressing, or automatic Docker/Kubernetes context activation.

#### Scenario: Autostart command is constructed
- **WHEN** a selected Colima profile is stopped and autostart is enabled
- **THEN** Trail invokes Colima with fixed argv flags enforcing the contained startup contract and without shell interpolation

### Requirement: Fail-closed Colima secret handling
Trail SHALL refuse Colima-backed container creation that requires host file-secret bind mounts until a VM-safe secret broker is available.

#### Scenario: Service declares a file secret
- **WHEN** reconciliation selects Colima for a runtime resource with one or more resolved file secrets
- **THEN** Trail fails before container creation and does not broaden the Colima host mount set

### Requirement: Provider lifecycle ownership
Trail SHALL retain its existing lane allocation, ownership-label, image-digest, health, stop, and cleanup checks regardless of runtime provider, and SHALL NOT implicitly stop or delete a Colima VM profile.

#### Scenario: Managed execution completes
- **WHEN** a command using Colima-backed runtime services finishes
- **THEN** Trail stops its owned lane containers according to existing lifecycle rules but leaves the Colima profile and unrelated resources intact

### Requirement: Typed and documented public contract
Trail SHALL expose selected provider, profile, explicit context, readiness, autostart, startup action, and containment status in Rust and CLI JSON output, and SHALL document setup, rollback, prerequisites, and limitations.

#### Scenario: Provider status is requested
- **WHEN** a caller requests environment runtime provider status
- **THEN** Trail returns the same typed report through Rust and CLI JSON with deterministic field meanings

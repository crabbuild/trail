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
Trail SHALL provide a workspace command that resolves or provisions a complete Colima runtime toolchain, configures a profile, optionally starts it, verifies the profile-specific Docker endpoint, and returns a typed provider report.

#### Scenario: Setup starts a missing profile
- **WHEN** the user runs Colima setup with startup enabled and the workspace profile is not running
- **THEN** Trail starts the profile with the contained Trail flags, verifies its explicit Docker context, and persists the provider configuration

#### Scenario: Setup preflight fails
- **WHEN** tool resolution or provisioning, profile startup, or Docker endpoint verification fails
- **THEN** Trail reports bounded actionable diagnostics and does not publish the requested provider configuration

#### Scenario: Setup without startup
- **WHEN** the user configures Colima with startup disabled
- **THEN** Trail resolves or provisions and verifies the required executables, persists autostart as disabled, and reports that the profile is not yet verified as running

### Requirement: Installation-free managed toolchain
Trail SHALL provision a complete pinned Colima, Lima, and Docker CLI toolchain during explicit setup on supported macOS architectures when a complete system toolchain is unavailable.

#### Scenario: No tools are installed
- **WHEN** setup runs on a supported macOS host without a complete system toolchain
- **THEN** Trail downloads the platform's immutable pinned artifacts, verifies their compiled SHA-256 digests, retains license notices and a receipt, and atomically publishes a ready managed toolchain without administrator access

#### Scenario: Artifact verification fails
- **WHEN** any managed artifact is oversized, truncated, has the wrong digest, contains an unsafe archive entry, or lacks its expected executable
- **THEN** Trail removes only its staging state, executes nothing from it, retains any prior published toolchain, and fails setup without changing workspace configuration

#### Scenario: Ordinary runtime operation lacks tools
- **WHEN** status, reconciliation, HTTP, MCP, or daemon behavior cannot find a complete system or previously published managed toolchain
- **THEN** Trail performs no network download and directs the caller to explicit setup

#### Scenario: Managed provisioning is unsupported
- **WHEN** setup lacks system tools on an unsupported operating system, architecture, or virtualization backend
- **THEN** Trail reports the supported managed platforms and manual prerequisite guidance without partially installing tools

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

#### Scenario: Trail-managed macOS toolchain starts
- **WHEN** the provisioned toolchain starts a profile on supported macOS
- **THEN** Trail selects the `vz` backend, prepends only the verified tool directory for child resolution, and isolates Colima and Docker configuration below Trail-owned user data

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
Trail SHALL expose selected provider, profile, explicit context, readiness, autostart, startup action, containment status, toolchain source, and pinned managed version in Rust and CLI JSON output, and SHALL document setup, rollback, cache/state locations, prerequisites, licenses, and limitations.

#### Scenario: Provider status is requested
- **WHEN** a caller requests environment runtime provider status
- **THEN** Trail returns the same typed report through Rust and CLI JSON with deterministic field meanings

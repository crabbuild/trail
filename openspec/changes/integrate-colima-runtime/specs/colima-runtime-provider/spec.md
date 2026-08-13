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
Trail SHALL expose selected provider, execution backend, profile, explicit context, readiness, autostart, startup action, containment status, toolchain source, and pinned managed version in Rust and CLI JSON output, and SHALL document setup, rollback, cache/state locations, prerequisites, licenses, and limitations.

#### Scenario: Provider status is requested
- **WHEN** a caller requests environment runtime provider status
- **THEN** Trail returns the same typed report through Rust and CLI JSON with deterministic field meanings

### Requirement: Explicit managed-execution backend
Trail SHALL support workspace managed-execution backend values `host` and `colima`, SHALL preserve `host` as the default for existing workspaces, and SHALL reject `colima` unless the selected provider and profile satisfy the contained Colima contract.

#### Scenario: Existing workspace executes a command
- **WHEN** a workspace has no configured execution backend
- **THEN** Trail uses the existing host managed-execution behavior without changing serialized lane or command results

#### Scenario: Colima execution is selected without a ready profile
- **WHEN** a managed command resolves `execution_backend = "colima"` but the dedicated profile or verified Lima tool is unavailable
- **THEN** Trail fails before command execution or source mutation with actionable setup guidance

#### Scenario: Setup enables contained execution
- **WHEN** successful Colima setup is requested with the Colima execution backend
- **THEN** Trail persists the provider and backend together only after the no-mount profile and guest-execution preflight succeed

### Requirement: No-mount lane projection
Trail SHALL project only the lane-visible managed workspace into an execution-scoped guest namespace and SHALL NOT mount or disclose the workspace root, original checkout, `.trail`, `.git`, host home, SSH agent, Docker socket, or unrelated lane state to the guest.

#### Scenario: Guest execution starts
- **WHEN** Trail prepares a Colima-backed managed command
- **THEN** it creates a uniquely owned guest namespace, imports a bounded deterministic source projection, verifies its manifest, and sets the guest working directory to the projected lane root

#### Scenario: Projection contains an unsafe entry
- **WHEN** projection encounters a reserved path, escaping symlink, unsupported file kind, collision, invalid normalized path, or configured size or entry limit
- **THEN** Trail executes no guest command, publishes no durable lane mutation, and removes only its owned staging and guest state

### Requirement: Same-VM lane service access
Trail SHALL execute Colima-backed managed commands in the same Lima instance whose Docker daemon owns the lane-private service allocations and SHALL derive guest-local service bindings without exposing the Docker socket.

#### Scenario: Lane command uses a private service
- **WHEN** managed execution has reconciled a healthy Colima-backed service
- **THEN** the guest command receives deterministic `TRAIL_SERVICE_*` and `TRAIL_SERVICES_JSON` bindings that reach that allocation from inside the VM

#### Scenario: Service binding cannot be verified
- **WHEN** a required service is healthy from the provider but its guest-local binding cannot be established
- **THEN** Trail fails execution before running the project command and retains the existing service lifecycle and diagnostic report

### Requirement: Trail-authoritative result import and checkpoint
Trail SHALL treat guest files as disposable candidate state, validate a bounded result projection, apply its delta through the lane materialization boundary, and use the existing managed-execution checkpoint operation as the only publication path.

#### Scenario: Guest command changes source files
- **WHEN** a guest command exits and its candidate projection passes validation
- **THEN** Trail imports the delta, checkpoints it as a lane operation with stable path and line identity, and returns the resulting root and operation in the managed-execution report

#### Scenario: Guest result is unchanged
- **WHEN** the candidate digest equals the input projection digest
- **THEN** Trail skips materialization writes and reports a successful execution with no checkpointed source change

#### Scenario: Guest result validation or apply fails
- **WHEN** the exported candidate violates path, type, size, symlink, ignore, private-path, or containment policy, or cannot be applied atomically
- **THEN** Trail leaves the durable lane root and ref unchanged, reports an infrastructure failure distinct from the command exit, and retains only bounded recovery evidence

### Requirement: Bounded guest process lifecycle
Trail SHALL execute guest commands without shell interpolation and SHALL bound duration, output, projection bytes, archive entries, individual files, and concurrent executions. Timeout and cancellation SHALL terminate the guest process group before result export.

#### Scenario: Command exits non-zero
- **WHEN** the guest process exits with a non-zero code without an infrastructure failure
- **THEN** Trail reports the exact bounded command result and may import valid source changes under the same checkpoint policy as host execution

#### Scenario: Command exceeds a limit
- **WHEN** execution, output, projection, or concurrency exceeds its configured bound
- **THEN** Trail terminates or rejects the execution, classifies the limit explicitly, and does not publish an unvalidated candidate

#### Scenario: Caller cancels execution
- **WHEN** CLI, HTTP, MCP, gate, or agent orchestration cancels a running guest command
- **THEN** Trail terminates the owned guest process group, records cancellation, performs idempotent cleanup, and leaves unrelated guest processes and the Colima profile running

### Requirement: Shared managed-command domain operation
Trail SHALL route CLI lane execution, HTTP and MCP lane-exec calls, readiness gates, and agent-managed project commands through the same library operation, backend selection, typed result, checkpoint, and error semantics.

#### Scenario: AI agent invokes lane execution
- **WHEN** an agent calls `trail.lane_exec` for a lane whose backend is Colima
- **THEN** Trail runs the project command in the guest while associating the result with the active lane, session, turn, trace, command fingerprint, and checkpoint provenance

#### Scenario: Agent provider remains host-side
- **WHEN** a terminal or ACP provider control process is launched on the host while managed commands use Colima
- **THEN** Trail reports host control-plane containment and guest data-plane containment separately and does not claim the provider binary ran inside the VM

#### Scenario: Equivalent interfaces execute the same request
- **WHEN** equivalent CLI, HTTP, or MCP requests select the same lane, root, command, and backend
- **THEN** their structured reports use aligned field meanings, lifecycle phases, and error codes

### Requirement: Durable provenance and recovery
Trail SHALL durably record the guest backend, profile and Lima instance, verified toolchain identity, source root, projection and candidate digests, guest namespace identity, limits, service allocation identities, command classification, checkpoint outcome, and cleanup status without persisting secret values.

#### Scenario: Execution completes normally
- **WHEN** checkpoint and cleanup finish
- **THEN** the final managed-execution receipt is terminal, references the resulting lane operation or unchanged root, and records that the guest namespace was removed

#### Scenario: Trail restarts after interruption
- **WHEN** recovery observes an execution interrupted during projection, execution, export, import, checkpoint, or cleanup
- **THEN** Trail reconciles durable phase and ownership evidence, resumes only an idempotent safe action, or fails closed with explicit guidance while preserving the last published lane root

#### Scenario: Stale guest namespace has ambiguous ownership
- **WHEN** doctor or recovery cannot bind a guest namespace to a terminal or abandoned Trail execution manifest
- **THEN** Trail does not delete it automatically and reports bounded manual recovery details

### Requirement: Guest cleanup preserves provider state
Trail SHALL remove only execution-scoped guest namespaces and processes it owns and SHALL NOT implicitly stop or delete the Colima profile, lane service allocations outside their existing lifecycle, or unrelated Lima state.

#### Scenario: Cleanup is retried
- **WHEN** finalization or recovery repeats cleanup for an already-removed guest namespace
- **THEN** cleanup succeeds idempotently and preserves the terminal execution receipt

#### Scenario: Cleanup command fails
- **WHEN** an owned guest namespace cannot be removed after bounded retries
- **THEN** Trail reports a recoverable cleanup blocker with the namespace identity and leaves the lane's durable checkpoint result unaltered

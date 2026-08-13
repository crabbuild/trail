## Context

Trail's lane environment model already separates immutable OCI image identity from lane-private runtime allocations. `workspace_runtime.rs` reconciles those allocations through a private `RuntimeProvider` trait, and the first part of this change adds an explicit Colima provider with a pinned managed toolchain and no-mount profile. Managed execution already owns environment discovery and sync, runtime-service reconciliation, layered lane-view mounting, execution phases, checkpointing, receipts, disposal, and unmounting. The remaining gap is that `exec_lane_workspace` and agent launch still spawn project processes on the host.

The integration crosses configuration, external process execution, filesystem projection, lane lifecycle, typed reports, recovery, CLI/HTTP/MCP transport, agent containment, and public documentation. Colima and Lima are independently released Go programs with VM images and large transitive supply-chain surfaces. Trail therefore needs both a bounded managed-toolchain contract and a Trail-owned guest-execution protocol rather than opaque binaries, persistent host mounts, or Lima's interactive synchronization policy.

## Goals / Non-Goals

**Goals:**

- Make Colima-backed lane service containers work through one Trail setup command and on-demand startup.
- Never depend on or mutate the user's ambient Docker context.
- Keep the Trail host authoritative for image verification, names, labels, ports, health, lifecycle, and cleanup.
- Use a workspace-specific Colima profile with no host mounts, SSH-agent forwarding, global context activation, Kubernetes, or reachable bridged address.
- Preserve deterministic provider selection and actionable fail-closed errors.
- Keep existing `auto` Docker/Podman behavior compatible.
- Require no manual Colima, Lima, Docker CLI, Homebrew, or administrator installation on supported macOS hosts.
- Pin, verify, atomically publish, and report the exact managed toolchain identity.
- Make managed commands and readiness gates runnable inside the same no-mount Colima VM as lane-private services.
- Keep durable Trail objects, refs, lane roots, line identity, sessions, turns, traces, approvals, and checkpoints authoritative across guest execution.
- Project a bounded lane snapshot into the guest and import candidate results only after path, type, size, and containment validation.
- Preserve deterministic cancellation, timeout, output-limit, cleanup, and crash-recovery behavior.
- Give AI agents a single CLI/HTTP/MCP managed-command capability whose receipts identify the exact sandbox and lane state used.

**Non-Goals:**

- Store third-party executables in Git or inside the `trail` executable.
- Automatically download tools during ordinary status, reconcile, daemon, HTTP, or MCP operations; network installation remains confined to explicit CLI/Rust setup.
- Manage QEMU on Linux or macOS versions that cannot use Apple's Virtualization framework in this change.
- Expose `.trail/`, the original checkout, or a Docker socket inside a lane command.
- Support Colima `containerd`, Kubernetes, Incus, remote daemons, or direct Lima command execution.
- Copy secret bytes into the VM. Colima-backed services with host file-secret bindings remain blocked until a separate broker is designed.
- Promise that every third-party agent provider binary is installed inside the guest. Trail keeps the provider control process in its existing contained host launcher unless a separately pinned guest distribution is available; the sandboxed data plane is the managed lane command/gate interface.
- Treat a guest filesystem as durable lane history or allow guest writes to bypass Trail validation and checkpointing.

## Decisions

### Colima is a host-owned OCI provider, not an environment adapter

Adapters continue to declare provider-neutral `oci` runtime resources. Provider selection occurs at reconciliation, below managed execution, so adapters cannot start VMs, choose contexts, or gain provider sockets. This reuses Trail's existing runtime allocation and cleanup invariants.

Alternative: implement Colima as an adapter plugin. Rejected because plugins are planners and deliberately have no provider or process authority.

### Use a pinned managed toolchain with system reuse

Trail first accepts a complete system `colima`, `limactl`, and `docker` toolchain. If any member is missing during explicit setup on supported macOS hosts, Trail downloads the complete pinned toolchain from immutable upstream release URLs. Colima and the Docker CLI are direct or single-file artifacts; the matching Lima distribution is unpacked with its required `share/lima` data. Every archive is size-bounded and checked against a SHA-256 digest compiled into Trail before extraction or publication.

Managed versions publish atomically below the user's Trail cache, never into `PATH`, `/usr/local`, Homebrew, or the workspace. Mutable VM and Docker configuration lives below Trail's user data directory, separately from the replaceable tool cache. A toolchain receipt identifies source, versions, platform, and manifest digest; bundled notices cover Colima's MIT license and Lima/Docker's Apache-2.0 licenses.

Ordinary provider detection is network-free: it may reuse an already-published managed toolchain but never repairs or downloads one. Missing or corrupt managed state directs the user back to explicit setup. Updating pins is a reviewed Trail release change; Trail never follows an unpinned `latest` URL.

Alternative: copy binaries into the Trail executable or repository. Rejected because it inflates every platform package, obscures third-party provenance, and prevents independent atomic replacement. The managed cache gives the same no-install user experience while retaining inspectable artifacts and receipts.

### Add explicit workspace runtime configuration

`TrailConfig` gains a defaulted runtime section with provider (`auto`, `docker`, `podman`, or `colima`), optional Colima profile, and Colima autostart. Missing fields deserialize to the compatibility defaults. A missing profile resolves to `trail-<workspace-id-prefix>`, avoiding collisions while remaining stable for a workspace.

`trail env runtime setup colima [--profile NAME] [--no-start]` resolves or provisions the toolchain, optionally starts/preflights the dedicated profile, and atomically persists the desired provider settings only after successful preflight. Read-only provider status is separately reportable and includes whether tools are system or Trail-managed.

### Address Colima through an explicit Docker context

Every Docker operation uses `docker --context <context> ...` and removes `DOCKER_HOST` from the child environment. The default Colima profile maps to context `colima`; other profiles map to `colima-<profile>`. Trail never calls `docker context use` and starts Colima with automatic activation disabled.

### Start with a contained profile contract

When autostart is required, Trail invokes Colima with fixed safe flags: Docker runtime, Apple's `vz` backend for a managed macOS toolchain, no mounts, no SSH-agent forwarding, no generated host SSH config, no Kubernetes, no reachable VM address, and no global context activation. `COLIMA_HOME` and `DOCKER_CONFIG` point at Trail-owned user data so even context metadata is isolated. Trail waits for `docker --context ... info` to succeed before publishing configuration or reconciling resources. Diagnostic output is bounded.

An already-running configured profile is accepted only when its explicit Docker context is healthy; the runtime receipt labels its containment as externally retained rather than claiming Trail verified historical startup flags. The workspace-derived default minimizes accidental adoption. A later hardening change may add cryptographically bound profile manifests if stronger adoption evidence is required.

### Block host-file secrets under no-mount Colima profiles

Colima's Docker daemon resolves bind-source paths inside the VM. Making arbitrary host secret paths visible would defeat the no-mount contract. The Colima provider therefore rejects nonempty resolved secret bindings before container creation with remediation to use Docker/Podman or a future broker.

### Add a managed-execution backend below every public command surface

Workspace runtime configuration gains a defaulted execution backend with `host` and `colima` values. `host` preserves existing behavior. `colima` is accepted only with the Colima provider and a ready contained profile. Backend selection happens inside managed execution after environment and service preparation, so CLI, HTTP, MCP, readiness gates, and agent workflows share one domain operation and one typed report.

The agent provider process remains the host-side control plane under Trail's existing scrubbed home and platform containment. Agents obtain the stronger data-plane boundary by calling `trail.lane_exec` through MCP, HTTP, or CLI; readiness policies use the same operation. Trail reports these two containment layers separately and never claims the provider process ran in the VM when it did not.

Alternative: add transport-specific Colima commands. Rejected because they would bypass the lane preparation, checkpoint, provenance, and recovery state machine.

### Reuse the Colima profile's underlying Lima instance

Colima profile names map deterministically to Lima instance names (`colima` for the default profile and `colima-<profile>` otherwise). Trail invokes the already-verified managed `limactl` with an explicit `LIMA_HOME` and instance name. Guest commands execute in the same VM whose Docker daemon owns lane service containers, so published service ports are guest-local and do not require exposing host loopback or a bridged VM address.

Trail does not invoke an ambient `ssh`, generated host SSH configuration, or a shell-constructed command. The executable and each argument remain separate, environment inheritance is allowlisted, and standard input/output/error are bounded by the existing process limits.

Alternative: create a second direct Lima VM for commands. Rejected because it duplicates lifecycle and disk cost and cannot treat Colima VM-local published service ports as local endpoints.

### Project and import; never mount the host lane

Each execution receives a random execution-scoped guest directory beneath a fixed Trail-owned guest root. Trail streams a deterministic, size-bounded archive of the lane-visible workspace into that directory. The archive excludes `.trail`, `.git`, ignored/private paths, sockets, devices, unsupported file kinds, and any path outside the lane view. It preserves only the portable modes and symlinks already accepted by Trail's path policy.

After execution, Trail streams a candidate archive back into a host staging directory owned by the managed-execution context. Before touching the lane view, Trail validates archive entry count, total and per-file size, relative NFC path policy, reserved paths, collisions, symlink targets, file kinds, and containment. It computes the candidate delta against the projected input, applies that delta through the existing lane materialization barrier, and invokes the existing checkpoint/finalization path. A failed validation or apply leaves the lane root and durable refs unchanged.

Trail deliberately does not use Lima `--sync`: its interactive accept/view/discard prompt and rsync merge policy would create a second authority for conflicts and publication. Trail owns acceptance and durable history.

### Translate service and environment bindings for the guest

Service allocation identity remains provider-neutral and durable. The host backend retains `127.0.0.1:<published-port>` bindings. The Colima backend rewrites only the execution environment so a service address resolves inside the Colima VM, while preserving service name, published port, allocation identity, and `TRAIL_SERVICES_JSON` schema. It never passes the Docker socket.

Host environment-generation output is projected read-only when it is inside Trail-owned generation roots and declared by the environment plan. Arbitrary host absolute paths and file-secret bindings are rejected. Writable caches are execution-scoped guest state and are not imported into source history unless the adapter explicitly declares a portable output already covered by Trail's environment contract.

### Journal the sandbox lifecycle and recover idempotently

Managed-execution phases expand to cover guest projection, guest execution, result export, candidate validation/import, checkpoint, and guest cleanup. The preparation receipt records the backend, profile/instance, toolchain identity, source root, projection digest, guest namespace, declared limits, and service bindings without secret values. Finalization records the output digest, exit classification, checkpoint operation, cleanup result, and any retained diagnostic namespace.

Guest directories are disposable projections, never sources of truth. Startup and doctor/recovery paths list only the configured instance's fixed Trail execution root, validate Trail-owned manifests, and remove stale namespaces whose durable execution is terminal or missing. Ambiguous ownership, a running process, or an uncheckpointed exported candidate fails closed and returns explicit recovery guidance. Cleanup is idempotent and never stops or deletes the Colima profile.

### Bound authority, time, and output

Projection/import bytes, archive entries, individual files, command output, execution duration, and concurrent guest executions use explicit limits. Timeout and cancellation terminate the guest process group before export. Non-zero command exit still permits importing valid source changes under existing managed-execution semantics; infrastructure, validation, cancellation, and cleanup failures remain distinct typed states. Secret values are removed before receipts, diagnostics, logs, HTTP, MCP, or CLI output.

## Risks / Trade-offs

- **First startup downloads a VM image and can take minutes** → Run only after the explicit setup command or when the workspace explicitly selects Colima with autostart; surface bounded diagnostics and status.
- **Managed installation is a supply-chain boundary** → Use immutable versioned URLs, compile-time SHA-256 pins, bounded downloads/extraction, atomic publication, receipts, and retained licenses; never execute an unverified stage.
- **Linux and older macOS need QEMU** → Keep system-toolchain support there and fail with actionable guidance; installation-free managed provisioning initially supports macOS arm64/x86_64 hosts capable of `vz`.
- **A running pre-existing profile may have broader historical settings** → Use a workspace-derived name, explicit context, and honest containment reporting; never claim its configuration was revalidated.
- **No host-file secrets with Colima** → Fail before container creation rather than expose home directories; retain existing Docker/Podman support for those declarations.
- **One VM per workspace consumes disk and memory** → Profiles are stable and reused; Trail stops only lane containers, not the VM, and never deletes profile data implicitly.
- **Docker/Colima output and schemas can change** → Depend primarily on exit status and the stable Docker CLI contract; keep parsing small, bounded, and tolerant of additive JSON fields.
- **Copy-based execution is slower than a host mount** → Use deterministic archive projection, skip unchanged imports by digest, retain the long-lived VM, and prefer safety over direct mutation; add scale limits and measurements.
- **A guest command may be killed between mutation and export** → Treat the guest directory as disposable, record the phase, and leave the durable lane root unchanged unless a fully validated candidate is imported and checkpointed.
- **Service addresses differ between host and guest** → Derive backend-local bindings from the same durable allocation report and verify them before execution.
- **Host-side agent providers can still expose their own host tools** → Keep existing platform containment, distinguish control-plane from data-plane enforcement in reports, and document that strict guest enforcement applies to Trail-managed commands and gates.

## Migration Plan

Existing workspaces deserialize the runtime section to `provider = "auto"` and `execution_backend = "host"`, preserving current behavior. Users opt in through `trail env runtime setup colima --execution-backend colima` or the corresponding configuration command. Rollback sets `runtime.execution_backend` to `host` and optionally `runtime.provider` to `auto`. Rollback does not delete or stop the Colima profile because that persistent external state requires an explicit user action.

Existing users with system tools continue using them. New users on supported macOS hosts receive the pinned managed toolchain during setup. Removing the cache is recoverable by rerunning setup; removing mutable VM state remains an explicit user action.

## Open Questions

- Whether Trail should broker runtime secrets through an in-guest ephemeral filesystem or Docker API upload without ever persisting bytes.
- Whether future strict-isolation readiness should require a signed/pinned Colima guest-image and profile manifest.
- Which agent providers should eventually gain separately pinned Linux guest distributions so their control process, not only their managed project-command data plane, can move into the VM.

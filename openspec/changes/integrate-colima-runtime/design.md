## Context

Trail's lane environment model already separates immutable OCI image identity from lane-private runtime allocations. `workspace_runtime.rs` reconciles those allocations through a private `RuntimeProvider` trait, but its only implementation probes ambient `docker` and `podman` CLIs. On macOS, Colima exposes Docker through a profile-specific context and normally changes the user's active context. Its default profile also mounts the user's home directory, which is broader host authority than a Trail runtime provider needs.

The integration crosses configuration, external process execution, lane lifecycle, typed reports, CLI parsing/rendering, and public documentation. Colima and Lima are independently released Go programs with VM images and large transitive supply-chain surfaces. Trail therefore needs a bounded managed-toolchain contract rather than placing opaque executables in Git or statically linking them into the Rust binary.

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

**Non-Goals:**

- Store third-party executables in Git or inside the `trail` executable.
- Automatically download tools during ordinary status, reconcile, daemon, HTTP, or MCP operations; network installation remains confined to explicit CLI/Rust setup.
- Manage QEMU on Linux or macOS versions that cannot use Apple's Virtualization framework in this change.
- Run the lane's agent or arbitrary managed command inside a VM in this change.
- Expose `.trail/`, the original checkout, or a Docker socket inside a lane command.
- Support Colima `containerd`, Kubernetes, Incus, remote daemons, or direct Lima command execution.
- Copy secret bytes into the VM. Colima-backed services with host file-secret bindings remain blocked until a separate broker is designed.

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

## Risks / Trade-offs

- **First startup downloads a VM image and can take minutes** → Run only after the explicit setup command or when the workspace explicitly selects Colima with autostart; surface bounded diagnostics and status.
- **Managed installation is a supply-chain boundary** → Use immutable versioned URLs, compile-time SHA-256 pins, bounded downloads/extraction, atomic publication, receipts, and retained licenses; never execute an unverified stage.
- **Linux and older macOS need QEMU** → Keep system-toolchain support there and fail with actionable guidance; installation-free managed provisioning initially supports macOS arm64/x86_64 hosts capable of `vz`.
- **A running pre-existing profile may have broader historical settings** → Use a workspace-derived name, explicit context, and honest containment reporting; never claim its configuration was revalidated.
- **No host-file secrets with Colima** → Fail before container creation rather than expose home directories; retain existing Docker/Podman support for those declarations.
- **One VM per workspace consumes disk and memory** → Profiles are stable and reused; Trail stops only lane containers, not the VM, and never deletes profile data implicitly.
- **Docker/Colima output and schemas can change** → Depend primarily on exit status and the stable Docker CLI contract; keep parsing small, bounded, and tolerant of additive JSON fields.

## Migration Plan

Existing workspaces deserialize the new runtime section to `provider = "auto"`, preserving current behavior. Users opt in through `trail env runtime setup colima`; rollback is `trail config set runtime.provider auto`. Rollback does not delete or stop the Colima profile because that persistent external state requires an explicit user action.

Existing users with system tools continue using them. New users on supported macOS hosts receive the pinned managed toolchain during setup. Removing the cache is recoverable by rerunning setup; removing mutable VM state remains an explicit user action.

## Open Questions

- Whether a later direct Lima backend should use one VM per workspace or an execution-scoped VM pool.
- Whether Trail should broker runtime secrets through an in-guest ephemeral filesystem or Docker API upload without ever persisting bytes.
- Whether future strict-isolation readiness should require a signed/pinned Colima guest-image and profile manifest.

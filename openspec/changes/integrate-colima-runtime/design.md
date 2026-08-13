## Context

Trail's lane environment model already separates immutable OCI image identity from lane-private runtime allocations. `workspace_runtime.rs` reconciles those allocations through a private `RuntimeProvider` trait, but its only implementation probes ambient `docker` and `podman` CLIs. On macOS, Colima exposes Docker through a profile-specific context and normally changes the user's active context. Its default profile also mounts the user's home directory, which is broader host authority than a Trail runtime provider needs.

The integration crosses configuration, external process execution, lane lifecycle, typed reports, CLI parsing/rendering, and public documentation. Colima and Lima are independently released Go programs with VM images and large transitive supply-chain surfaces, so statically embedding or vendoring them into the Rust binary would make Trail responsible for a second virtualization distribution and update channel.

## Goals / Non-Goals

**Goals:**

- Make Colima-backed lane service containers work through one Trail setup command and on-demand startup.
- Never depend on or mutate the user's ambient Docker context.
- Keep the Trail host authoritative for image verification, names, labels, ports, health, lifecycle, and cleanup.
- Use a workspace-specific Colima profile with no host mounts, SSH-agent forwarding, global context activation, Kubernetes, or reachable bridged address.
- Preserve deterministic provider selection and actionable fail-closed errors.
- Keep existing `auto` Docker/Podman behavior compatible.

**Non-Goals:**

- Vendor, download, upgrade, delete, or redistribute Colima, Lima, Docker, or VM images.
- Run the lane's agent or arbitrary managed command inside a VM in this change.
- Expose `.trail/`, the original checkout, or a Docker socket inside a lane command.
- Support Colima `containerd`, Kubernetes, Incus, remote daemons, or direct Lima command execution.
- Copy secret bytes into the VM. Colima-backed services with host file-secret bindings remain blocked until a separate broker is designed.

## Decisions

### Colima is a host-owned OCI provider, not an environment adapter

Adapters continue to declare provider-neutral `oci` runtime resources. Provider selection occurs at reconciliation, below managed execution, so adapters cannot start VMs, choose contexts, or gain provider sockets. This reuses Trail's existing runtime allocation and cleanup invariants.

Alternative: implement Colima as an adapter plugin. Rejected because plugins are planners and deliberately have no provider or process authority.

### Use external executables instead of embedding projects

Trail resolves `colima` and `docker` as separate argv-based executables, reports missing prerequisites, and never invokes a shell. This preserves independent installation and security updates and avoids adding large Go/VM assets to the Trail release.

Alternative: bundle Colima/Lima binaries or source. Rejected because platform signing, VM image distribution, CVE response, licenses, and update cadence would become Trail release responsibilities.

### Add explicit workspace runtime configuration

`TrailConfig` gains a defaulted runtime section with provider (`auto`, `docker`, `podman`, or `colima`), optional Colima profile, and Colima autostart. Missing fields deserialize to the compatibility defaults. A missing profile resolves to `trail-<workspace-id-prefix>`, avoiding collisions while remaining stable for a workspace.

`trail env runtime setup colima [--profile NAME] [--no-start]` validates prerequisites, optionally starts/preflights the dedicated profile, and atomically persists the desired provider settings only after successful preflight. Read-only provider status is separately reportable.

### Address Colima through an explicit Docker context

Every Docker operation uses `docker --context <context> ...` and removes `DOCKER_HOST` from the child environment. The default Colima profile maps to context `colima`; other profiles map to `colima-<profile>`. Trail never calls `docker context use` and starts Colima with automatic activation disabled.

### Start with a contained profile contract

When autostart is required, Trail invokes Colima with fixed safe flags: Docker runtime, no mounts, no SSH-agent forwarding, no generated host SSH config, no Kubernetes, no reachable VM address, and no global context activation. Trail waits for `docker --context ... info` to succeed before publishing configuration or reconciling resources. Diagnostic output is bounded.

An already-running configured profile is accepted only when its explicit Docker context is healthy; the runtime receipt labels its containment as externally retained rather than claiming Trail verified historical startup flags. The workspace-derived default minimizes accidental adoption. A later hardening change may add cryptographically bound profile manifests if stronger adoption evidence is required.

### Block host-file secrets under no-mount Colima profiles

Colima's Docker daemon resolves bind-source paths inside the VM. Making arbitrary host secret paths visible would defeat the no-mount contract. The Colima provider therefore rejects nonempty resolved secret bindings before container creation with remediation to use Docker/Podman or a future broker.

## Risks / Trade-offs

- **First startup downloads a VM image and can take minutes** → Run only after the explicit setup command or when the workspace explicitly selects Colima with autostart; surface bounded diagnostics and status.
- **A running pre-existing profile may have broader historical settings** → Use a workspace-derived name, explicit context, and honest containment reporting; never claim its configuration was revalidated.
- **No host-file secrets with Colima** → Fail before container creation rather than expose home directories; retain existing Docker/Podman support for those declarations.
- **One VM per workspace consumes disk and memory** → Profiles are stable and reused; Trail stops only lane containers, not the VM, and never deletes profile data implicitly.
- **Docker/Colima output and schemas can change** → Depend primarily on exit status and the stable Docker CLI contract; keep parsing small, bounded, and tolerant of additive JSON fields.

## Migration Plan

Existing workspaces deserialize the new runtime section to `provider = "auto"`, preserving current behavior. Users opt in through `trail env runtime setup colima`; rollback is `trail config set runtime.provider auto`. Rollback does not delete or stop the Colima profile because that persistent external state requires an explicit user action.

## Open Questions

- Whether a later direct Lima backend should use one VM per workspace or an execution-scoped VM pool.
- Whether Trail should broker runtime secrets through an in-guest ephemeral filesystem or Docker API upload without ever persisting bytes.
- Whether future strict-isolation readiness should require a signed/pinned Colima guest-image and profile manifest.

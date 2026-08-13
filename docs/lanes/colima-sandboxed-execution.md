# Colima-Sandboxed Lane Execution

Trail can keep its lane database, mounts, agent coordination, and checkpoints on
the host while running managed lane commands inside the Lima virtual machine
owned by a contained Colima profile. This is an optional data-plane backend;
existing workspaces continue to execute on the host.

## When it helps

Use the Colima backend when a lane command may execute untrusted repository
code, install dependencies, invoke compilers, or contact lane-private services.
It gives these commands a VM boundary without giving the guest a host mount,
the Docker socket, `.trail/`, the real Git directory, the user's home, or
ambient credentials.

Typical uses include:

- an AI agent asking Trail to run a repository test, formatter, generator, or
  build command;
- readiness test and eval gates that should use the same environment as agent
  commands;
- several lanes using isolated service containers in one contained Colima VM;
- reviewing the exact source delta produced by a non-zero or timed-out command;
  and
- retaining command, projection, checkpoint, cleanup, session, turn, and trace
  evidence for review or handoff.

The coding-agent process itself remains a contained host control plane. It can
use Trail through CLI, HTTP, or MCP, while `trail.lane_exec` and gate commands
run as the guest data plane. Trail reports this split explicitly; it does not
claim that an arbitrary host agent binary moved into the VM.

## Setup

On supported macOS hosts, setup can install Trail's pinned Colima, Lima, and
Docker CLI tools without Homebrew or a separate Colima installation:

```sh
trail env runtime setup colima --execution-backend colima
trail env runtime provider status
```

Trail starts a dedicated profile with no host mounts, SSH-agent forwarding,
generated host SSH configuration, bridged address, Kubernetes, or global
context activation. Linux and unsupported macOS hosts can use the same backend
when compatible `colima`, `limactl`, and `docker` executables are already
available. Colima still downloads and owns its guest image; Trail does not
embed that image in the `trail` binary.

`--no-start` can provision tools for later use, but it cannot enable the Colima
execution backend because Trail must verify the running guest first.

## Execution flow

```text
host Trail control plane
  resolve/sync services and mount lane view
  build deterministic bounded source projection
                    |
                    | tar stream through verified limactl (no host mount)
                    v
contained Colima/Lima guest
  create execution namespace -> run argv -> export candidate -> delete namespace
                    |
                    | bounded candidate stream
                    v
host Trail control plane
  validate archive -> reject concurrent host edits -> import source delta
  -> checkpoint lane operation -> dispose owned services -> unmount
```

Each execution uses a workspace- and execution-derived directory below
`/tmp/trail-executions` in the guest. Arguments are passed directly without
shell interpolation. Environment values are allowlisted and host lane paths are
translated to the guest workspace. Runtime service bindings remain on guest
loopback because Colima's Docker daemon and the managed command share the same
VM; Docker sockets and host paths are never injected.

The projection and candidate import are bounded by the lane workspace-view
entry, total-byte, and single-file limits (or conservative defaults). Paths are
normalized and checked for traversal, case collisions, unsupported entry kinds,
and escaping symlinks. Secret-class, Trail-internal, and real Git state are
excluded. Only validated source changes return to the lane; generated,
dependency, and scratch output remains disposable. A concurrent host source
change makes import fail closed.

## Commands and agent use

```sh
# Default guest timeout is 3600 seconds; accepted range is 1..86400.
trail lane exec fix-login --timeout-secs 900 -- cargo test

# Associate the checkpoint with an existing open turn.
trail lane exec fix-login --turn turn_... -- cargo test

# From another terminal, cancel the lane's only live guest execution.
trail lane exec-cancel fix-login

# Select an execution when several commands are live.
trail lane exec-cancel fix-login --execution-id exec_...

# Return to compatible host execution.
trail config set runtime.execution_backend host
```

HTTP `POST /v1/lanes/{lane}/exec` and MCP `trail.lane_exec` accept the same
`command`, optional `turn_id`, and optional `timeout_secs`. When `turn_id` is
present, Trail rejects an ended or cross-lane turn before launch and reports its
session, turn, and derived trace identity in the lifecycle receipt. This is the
recommended path for AI hosts: open a turn, call `trail.lane_exec` for command
work, inspect its structured lifecycle/checkpoint result, run gates, then end
the turn.

CLI `lane exec-cancel`, HTTP `POST /v1/lanes/{lane}/exec/cancel`, and MCP
`trail.lane_exec_cancel` expose the same cancellation operation. The optional
`execution_id` is required only when more than one cancellable execution is
live for the lane. Trail writes the cancellation request before acting,
terminates only that execution's recorded guest process group, skips candidate
import, cleans its owned namespace, and retains `terminal_cancelled` evidence.
The original blocking request returns `EXECUTION_CANCELLED` (CLI exit 17, HTTP
409, and the same structured MCP error). Cancellation never stops the Colima
profile or unrelated guest processes.
HTTP lane execution is dispatched on a workspace-scoped daemon worker, leaving
the authenticated listener responsive to the matching cancellation request.
Because one MCP stdio connection processes requests in order, an agent cancels
a blocking MCP execution from a second Trail MCP session (or through the CLI or
HTTP endpoint). The cancellation operation and durable receipt are identical.

The result distinguishes `succeeded`, `command_failed`, and `timed_out`;
cancellation is a distinct structured error rather than a command exit.
Non-zero and timed-out commands still proceed through candidate validation,
source checkpointing, service disposal, namespace cleanup, and lane unmount.
Candidate validation and infrastructure failures are returned as distinct
`EXECUTION_VALIDATION_FAILED` (CLI exit 18 / HTTP 422) and
`EXECUTION_INFRASTRUCTURE_FAILED` (CLI exit 19 / HTTP 503) Trail errors rather
than being disguised as command exits.

## Recovery and boundaries

Trail writes private execution manifests under `.trail/managed-executions/`.
Before another guest command it identifies live owners, safely removes abandoned
pre-import namespaces, and refuses to guess after ambiguous execute/export/import
states. `trail doctor` reports live, recoverable, ambiguous, and terminal receipt
counts without mutating them. Completed receipts are retained in a bounded set.

If doctor reports an ambiguous execution, inspect the lane and its current
workdir first. Checkpoint intentional source with `trail lane checkpoint
<LANE>`, then retry only after resolving the preserved state. Do not edit the
receipt or delete the guest namespace manually.

The backend does not provide a general-purpose remote shell, persist arbitrary
guest build output, expose host secrets, or stop/delete the Colima profile
during lane cleanup. File-secret OCI mounts remain unsupported. VM image trust,
kernel isolation, and Colima vulnerabilities remain part of the local Colima
trust boundary.

# Strict Agent Execution Envelope Design

**Date:** 2026-07-28
**Status:** Approved
**Scope:** Terminal agents, ACP agents, `lane exec`, lane tests and evals, managed hooks, and managed build commands

## Summary

Trail lanes currently separate recorded work, workdirs, history, review evidence,
and merge coordination. They do not form an enforced process boundary. On macOS,
the terminal-agent sandbox denies writes under the workspace and then grants writes
to the entire `.trail` subtree. That subtree contains sibling workdirs, workspace
views, database state, refs, caches, and other internal state. On platforms without
the macOS launcher, terminal agents run without an equivalent boundary.

Trail will replace these per-surface launch rules with one typed, cross-platform
`ExecutionEnvelope`. Strict isolation becomes the default. A managed process receives
only its current lane workdir, private runtime directories, declared read-only inputs,
and a narrow lane-scoped Trail capability. Direct access to the original workspace,
sibling lanes, Trail internals, host user state, and undeclared services is denied.

If the selected platform cannot enforce the envelope, Trail refuses to launch. An
explicit permissive compatibility mode remains available, but it is visibly marked
unsafe and cannot produce strict-isolation readiness evidence.

## Goals

1. Prevent one lane's process tree from reading or mutating sibling lanes.
2. Prevent managed processes from directly accessing Trail's database, refs, views,
   caches, tokens, and other internal state.
3. Prevent managed processes from mutating the original workspace or parent Git
   checkout.
4. Remove ambient host HOME, temp, XDG, credential, and tool-state inheritance.
5. Give terminal agents, ACP agents, `lane exec`, tests, evals, hooks, and managed
   build commands the same isolation contract.
6. Constrain the complete process tree and finish teardown before checkpointing or
   unmounting.
7. Allow independent lane commands to execute concurrently while keeping Trail
   evidence transactions short, serialized, retryable, and idempotent.
8. Record enough non-secret provenance to prove which isolation contract protected a
   gate, checkpoint, review, or handoff.
9. Fail closed whenever strict enforcement cannot be established or retained.

## Non-goals

1. This design does not require every Trail user to install Docker or Podman.
2. It does not turn Trail into a general container orchestrator.
3. It does not provide confidentiality against the host administrator or the user
   account that owns the Trail workspace.
4. It does not make unmanaged commands run outside Trail's execution surfaces safe.
5. It does not grant an agent arbitrary Trail CLI access from inside the sandbox.
6. It does not retroactively certify historical task evidence as strictly isolated.
7. It does not solve cross-host or remote multi-tenancy.

## Security Contract

### Isolation modes

Trail supports two explicit modes:

| Mode | Behavior |
| --- | --- |
| `strict` | Deny-by-default envelope. Launch fails if every required boundary cannot be enforced. This is the default. |
| `permissive` | Compatibility behavior with weaker host access. It must be explicitly selected and is never represented as strictly isolated. |

There is no automatic fallback from `strict` to `permissive`.

### Strict-mode authority

A strict process may receive:

- read/write access to its exact lane workdir;
- read/write access to lane-private HOME, temp, XDG, and declared private state;
- read-only access to required system runtimes and explicitly declared external roots;
- access to explicitly bound immutable dependency layers;
- access to explicitly declared lane-private services;
- a private, authenticated, lane-scoped Trail IPC capability; and
- only the environment variables included in the normalized envelope.

A strict process must not receive:

- access to the original workspace or parent Git checkout;
- access to sibling materialized workdirs;
- access to sibling layered-view uppers or mountpoints;
- direct access to `.trail`, including the SQLite database, refs, views, caches,
  journals, tokens, sockets, configuration, and capture state;
- host HOME, host temp, host XDG state, undeclared package-manager state, SSH state,
  cloud credentials, or arbitrary inherited environment variables;
- access to unrelated lane services or host loopback ports; or
- permission to create a descendant outside Trail's managed process tree.

### Threat model

The boundary must resist both accidental path confusion and a deliberately hostile
agent processing untrusted repository content, prompts, tool output, or dependency
metadata. Required adversarial cases include absolute paths, `..`, symlinks, hard
links, bind or mount tricks where available, case aliases, Unicode aliases, file
replacement races, inherited descriptors, detached descendants, and attempts to use
Trail's own IPC for another lane.

The boundary protects lane integrity, Trail integrity, and the confidentiality of
other lane-local data from managed agent processes. It does not protect against a
compromised kernel, host administrator, or direct commands launched outside Trail.

## Architecture

### `ExecutionEnvelope`

All managed execution surfaces construct the same platform-neutral value:

```rust
pub struct ExecutionEnvelope {
    pub schema_version: u32,
    pub execution_id: String,
    pub workspace_id: String,
    pub lane_id: String,
    pub task_id: Option<String>,
    pub turn_id: Option<String>,
    pub source_root: ObjectId,
    pub workdir: PathBuf,
    pub isolation_mode: IsolationMode,
    pub filesystem: FilesystemPolicy,
    pub environment: EnvironmentPolicy,
    pub network: NetworkPolicy,
    pub process: ProcessPolicy,
    pub trail_capability: TrailCapabilityPolicy,
    pub provenance: ExecutionProvenance,
}
```

The concrete model may use existing Trail IDs and report types, but the envelope is a
single immutable contract after validation. Launch code must not append ambient grants
after validation.

### Filesystem policy

`FilesystemPolicy` contains canonical, typed grants:

- `ReadOnly(path, purpose)`
- `ReadWrite(path, purpose)`
- `Execute(path, purpose)`
- `PrivateDirectory(path, lifecycle)`
- `Denied(path, reason)`

Validation must:

1. resolve every existing path without following an untrusted final symlink;
2. bind grants to stable filesystem identity where the platform supports it;
3. reject overlapping grants whose effective permission is ambiguous;
4. reject any grant that contains or aliases Trail internals, the source workspace,
   a sibling lane, or an undeclared external root;
5. reject non-normalized, non-absolute, case-ambiguous, or Unicode-ambiguous paths;
6. revalidate identities immediately before sandbox entry; and
7. record normalized grants without exposing secret values.

The current lane workdir is writable. Required system paths and declared external
roots are read-only. Everything else is denied.

### Environment policy

Managed processes start from `env_clear()`. Trail adds an explicit allowlist containing:

- a sanitized `PATH` assembled from approved executable roots;
- lane-private HOME, TMPDIR/TMP/TEMP, and XDG directories;
- lane and execution identifiers that are safe to expose;
- environment-generation bindings and typed cache bindings;
- declared toolchain variables;
- late-bound secret handles, never secret values in persisted provenance; and
- the private Trail IPC descriptor or socket identifier.

Inherited repository selectors, Git selectors, compiler injection variables, dynamic
loader variables, shell startup variables, agent credentials, and cloud credentials
are denied unless a typed component policy explicitly grants them.

### Network policy

Strict mode denies network access by default. Components may declare:

- no network;
- selected Unix-domain or named-pipe endpoints;
- selected lane-private service identities; or
- explicit external destinations under a separately reviewable policy.

Loopback is not implicitly trusted. Lane-private services must use a private network
namespace, authenticated proxy, or capability socket so another lane cannot discover
and connect to the host port directly.

### Process policy

Every managed launch creates a process-tree authority:

- Unix process group plus platform containment such as a cgroup where available;
- parent-death behavior where supported;
- Windows Job Object for the full tree;
- wall-time, process-count, file-descriptor, memory, and CPU budgets;
- controlled inherited descriptors; and
- deterministic cancellation, kill, and wait behavior.

Checkpointing, unmounting, capability revocation completion, and final gate publication
must wait until the complete process tree is stopped.

### Platform sandbox backends

One `ExecutionSandbox` interface consumes a validated envelope:

```rust
trait ExecutionSandbox {
    fn preflight(&self, envelope: &ExecutionEnvelope) -> Result<SandboxProof>;
    fn launch(
        &self,
        envelope: &ExecutionEnvelope,
        command: &CommandSpec,
        proof: &SandboxProof,
    ) -> Result<ManagedProcessTree>;
}
```

Initial backends:

| Platform | Enforcement |
| --- | --- |
| macOS | `sandbox-exec` profile generated from deny-by-default grants, plus managed process-group teardown |
| Linux | Existing Landlock filesystem enforcement generalized for agents, seccomp for prohibited networking and privilege operations, plus process-tree authority |
| Windows | Existing AppContainer grant model generalized for agents, plus Job Object teardown |

The current macOS `(allow default)` profile and blanket `.trail` write grant are
removed. Non-macOS launches may not bypass enforcement by directly executing the child.

Containers remain a future higher-isolation backend implementing the same interface.
They are not required for the initial strict contract.

### Lane-scoped Trail capability broker

Trail remains outside the sandbox and owns all direct database and internal filesystem
access. A managed process receives a short-lived capability through an inherited file
descriptor, private Unix-domain socket, or Windows named pipe. Environment variables
may identify the descriptor but must not carry a reusable bearer secret.

The capability is bound to:

- workspace identity;
- lane ID;
- task and turn IDs where present;
- execution ID;
- process-tree identity;
- allowed operation set;
- issue and expiry times;
- monotonic request sequence; and
- revocation state.

Initial allowed operations are intentionally narrow:

- read current-lane status;
- append a bounded current-lane event;
- upload a bounded scoped artifact or output blob;
- request current-lane checkpoint or record;
- report progress or cancellation; and
- query declared lane-private runtime bindings.

Requests naming another lane, ref, task, turn, workspace, or unrestricted path are
rejected before any mutation. Capabilities are not accepted by the general daemon HTTP
or MCP surface unless those surfaces implement the same binding checks.

The broker records bounded security events for rejected requests without persisting
secret material or attacker-controlled unbounded payloads.

## Execution Lifecycle

Every managed surface uses the following state machine:

1. Resolve the exact lane head, source root, task, turn, and workdir.
2. Discover and synchronize the required environment generation.
3. Build the `ExecutionEnvelope`.
4. Validate grants and create lane-private runtime directories.
5. Persist an idempotent `execution_preparing` record.
6. Mint the lane capability.
7. Preflight the platform sandbox and persist its proof summary.
8. Revalidate filesystem identities.
9. Launch the managed process tree.
10. Publish `execution_started` in a short transaction.
11. Capture bounded events and outputs while the process runs.
12. Cancel or wait for completion.
13. Stop and wait for every descendant.
14. Revoke the capability.
15. Record outputs and the terminal execution result.
16. Checkpoint source changes when the calling workflow requires it.
17. Remove ephemeral private state and publish `execution_completed`.

No database write lock is held while the child command executes.

### Crash recovery

Recovery is keyed by `execution_id` and phase:

- a prepared but unlaunched execution revokes its capability and removes ephemeral
  state;
- a launched execution with a live matching owner is not stolen;
- a launched execution with a dead owner is terminated through platform authority
  before checkpointing;
- a completed child with unpublished evidence replays the exact terminal receipt
  idempotently;
- a lost or unverifiable process-tree identity produces `isolation_lost` and blocks
  readiness; and
- capability revocation is idempotent and survives Trail restart.

Wall-clock age alone never authorizes takeover of a live execution.

## Gates and Concurrency

Lane tests and evals currently need short write transactions to create and close turns
and events. Contending commands may receive `WORKSPACE_LOCKED` even though their child
commands could run independently.

The new gate flow separates authority from execution:

1. Begin an idempotent gate receipt in a short bounded transaction.
2. Release the workspace lock.
3. Run the child under its envelope.
4. Reacquire authority with bounded jittered retry.
5. Publish the immutable output objects and terminal gate receipt exactly once.

Retries use the execution and receipt IDs as idempotency keys. They never re-run the
child command merely because final publication contended. Independent lane gates may
run concurrently. Same-lane policies may serialize when they would race the same
workdir or turn.

## User-Facing Behavior

### CLI

Managed launch commands accept:

```text
--isolation strict|permissive
```

The workspace default is:

```text
execution.isolation = "strict"
```

Strict mode reports:

- isolation level;
- platform backend;
- envelope schema;
- execution ID;
- private runtime roots;
- capability operation classes;
- network class;
- process-tree containment class; and
- sandbox proof status.

Permissive mode prints a prominent warning in human output and includes
`isolation_level: "permissive"` in JSON.

### Readiness and review

Readiness blocks strict claims when:

- the latest required gate is permissive or isolation-unverified;
- the recorded envelope does not match the current lane source or environment
  generation;
- process-tree teardown is incomplete;
- capability revocation is incomplete;
- sandbox proof is missing, stale, or invalid;
- an `isolation_lost` event remains unresolved; or
- required platform conformance evidence is absent.

Historical tasks and gates remain readable but are labeled `isolation_unverified`.
They do not become strict evidence retroactively.

## Error Contract

New stable errors:

| Code | Meaning |
| --- | --- |
| `ISOLATION_UNAVAILABLE` | The selected platform cannot enforce the requested strict envelope. |
| `EXECUTION_ENVELOPE_INVALID` | The requested grants, environment, or policy are unsafe or inconsistent. |
| `SANDBOX_PREFLIGHT_FAILED` | Platform sandbox setup failed before launch. |
| `SANDBOX_IDENTITY_CHANGED` | A granted path or executable changed between validation and launch. |
| `LANE_CAPABILITY_DENIED` | A broker request exceeded the capability's lane or operation authority. |
| `LANE_CAPABILITY_REVOKED` | A process used a capability after revocation or expiry. |
| `PROCESS_TREE_NOT_TERMINATED` | Trail could not prove that every descendant stopped. |
| `ISOLATION_LOST` | Trail lost or could not verify the enforcement boundary after launch. |

None of these errors trigger automatic permissive fallback.

## Persistence and Provenance

Persist an execution record with:

- execution and envelope schema versions;
- lane, task, session, and turn identity;
- source root and environment generation;
- normalized non-secret grant summaries;
- sandbox backend and proof digest;
- isolation level;
- network and process policy classes;
- capability operation classes and revocation result;
- child exit, timeout, and process-tree teardown result;
- gate or checkpoint receipt IDs; and
- terminal status.

Secret values, raw tokens, unrestricted host paths, and unbounded diagnostics are never
persisted.

Schema migrations are additive before the strict default changes. Readers must preserve
historical tasks with an explicit `isolation_unverified` projection.

## Rollout

1. Add the envelope model, validation, reports, persistence, and error contracts.
2. Generalize the existing restricted-command sandbox code behind
   `ExecutionSandbox`.
3. Move lane tests, evals, and `lane exec` to the envelope.
4. Add short idempotent gate receipt transactions and contention retry.
5. Move managed hooks and build commands.
6. Move terminal agents.
7. Move ACP upstream and session processes.
8. Add the lane-scoped capability broker and remove direct `.trail` access.
9. Run the shared adversarial conformance suite on macOS, Linux, and Windows.
10. Change the default to strict only when all supported-platform release gates pass.
11. Retain explicit permissive mode with permanent provenance and readiness warnings.

During development, strict mode may exist behind an experimental feature flag. The
released default must not change until the complete platform matrix passes. Once the
default changes, a missing backend is a launch error rather than a compatibility
fallback.

## Verification Strategy

### Shared adversarial conformance suite

Run the same contract against terminal agents, ACP agents, `lane exec`, tests, and
evals:

1. Lane B cannot read, write, truncate, delete, rename, or enumerate lane A.
2. Lane B cannot access lane A through symlinks, hard links, case aliases, Unicode
   aliases, open descriptors, or path-replacement races.
3. A process cannot access the original workspace or parent Git checkout.
4. A process cannot access Trail's database, refs, views, caches, journals, tokens, or
   daemon control state.
5. Host HOME, temp, XDG state, and undeclared secret variables are absent.
6. Only declared executable and read-only system paths are visible.
7. A copied or replayed capability cannot address another lane or execution.
8. Revoked and expired capabilities fail without mutation.
9. Detached descendants are terminated before checkpoint, unmount, or gate completion.
10. A sibling cannot connect to another lane's private runtime service.
11. Sandbox setup and identity races fail before child execution.
12. Trail or broker death cannot broaden filesystem or database authority.

### Concurrency and recovery

1. Four and 64 distinct lane executions run concurrently without `WORKSPACE_LOCKED`.
2. Concurrent start and finish receipts publish exactly once.
3. Same-lane conflicting executions follow an explicit serialization policy.
4. Killing Trail at every durable lifecycle phase recovers without orphan authority.
5. Killing the child, sandbox helper, or process-tree owner leaves no surviving
   descendant.
6. Capability revocation and output publication are idempotent under retries.
7. Database, filesystem, and sandbox failures never activate permissive behavior.

### Platform release evidence

macOS, Linux, and Windows must each run:

- the full shared adversarial suite;
- native process-tree teardown tests;
- native filesystem identity-race tests;
- capability broker tests;
- 64-lane concurrency tests; and
- the repository-wide regression suite.

Unsupported platforms may expose permissive mode but must reject strict launch.

## Acceptance Criteria

The change is complete only when:

- strict isolation is the default for every managed execution surface;
- no managed process receives a blanket `.trail` grant;
- sibling-lane and Trail-internal access fails in real process probes;
- the original workspace remains unchanged under hostile probes;
- ambient host user state and undeclared secrets are absent;
- all descendants terminate before finalization;
- capabilities authorize only the current lane and permitted operation classes;
- independent lane gates run concurrently without user-visible lock failures;
- readiness distinguishes strict, permissive, and historical unverified evidence;
- every supported platform passes the same conformance contract; and
- the complete repository test suite is green.

## Alternatives Considered

### Patch only the macOS profile

Removing the `.trail` grant on macOS is a useful emergency mitigation, but it leaves
Linux, Windows, ACP, gates, environment inheritance, and descendant lifetime
inconsistent. It is insufficient as the product architecture.

### Require containers for every agent

Containers can provide a stronger backend and should remain possible through
`ExecutionSandbox`. Requiring them initially would add runtime dependencies, reduce
native tool compatibility, and complicate macOS and Windows workflows. The existing
native sandbox foundations can enforce the initial contract without making containers
mandatory.

### Rely on claims, readiness, and merge review

Claims and merge review coordinate cooperative agents. They detect some damage after
the fact but cannot prevent cross-lane reads, writes, metadata tampering, or misleading
provenance. They are complementary controls, not an isolation boundary.

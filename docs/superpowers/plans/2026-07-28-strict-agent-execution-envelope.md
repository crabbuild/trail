# Strict Agent Execution Envelope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every Trail-managed command run inside a strict, lane-scoped, fail-closed execution envelope that prevents access to sibling lanes, Trail internals, the original workspace, ambient host state, and unmanaged descendants.

**Architecture:** Add a platform-neutral `execution` subsystem that owns envelope construction, validation, sandbox launch, process-tree lifetime, capability brokerage, and durable receipts. Migrate each execution surface onto that subsystem in dependency order, then enable strict isolation by default only after macOS, Linux, and Windows pass the same adversarial conformance suite.

**Tech Stack:** Rust 2024, rusqlite/SQLite, serde/CBOR/JSON, macOS Seatbelt via `sandbox-exec`, Linux Landlock and seccomp, Windows AppContainer and Job Objects, existing Trail lane/session/gate models, Clap CLI, GitHub Actions.

## Global Constraints

- Follow the approved design in `docs/superpowers/specs/2026-07-28-strict-agent-execution-envelope-design.md`.
- Use implementation-first sequencing: implement production code before adding its regression tests. Do not use red/green TDD steps.
- The completed change uses strict isolation by default; permissive behavior requires
  explicit `--isolation permissive`. Keep the compatibility default during Tasks 1-11
  and perform the user-visible strict-default cutover only in Task 12 after the native
  conformance gates pass.
- Never fall back automatically from strict to permissive.
- Start every managed child with `env_clear()` and an explicit normalized allowlist.
- Never grant a managed child direct access to `.trail`, the original workspace, or a sibling lane.
- Hold no workspace write lock while a child process executes.
- Persist no capability secret, secret value, or unbounded attacker-controlled diagnostic.
- Every task ends with focused verification and a single-purpose commit.
- Preserve all pre-existing untracked workspace files and unrelated user changes.

## File Structure

Create the execution subsystem under `trail/src/execution/`:

| File | Responsibility |
| --- | --- |
| `mod.rs` | Stable internal exports and `ExecutionManager` entry point |
| `model.rs` | Envelope, policies, modes, phases, proofs, receipts, and reports |
| `validate.rs` | Canonical grant validation, alias rejection, and pre-launch identity revalidation |
| `environment.rs` | Private HOME/temp/XDG creation and normalized `env_clear()` allowlist |
| `sandbox.rs` | `ExecutionSandbox` trait and platform backend selection |
| `sandbox_macos.rs` | Deny-by-default Seatbelt profile generation |
| `sandbox_linux.rs` | Landlock/seccomp invocation and enforcement |
| `sandbox_windows.rs` | AppContainer grant preparation and Job Object integration |
| `process.rs` | Managed process tree, timeout, cancellation, kill, and wait |
| `lifecycle.rs` | Prepare/start/finish/recover state machine |
| `capability.rs` | Lane-scoped capability issue, authorize, sequence, expire, and revoke |
| `broker.rs` | Private descriptor/socket protocol and bounded request dispatch |
| `store.rs` | SQLite execution, capability, and gate-receipt persistence |

Keep surface-specific orchestration in its existing module. Those modules call
`ExecutionManager`; they do not construct platform sandbox commands themselves.

---

### Task 1: Define the execution contract, configuration, reports, and stable errors

**Context:** Current isolation decisions are spread across terminal-agent, ACP, gate,
workspace-view, and restricted-recipe code. This task creates the names and types that
every later task consumes without changing process launch behavior.

**Files:**
- Create: `trail/src/execution/mod.rs`
- Create: `trail/src/execution/model.rs`
- Modify: `trail/src/lib.rs`
- Modify: `trail/src/model.rs`
- Modify: `trail/src/model/domain/config.rs`
- Modify: `trail/src/model/reports/lane.rs`
- Modify: `trail/src/model/lane/core.rs`
- Modify: `trail/src/model/lane/activity.rs`
- Modify: `trail/src/error.rs`
- Modify: `trail/src/db/util/config/entries.rs`
- Modify: `trail/src/db/util/config/set.rs`
- Test: unit tests beside the modified model, config, and error modules

**Interfaces:**
- Produces: `IsolationMode`, `ExecutionEnvelope`, policy/grant types,
  `ExecutionPhase`, `SandboxProof`, `ExecutionReceipt`, `ExecutionIsolationSummary`,
  and the eight stable error variants.
- Consumes: existing `ObjectId`, workspace/lane/task/turn IDs, `TrailConfig`, and
  report serialization conventions.

- [ ] **Step 1: Add the platform-neutral model**

Implement these exact public-in-crate contracts in `execution/model.rs`:

```rust
pub const EXECUTION_ENVELOPE_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum IsolationMode { Strict, Permissive }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    ReadOnly,
    ReadWrite,
    Execute,
    PrivateDirectory { lifecycle: PrivateDirectoryLifecycle },
    Denied { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FilesystemGrant {
    pub path: PathBuf,
    pub access: FilesystemAccess,
    pub purpose: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentPolicy {
    pub variables: BTreeMap<String, String>,
    pub private_home: PathBuf,
    pub private_tmp: PathBuf,
    pub private_xdg_config: PathBuf,
    pub private_xdg_cache: PathBuf,
    pub private_xdg_data: PathBuf,
    pub private_xdg_state: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NetworkPolicy { DenyAll, LaneServices(Vec<String>) }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProcessPolicy {
    pub wall_time_secs: u64,
    pub max_processes: u32,
    pub max_open_files: u32,
    pub memory_bytes: Option<u64>,
    pub cpu_seconds: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrailCapabilityPolicy {
    pub operations: BTreeSet<CapabilityOperation>,
    pub ttl_secs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutionProvenance {
    pub producer: String,
    pub command_sha256: String,
    pub environment_generation: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
    pub filesystem: Vec<FilesystemGrant>,
    pub environment: EnvironmentPolicy,
    pub network: NetworkPolicy,
    pub process: ProcessPolicy,
    pub trail_capability: TrailCapabilityPolicy,
    pub provenance: ExecutionProvenance,
}
```

Add `PrivateDirectoryLifecycle` (`Ephemeral`, `RetainedOnFailure`),
`CapabilityOperation`, and `ExecutionPhase` with `preparing`, `preflighted`, `started`,
`stopping`, `completed`, `failed`, and `isolation_lost`. Add `SandboxProof` with
backend, platform, envelope digest, grant-identity digest, containment class, network
class, and preflight time. Add `ExecutionReceipt` with execution ID, phase, source and
environment generations, exit/timeout state, capability revocation state,
process-tree termination state, proof, bounded output object IDs, failure code, and
completion time. Add `ExecutionIsolationSummary` with isolation mode, execution ID,
envelope schema/digest, backend, proof status, network class, containment class,
capability revocation, and process-tree termination.

- [ ] **Step 2: Add the staged isolation configuration**

Add `ExecutionConfig { isolation: IsolationMode }` to `TrailConfig`, expose
`execution.isolation`, and accept only `strict` or `permissive` in `config set`.
For Tasks 1-11, deserialize missing configuration as `Permissive` so an intermediate
commit cannot silently change existing launch behavior before all surfaces and native
backends are ready. Task 12 changes this one default to `Strict` after the exact-SHA
platform gates pass. Explicit `strict` requests are fail-closed throughout.

- [ ] **Step 3: Extend reports without breaking historical JSON**

Add `#[serde(default)] isolation: ExecutionIsolationSummary` to
`WorkspaceExecReport`, `LaneTestReport`, `LaneTestSummary`, `LaneReadinessReport`,
`AgentTaskReport`, and `AgentRunReport`. Make the default summary
`isolation_level = "unverified"` so historical rows do not become strict evidence.

- [ ] **Step 4: Add stable errors**

Add concrete variants and code mappings for:

```text
ISOLATION_UNAVAILABLE
EXECUTION_ENVELOPE_INVALID
SANDBOX_PREFLIGHT_FAILED
SANDBOX_IDENTITY_CHANGED
LANE_CAPABILITY_DENIED
LANE_CAPABILITY_REVOKED
PROCESS_TREE_NOT_TERMINATED
ISOLATION_LOST
```

Use exit code `17` for isolation/envelope failures and `18` for capability failures;
include those values in error unit tests and CLI structured-error fixtures.

- [ ] **Step 5: Add contract tests after implementation**

Test serde round trips, the temporary compatibility default, explicit strict and
permissive parsing, historical report defaults, config entry/set behavior, and every
error code/exit-code pair. Task 12 replaces the compatibility-default assertion with
the final strict-default assertion.

Run:

```bash
cargo test -p trail model::execution
cargo test -p trail db::util::config
cargo test -p trail error::tests
```

Expected: all selected tests pass.

- [ ] **Step 6: Commit**

```bash
git add trail/src/execution/mod.rs trail/src/execution/model.rs \
  trail/src/lib.rs trail/src/model.rs \
  trail/src/model/domain/config.rs trail/src/model/reports/lane.rs \
  trail/src/model/lane/core.rs trail/src/model/lane/activity.rs \
  trail/src/error.rs trail/src/db/util/config/entries.rs \
  trail/src/db/util/config/set.rs
git commit -m "feat: define strict execution envelope contract"
```

### Task 2: Add schema v21 and durable execution stores

**Context:** Strict evidence, crash recovery, capability revocation, and idempotent gate
publication require durable state. Current schema version is 20; this task performs one
additive migration and provides store methods without launching children.

**Files:**
- Create: `trail/src/execution/store.rs`
- Modify: `trail/src/execution/mod.rs`
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/storage/schema.rs`
- Modify: `trail/src/db/storage/schema/ddl.rs`
- Modify: `trail/src/db/storage/mod.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: `trail/src/lib.rs` test-support exports
- Create: `trail/tests/schema_v21_managed_executions.rs`

**Interfaces:**
- Consumes: Task 1 execution models.
- Produces: `prepare_execution`, `mark_execution_preflighted`,
  `mark_execution_started`, `finish_execution`, `mark_isolation_lost`,
  `recoverable_executions`, capability row operations, and idempotent gate receipts.

- [ ] **Step 1: Define exact v21 DDL**

Add `MANAGED_EXECUTIONS_V21` containing:

```sql
CREATE TABLE managed_executions (
  execution_id TEXT PRIMARY KEY,
  lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
  session_id TEXT REFERENCES lane_sessions(session_id),
  task_id TEXT,
  turn_id TEXT,
  source_root TEXT NOT NULL,
  environment_generation TEXT,
  envelope_json TEXT NOT NULL,
  envelope_sha256 TEXT NOT NULL,
  isolation_level TEXT NOT NULL CHECK(isolation_level IN ('strict','permissive')),
  filesystem_summary_json TEXT NOT NULL,
  network_class TEXT NOT NULL,
  process_class TEXT NOT NULL,
  capability_operations_json TEXT NOT NULL,
  backend TEXT,
  sandbox_proof_sha256 TEXT,
  phase TEXT NOT NULL CHECK(phase IN
    ('preparing','preflighted','started','stopping','completed','failed','isolation_lost')),
  owner_pid INTEGER,
  owner_start_identity TEXT,
  capability_revoked INTEGER NOT NULL DEFAULT 0 CHECK(capability_revoked IN (0,1)),
  process_tree_terminated INTEGER NOT NULL DEFAULT 0 CHECK(process_tree_terminated IN (0,1)),
  gate_receipt_id TEXT,
  checkpoint_receipt_id TEXT,
  exit_code INTEGER,
  failure_code TEXT,
  terminal_receipt_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  completed_at INTEGER
);
CREATE INDEX managed_executions_lane_phase
  ON managed_executions(lane_id, phase, updated_at);

CREATE TABLE execution_capabilities (
  capability_id TEXT PRIMARY KEY,
  execution_id TEXT NOT NULL REFERENCES managed_executions(execution_id) ON DELETE CASCADE,
  token_sha256 TEXT NOT NULL UNIQUE,
  operations_json TEXT NOT NULL,
  next_sequence INTEGER NOT NULL DEFAULT 1 CHECK(next_sequence > 0),
  issued_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  revoked_at INTEGER
);

CREATE TABLE lane_gate_receipts (
  receipt_id TEXT PRIMARY KEY,
  execution_id TEXT NOT NULL UNIQUE REFERENCES managed_executions(execution_id),
  lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
  turn_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('test','eval')),
  state TEXT NOT NULL CHECK(state IN ('started','completed','failed')),
  result_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
```

Increment `TRAIL_SCHEMA_VERSION` to 21, add v20-to-v21 migration, validate exact
`sqlite_master` shape, and keep v18/v19/v20 migrations converging on v21.
The JSON summary columns contain canonical, size-bounded, non-secret projections; they
must not contain capability tokens, environment values, raw unrestricted host paths,
or child stderr.

- [ ] **Step 2: Implement store compare-and-set transitions**

Each phase transition must use `UPDATE ... WHERE execution_id=? AND phase=?`; require
exactly one affected row. `finish_execution` must atomically set terminal phase,
revocation, process termination, exit/failure fields, and completion time.

- [ ] **Step 3: Implement idempotent receipt and capability stores**

Use immutable execution and receipt IDs as replay keys. Duplicate writes with identical
canonical JSON return the existing row; different payloads return `Corrupt`.
Capability authorization atomically checks expiry/revocation and increments
`next_sequence`.

- [ ] **Step 4: Add migration and store tests after implementation**

Cover fresh v21, v20 migration, rollback at DDL/version boundaries, phase-CAS loss,
identical replay, conflicting replay, capability sequence races, and historical rows.

Run:

```bash
cargo test -p trail --test schema_v21_managed_executions
cargo test -p trail execution::store
```

Expected: all tests pass with schema version 21.

- [ ] **Step 5: Commit**

```bash
git add trail/src/execution/store.rs trail/src/execution/mod.rs \
  trail/src/db/mod.rs trail/src/db/storage/schema.rs \
  trail/src/db/storage/schema/ddl.rs trail/src/db/storage/mod.rs \
  trail/src/db/core/init.rs trail/src/lib.rs \
  trail/tests/schema_v21_managed_executions.rs
git commit -m "feat: persist managed execution authority"
```

### Task 3: Build and validate strict envelopes

**Context:** Later launchers must consume one immutable validated contract. This task
centralizes filesystem grants, private directories, environment clearing, and identity
proofs; it does not execute commands.

**Files:**
- Create: `trail/src/execution/validate.rs`
- Create: `trail/src/execution/environment.rs`
- Modify: `trail/src/execution/mod.rs`
- Modify: `trail/src/db/lane/workspace_view.rs`
- Test: unit tests in the new modules

**Interfaces:**
- Consumes: Task 1 models and existing lane/environment status queries.
- Produces:

```rust
pub struct ExecutionRequest {
    pub lane: String,
    pub task_id: Option<String>,
    pub turn_id: Option<String>,
    pub command: Vec<String>,
    pub isolation_mode: IsolationMode,
    pub timeout_secs: u64,
    pub capability_operations: BTreeSet<CapabilityOperation>,
}

impl Trail {
    pub(crate) fn build_execution_envelope(
        &self,
        request: &ExecutionRequest,
    ) -> Result<ExecutionEnvelope>;
}

pub(crate) fn validate_envelope(
    envelope: ExecutionEnvelope,
    workspace_root: &Path,
    db_dir: &Path,
    sibling_roots: &[PathBuf],
) -> Result<ValidatedEnvelope>;
```

- [ ] **Step 1: Build lane-private runtime paths**

Create `.trail/executions/<execution_id>/{home,tmp,xdg-config,xdg-cache,xdg-data,xdg-state,ipc}`
with mode `0700` on Unix and owner-only ACLs on Windows. Never grant the parent
`.trail/executions` directory.

- [ ] **Step 2: Replace ambient environment construction**

Build a `BTreeMap` after `env_clear()` with sanitized `PATH`, private HOME/temp/XDG,
lane/source/view/generation IDs, typed environment-generation bindings, and only
declared cache/service variables. Remove `TRAIL_WORKSPACE` because it exposes the
original root; expose opaque `TRAIL_WORKSPACE_ID` instead.

- [ ] **Step 3: Implement grant validation and identity pinning**

Canonicalize existing paths, use no-follow metadata for final components, reject
relative/non-normalized paths, nested conflicting access, source/db/sibling aliases,
case-fold and NFC aliases, and grants outside approved system roots. Store device,
inode/file-index, type, and canonical path in `ValidatedGrant`; revalidate immediately
before launch.

- [ ] **Step 4: Add post-implementation validation tests**

Cover normal lane workdirs plus absolute escape, `..`, symlink parent/final component,
hard-link alias, sibling lane, `.trail`, source root, case alias, Unicode alias,
identity replacement, ambient secret removal, and private-directory permissions.

Run:

```bash
cargo test -p trail execution::validate
cargo test -p trail execution::environment
```

Expected: all tests pass; hostile grants return `EXECUTION_ENVELOPE_INVALID` or
`SANDBOX_IDENTITY_CHANGED`.

- [ ] **Step 5: Commit**

```bash
git add trail/src/execution/validate.rs trail/src/execution/environment.rs \
  trail/src/execution/mod.rs trail/src/db/lane/workspace_view.rs
git commit -m "feat: build and validate lane execution envelopes"
```

### Task 4: Generalize native sandbox backends

**Context:** Restricted recipes are Trail's managed build-command surface and already
contain strong macOS, Linux, and Windows enforcement, while agents duplicate weaker
launchers. Move backend generation behind a single interface and migrate those build
commands first, without changing the remaining execution surfaces yet.

**Files:**
- Create: `trail/src/execution/sandbox.rs`
- Create: `trail/src/execution/sandbox_macos.rs`
- Create: `trail/src/execution/sandbox_linux.rs`
- Create: `trail/src/execution/sandbox_windows.rs`
- Modify: `trail/src/execution/mod.rs`
- Modify: `trail/src/cli/mod.rs`
- Modify: `trail/src/cli/environment_sandbox.rs`
- Modify: `trail/src/db/lane/workspace_environment.rs`
- Test: backend unit tests and existing restricted-recipe tests

**Interfaces:**
- Consumes: `ValidatedEnvelope`.
- Produces:

```rust
pub trait ExecutionSandbox {
    fn preflight(&self, envelope: &ValidatedEnvelope) -> Result<SandboxProof>;
    fn command(
        &self,
        envelope: &ValidatedEnvelope,
        spec: &CommandSpec,
        proof: &SandboxProof,
    ) -> Result<PreparedSandboxCommand>;
}

pub(crate) fn native_sandbox(mode: IsolationMode)
    -> Result<Box<dyn ExecutionSandbox + Send + Sync>>;
```

- [ ] **Step 1: Implement macOS deny-by-default profile generation**

Generate `(deny default)`, import `system.sb`, deny network by default, allow only the
validated executable/system reads, exact read-only grants, exact writable workdir and
private roots, and the broker descriptor. Do not generate any `.trail` or workspace
parent grant.

- [ ] **Step 2: Generalize Linux Landlock/seccomp parsing**

Replace recipe-specific `--root/--read/--output/--cache` parsing with a canonical
serialized envelope file descriptor. Apply Landlock hard requirements for filesystem
rights and seccomp for network, namespace, ptrace, BPF, module, and privilege syscalls.

- [ ] **Step 3: Generalize Windows AppContainer**

Grant RX/R/M rights only to validated entries, put the child in a Job Object before it
runs, and reject systems where AppContainer setup or the required Job limits fail.

- [ ] **Step 4: Migrate restricted recipes onto the shared backend**

Translate recipe inputs/outputs/caches into `ExecutionEnvelope` grants, keeping current
managed-build behavior and fail-closed tests. Route recipe command launch and teardown
through the shared backend, but retain recipe publication semantics. Delete duplicate
profile/argument generation only after the shared path passes.

- [ ] **Step 5: Add backend tests after implementation**

Assert generated policies omit source, sibling, and `.trail`; execute native probes for
read/write/network denial; retain existing restricted-command recipe coverage.

Run:

```bash
cargo test -p trail execution::sandbox
cargo test -p trail restricted_command_recipe
```

On native CI also run the existing Linux and Windows recipe scripts.

- [ ] **Step 6: Commit**

```bash
git add trail/src/execution/sandbox.rs trail/src/execution/sandbox_macos.rs \
  trail/src/execution/sandbox_linux.rs trail/src/execution/sandbox_windows.rs \
  trail/src/execution/mod.rs trail/src/cli/mod.rs \
  trail/src/cli/environment_sandbox.rs \
  trail/src/db/lane/workspace_environment.rs
git commit -m "feat: unify native execution sandboxes"
```

### Task 5: Add managed process trees and the durable lifecycle

**Context:** Current gate timeout kills one direct child, and terminal/ACP launchers
wait for one process. This task creates the only permitted child lifecycle.

**Files:**
- Create: `trail/src/execution/process.rs`
- Create: `trail/src/execution/lifecycle.rs`
- Modify: `trail/src/execution/mod.rs`
- Modify: `trail/src/db/util/command_run.rs`
- Modify: `trail/src/cli/mod.rs`
- Test: process and lifecycle unit/integration tests

**Interfaces:**
- Consumes: Tasks 2-4 store, envelope, and sandbox interfaces.
- Produces:

```rust
pub struct ManagedProcessTree { /* private platform authority */ }

impl ManagedProcessTree {
    pub fn wait(&mut self) -> Result<ExecutionOutput>;
    pub fn cancel_and_wait(&mut self) -> Result<ExecutionOutput>;
    pub fn prove_terminated(&self) -> Result<()>;
}

impl ExecutionManager {
    pub fn execute(
        &mut self,
        request: ExecutionRequest,
        io: ExecutionIo,
    ) -> Result<ExecutionReceipt>;
    pub fn recover_incomplete(&mut self) -> Result<Vec<ExecutionReceipt>>;
}
```

- [ ] **Step 1: Implement platform process-tree containment**

Use a new process group and parent-death signal where supported on Unix; attach Windows
children to the prepared Job Object before resuming them. Apply wall-time and declared
resource limits. Close all non-allowlisted descriptors.

- [ ] **Step 2: Implement execute state transitions**

Persist `preparing`, issue runtime state, preflight, revalidate identities, spawn,
publish `started`, wait/cancel, stop descendants, revoke capability, publish outputs,
then transition terminal state. Return no successful receipt unless
`prove_terminated()` succeeds.

- [ ] **Step 3: Implement recovery**

Reopen incomplete rows, protect live matching owners, terminate dead-owner trees using
persisted platform authority, replay terminal receipts idempotently, and mark
unverifiable ownership `isolation_lost`.

- [ ] **Step 4: Route the generic timeout helper through process trees**

Replace `child.kill()` in `run_command_with_timeout_env` with the shared manager or
delete the helper after all callers move. No caller may retain direct-child-only
timeout semantics.

- [ ] **Step 5: Add lifecycle tests after implementation**

Cover normal exit, timeout, cancellation, forked/detached descendant, Trail death,
child death, identity loss, every persisted phase, duplicate recovery, and teardown
before checkpoint.

Run:

```bash
cargo test -p trail execution::process
cargo test -p trail execution::lifecycle
```

Expected: no surviving descendant and exact terminal receipt replay.

- [ ] **Step 6: Commit**

```bash
git add trail/src/execution/process.rs trail/src/execution/lifecycle.rs \
  trail/src/execution/mod.rs trail/src/db/util/command_run.rs trail/src/cli/mod.rs
git commit -m "feat: manage isolated process lifecycles"
```

### Task 6: Implement the lane-scoped capability broker

**Context:** Removing `.trail` access requires a narrow way for managed processes to
append events, upload bounded outputs, and request current-lane operations.

**Files:**
- Create: `trail/src/execution/capability.rs`
- Create: `trail/src/execution/broker.rs`
- Modify: `trail/src/execution/lifecycle.rs`
- Modify: `trail/src/execution/mod.rs`
- Modify: `trail/src/db/lane/control/events.rs`
- Modify: `trail/src/db/lane/workdir/record.rs`
- Test: capability and broker unit/integration tests

**Interfaces:**
- Consumes: Task 2 capability store and Task 5 lifecycle.
- Produces length-prefixed CBOR protocol:

```rust
pub struct BrokerRequest {
    pub execution_id: String,
    pub sequence: u64,
    pub operation: CapabilityOperation,
    pub payload: serde_cbor::Value,
}

pub struct BrokerResponse {
    pub sequence: u64,
    pub result: BrokerResult,
}
```

Allowed operations are `LaneStatus`, `AppendEvent`, `UploadArtifact`,
`RecordCurrentLane`, `ReportProgress`, `ReportCancellation`, and
`RuntimeBindings`.

- [ ] **Step 1: Issue capabilities without persisted bearer secrets**

Generate 256 random bits, persist only SHA-256, pass the secret through an inherited
socket/pipe descriptor, bind it to execution/lane/process identity, and revoke it
before terminal publication.

- [ ] **Step 2: Implement bounded protocol and authorization**

Cap frames at 1 MiB, reject unknown fields/operations, require exact monotonic sequence,
verify expiry/revocation and current lane, and dispatch only typed current-lane methods.
General daemon HTTP/MCP tokens must not authorize broker requests.

- [ ] **Step 3: Record bounded security events**

Record code, execution, operation class, and denial reason; do not persist the token or
raw hostile payload.

- [ ] **Step 4: Add broker tests after implementation**

Cover valid operations, sibling lane/ref/task substitution, replay, skipped sequence,
expired/revoked token, copied token from another process, oversized/malformed frames,
broker restart, and concurrent authorization.

Run:

```bash
cargo test -p trail execution::capability
cargo test -p trail execution::broker
```

- [ ] **Step 5: Commit**

```bash
git add trail/src/execution/capability.rs trail/src/execution/broker.rs \
  trail/src/execution/lifecycle.rs trail/src/execution/mod.rs \
  trail/src/db/lane/control/events.rs \
  trail/src/db/lane/workdir/record.rs
git commit -m "feat: broker lane-scoped execution capabilities"
```

### Task 7: Migrate lane exec, tests, and evals; remove visible lock contention

**Context:** These are bounded, non-interactive surfaces and are the safest first
consumers. Current gate children run outside a strict envelope and concurrent starts
can fail with `WORKSPACE_LOCKED`.

**Files:**
- Modify: `trail/src/db/lane/workspace_view.rs`
- Modify: `trail/src/db/lane/gates/runner.rs`
- Modify: `trail/src/db/lane/gates/wrappers.rs`
- Modify: `trail/src/db/util/gates.rs`
- Modify: `trail/src/model/reports/lane.rs`
- Modify: `trail/src/cli/command/lane_args.rs`
- Modify: `trail/src/cli/command/handler/lane/work.rs`
- Modify: `trail/src/server/request_types/lane.rs`
- Modify: `trail/src/server/route/lane/lanes.rs`
- Modify: `trail/src/mcp/tool_call/lane.rs`
- Test: `trail/tests/e2e.rs`
- Create: `trail/tests/execution_concurrency.rs`

**Interfaces:**
- Consumes: `ExecutionManager::execute`.
- Produces: isolation-aware `WorkspaceExecReport`, `LaneTestReport`, and durable
  `lane_gate_receipts`.

- [ ] **Step 1: Add isolation selection to all lane execution APIs**

Extend `LaneExecArgs`, `LaneTestArgs`, server requests, and MCP arguments with
`Option<IsolationMode>`; resolve `None` from `config.execution.isolation`.

- [ ] **Step 2: Route lane exec through `ExecutionManager`**

Build an envelope for the exact view/source generation, execute without a workspace
lock, persist the receipt, and include `ExecutionIsolationSummary` in the report.

- [ ] **Step 3: Split gate bookkeeping from child execution**

Create the turn, started event, execution row, and gate receipt in one short
transaction; release authority; run once; publish immutable outputs and the terminal
receipt with bounded jittered retry. Use `receipt_id` and `execution_id` to make finish
idempotent without rerunning the child.

- [ ] **Step 4: Add post-implementation concurrency tests**

Run four and 64 distinct lanes with a barrier so child commands overlap. Assert zero
`WORKSPACE_LOCKED`, one started/finished receipt per lane, distinct source roots, and no
reruns when final publication is forced to contend.

Run:

```bash
cargo test -p trail --test execution_concurrency
cargo test -p trail --test e2e lane_test
cargo test -p trail --test e2e lane_exec
```

- [ ] **Step 5: Commit**

```bash
git add trail/src/db/lane/workspace_view.rs trail/src/db/lane/gates/runner.rs \
  trail/src/db/lane/gates/wrappers.rs trail/src/db/util/gates.rs \
  trail/src/model/reports/lane.rs \
  trail/src/cli/command/lane_args.rs trail/src/cli/command/handler/lane/work.rs \
  trail/src/server/request_types/lane.rs trail/src/server/route/lane/lanes.rs \
  trail/src/mcp/tool_call/lane.rs trail/tests/e2e.rs \
  trail/tests/execution_concurrency.rs
git commit -m "feat: isolate concurrent lane commands"
```

### Task 8: Migrate terminal agents and managed hooks

**Context:** This closes the demonstrated sibling-lane overwrite. The current terminal
launcher grants the whole `.trail` subtree and inherits ambient environment.

**Files:**
- Modify: `trail/src/cli/command/agent_args.rs`
- Modify: `trail/src/cli/command/handler/agent.rs`
- Modify: `trail/src/agent_hooks/install.rs`
- Modify: `trail/src/db/agent.rs`
- Modify: `trail/src/model/lane/activity.rs`
- Modify: `trail/src/cli/command/render/agent.rs`
- Test: `trail/tests/e2e.rs`
- Create: `trail/tests/terminal_agent_isolation.rs`

**Interfaces:**
- Consumes: Tasks 3-6 manager and broker.
- Produces: `trail agent start|continue --isolation strict|permissive` and strict
  `AgentRunReport` provenance.

- [ ] **Step 1: Thread isolation mode through start and continue**

Add the Clap option, resolve the strict workspace default, include the mode in task
events, and preserve it across follow-up tasks unless explicitly overridden.

- [ ] **Step 2: Replace direct terminal process launch**

Delete `confined_terminal_agent_command`. Build an `ExecutionRequest`, pass inherited
stdio through `ExecutionIo`, and let `ExecutionManager` perform env clearing, sandbox
launch, process-tree wait, revocation, and terminal receipt publication.

- [ ] **Step 3: Move hooks behind scoped broker access**

Hook assets may write only to lane-private capture paths or the broker descriptor.
Project hook configuration is a declared read-only grant; user-level hook configuration
is not visible unless explicitly declared.

- [ ] **Step 4: Add the exact sibling-write regression after implementation**

Create two real lanes, launch a custom terminal command in lane B that attempts read,
write, delete, rename, symlink, hard-link, and enumeration against lane A, `.trail`,
and the original workspace. Assert each operation fails, lane A stays clean, Trail
state is valid, and the attacker receipt records strict enforcement.

Also test ambient HOME/secret absence, permissive explicit behavior, and no automatic
fallback when `sandbox-exec`/Landlock/AppContainer is unavailable.

Run:

```bash
cargo test -p trail --test terminal_agent_isolation
cargo test -p trail --test e2e terminal_agent
```

- [ ] **Step 5: Commit**

```bash
git add trail/src/cli/command/agent_args.rs trail/src/cli/command/handler/agent.rs \
  trail/src/agent_hooks/install.rs trail/src/db/agent.rs \
  trail/src/model/lane/activity.rs trail/src/cli/command/render/agent.rs \
  trail/tests/e2e.rs trail/tests/terminal_agent_isolation.rs
git commit -m "feat: enforce strict terminal agent isolation"
```

### Task 9: Migrate ACP agents and sessions

**Context:** ACP currently launches its upstream process before lane sessions are fully
mapped and uses a permissive macOS profile or direct execution elsewhere.

**Files:**
- Modify: `trail/src/acp.rs`
- Modify: `trail/src/acp/transport.rs`
- Modify: `trail/src/acp/capture.rs`
- Modify: `trail/src/cli/command/agent_args.rs`
- Modify: `trail/src/cli/command/handler/agent.rs`
- Test: `trail/tests/acp_workspace_mapping.rs`
- Test: `trail/tests/acp_session_semantics.rs`
- Create: `trail/tests/acp_isolation.rs`

**Interfaces:**
- Consumes: `ExecutionManager`, broker descriptor, and existing ACP path mappings.
- Produces: one managed execution per ACP upstream connection, with lane-bound session
  mappings and terminal receipt.

- [ ] **Step 1: Resolve lane and envelope before starting upstream ACP**

Extend `AcpRelayOptions` with `isolation_mode`; materialize/resolve the lane first;
build exact path grants for the session workdir and declared additional roots.

- [ ] **Step 2: Replace `confined_acp_command`**

Launch upstream transport through `ExecutionManager`, keep raw ACP stdin/stdout
forwarding byte-exact, and route Trail mutations through the broker. Preserve existing
correlation, backpressure, spill, and shutdown behavior.

- [ ] **Step 3: Validate additional roots**

Default additional roots to read-only, reject roots that alias Trail/source/sibling
state, and require an explicit typed capability before any writable additional root.

- [ ] **Step 4: Add ACP isolation tests after implementation**

Run three synchronized relays in distinct lanes; attempt cross-lane resources, terminal
commands, additional-root aliases, capability replay, and daemon loss. Retain all ACP
v1 byte/correlation conformance tests.

Run:

```bash
cargo test -p trail --test acp_isolation
cargo test -p trail --test acp_conformance
cargo test -p trail --test acp_faults
cargo test -p trail --test acp_workspace_mapping
```

- [ ] **Step 5: Commit**

```bash
git add trail/src/acp.rs trail/src/acp/transport.rs trail/src/acp/capture.rs \
  trail/src/cli/command/agent_args.rs trail/src/cli/command/handler/agent.rs \
  trail/tests/acp_workspace_mapping.rs trail/tests/acp_session_semantics.rs \
  trail/tests/acp_isolation.rs
git commit -m "feat: isolate ACP agent executions"
```

### Task 10: Enforce isolation provenance in readiness, API, and UX

**Context:** Strict execution is incomplete if permissive or historical evidence can be
mistaken for strict readiness.

**Files:**
- Modify: `trail/src/db/lane/readiness.rs`
- Modify: `trail/src/db/agent.rs`
- Modify: `trail/src/db/storage/lane_gates.rs`
- Modify: `trail/src/cli/command/render/lane/identity/reports.rs`
- Modify: `trail/src/cli/command/render/agent.rs`
- Modify: `trail/src/server/openapi/schemas/lane.rs`
- Modify: `trail/src/server/openapi/paths/lanes.rs`
- Modify: `trail/src/mcp/tools/lane.rs`
- Modify: `docs/reference/cli/lanes.md`
- Modify: `docs/integrations/acp.md`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: persisted `ExecutionIsolationSummary`.
- Produces readiness blockers `isolation_unverified`, `isolation_permissive`,
  `isolation_lost`, `process_tree_not_terminated`, and `capability_not_revoked`.

- [ ] **Step 1: Load execution provenance with gates and tasks**

Join latest gate/task execution rows and project historical missing rows as
`unverified`; never infer strictness from platform or command name.

- [ ] **Step 2: Add readiness blockers and renderers**

Block required strict gates when proof/source/environment is stale, teardown or
revocation is incomplete, or isolation is permissive/unverified/lost. Render backend,
mode, execution ID, proof status, network class, and containment class in human/JSON
reports and handoffs.

- [ ] **Step 3: Update OpenAPI, MCP schemas, and documentation**

Expose the same enum values and fields across CLI/HTTP/MCP; document the explicit
permissive escape hatch and its readiness consequence.

- [ ] **Step 4: Add post-implementation projection tests**

Cover strict ready, permissive blocked, historical unverified, lost isolation,
revocation incomplete, terminated incomplete, stale source/environment, and
CLI/HTTP/MCP equality.

Run:

```bash
cargo test -p trail --test e2e readiness
cargo test -p trail server::openapi
cargo test -p trail mcp::tools
```

- [ ] **Step 5: Commit**

```bash
git add trail/src/db/lane/readiness.rs trail/src/db/agent.rs \
  trail/src/db/storage/lane_gates.rs \
  trail/src/cli/command/render/lane/identity/reports.rs \
  trail/src/cli/command/render/agent.rs \
  trail/src/server/openapi/schemas/lane.rs \
  trail/src/server/openapi/paths/lanes.rs trail/src/mcp/tools/lane.rs \
  trail/tests/e2e.rs docs/reference/cli/lanes.md docs/integrations/acp.md
git commit -m "feat: require isolation evidence for readiness"
```

### Task 11: Add shared adversarial conformance and native CI gates

**Context:** The default cannot switch until every supported platform proves the same
boundary with real processes.

**Files:**
- Create: `trail/tests/execution_isolation_conformance.rs`
- Create: `scripts/verify-macos-execution-isolation.sh`
- Create: `scripts/verify-linux-execution-isolation.sh`
- Create: `scripts/verify-windows-execution-isolation.ps1`
- Create: `.github/workflows/execution-isolation.yml`
- Modify: `.github/workflows/release-readiness.yml`
- Modify: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: every migrated execution surface.
- Produces: one shared conformance manifest and exact-SHA native release gates.

- [ ] **Step 1: Implement one reusable hostile probe matrix**

Parameterize the same fixture over terminal, ACP, lane exec, test, and eval. Probe
sibling/source/internal reads and writes, links, traversal, aliases, environment
secrets, broker replay, unrelated loopback service access, and detached descendants.

- [ ] **Step 2: Add native wrappers**

Each wrapper builds the exact candidate binary, records its SHA-256 and source commit,
runs every probe without skips, records backend/proof summaries, checks `trail doctor`
and `trail fsck`, and emits a checksummed evidence manifest.

- [ ] **Step 3: Add 64-lane concurrency**

Start 64 distinct strict executions behind a barrier; require zero lock errors, exact
one start/finish receipt, clean sibling/source state, and no surviving processes,
sockets, mounts, capabilities, or private runtime directories.

- [ ] **Step 4: Gate release workflows**

Require macOS, Linux, and Windows jobs at the exact release SHA. Do not permit an
allowed-failure platform or cached evidence from another commit.

- [ ] **Step 5: Verify locally and in CI**

Run the host-native wrapper and:

```bash
cargo test -p trail --test execution_isolation_conformance
cargo test -p trail --test execution_concurrency
```

Expected: every selected surface/backend passes; no skipped strict probe.

- [ ] **Step 6: Commit**

```bash
git add trail/tests/execution_isolation_conformance.rs \
  scripts/verify-macos-execution-isolation.sh \
  scripts/verify-linux-execution-isolation.sh \
  scripts/verify-windows-execution-isolation.ps1 \
  .github/workflows/execution-isolation.yml .github/workflows/release-readiness.yml \
  .github/workflows/release.yml
git commit -m "test: gate strict execution isolation"
```

### Task 12: Cut over the default and complete repository verification

**Context:** Earlier tasks add strict support and evidence. This task removes legacy
paths, activates the default, refreshes checked inventories, and proves the repository
is releasable.

**Files:**
- Modify: `trail/src/cli/command/handler/agent.rs`
- Modify: `trail/src/acp.rs`
- Modify: `trail/src/db/lane/workspace_view.rs`
- Modify: `trail/src/db/lane/gates/runner.rs`
- Modify: `trail/src/db/change_ledger/activation.rs`
- Modify: `trail/tests/changed_path_ledger_activation.rs`
- Modify: `README.md`
- Modify: `docs/guides/hardening-agent-workflows.md`
- Modify: `docs/audits/trail-lane-environment-isolation-audit-2026-07-12.md`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: all prior tasks and exact native CI evidence.
- Produces: strict-default release behavior with no legacy managed direct launch.

- [ ] **Step 1: Remove every legacy managed launcher**

Inventory every production `Command::new`, `Command::spawn`, `Command::status`,
`Command::output`, and `CommandExt::exec` call and classify it as either a managed
repository-content execution or a trusted Trail maintenance/helper process. Prove that
terminal, ACP, gate, lane-exec, managed-hook, and restricted-recipe/build paths launch
only through `trail/src/execution/`; keep a reviewed allowlist for Git plumbing,
mount/unmount helpers, daemon startup, sandbox helpers, and UI helpers. Remove
permissive macOS profiles and non-macOS direct fallbacks.

- [ ] **Step 2: Activate strict default**

After the exact candidate SHA passes Task 11 on macOS, Linux, and Windows, change
`ExecutionConfig::default().isolation` from `Permissive` to `Strict`; ensure every
CLI/API/MCP surface resolves omitted mode to strict and refuses unavailable
enforcement. Replace Task 1's compatibility-default test with a strict-default
regression. Verify explicit permissive mode remains visibly marked and
readiness-blocking.

- [ ] **Step 3: Update truth-in-advertising documentation**

Document the new strict boundary and residual host-admin/non-managed-command limits.
Mark the 2026-07-12 audit findings resolved only where the new conformance evidence
directly proves resolution; preserve unresolved network/ecosystem items.

- [ ] **Step 4: Refresh checked activation inventories**

Run the repository inventory/audit scripts, review every changed producer/mutation
entry, update production and test digest constants together, and rerun the exact
activation gate. Do not copy a digest without reviewing its generated inventory.

- [ ] **Step 5: Run complete verification**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p trail --lib
RUST_TEST_THREADS=1 cargo test -p trail --lib
cargo test -p trail
python3 -m unittest discover -s scripts -p 'test_*.py'
trail doctor
trail fsck
```

Expected: zero failures; benchmark-only ignored tests are documented; source worktree
contains only the planned changes.

- [ ] **Step 6: Commit**

```bash
git add trail/src/cli/command/handler/agent.rs trail/src/acp.rs \
  trail/src/db/lane/workspace_view.rs trail/src/db/lane/gates/runner.rs \
  trail/src/db/change_ledger/activation.rs \
  trail/tests/changed_path_ledger_activation.rs README.md \
  docs/guides/hardening-agent-workflows.md \
  docs/audits/trail-lane-environment-isolation-audit-2026-07-12.md \
  CHANGELOG.md
git commit -m "feat: make strict lane isolation the default"
```

## Final Review Checklist

- [ ] Every managed execution surface uses `ExecutionManager`.
- [ ] Restricted recipes and other managed build commands use the shared envelope;
      only reviewed Trail maintenance helpers remain outside it.
- [ ] No managed child has a blanket `.trail` or source-workspace grant.
- [ ] Strict launches start from `env_clear()`.
- [ ] Strict never falls back to permissive.
- [ ] Capabilities are lane/process/operation/sequence/expiry bound and persisted only as hashes.
- [ ] Full process trees terminate before checkpoint, unmount, gate finish, or capability revocation completion.
- [ ] Independent lane gates run concurrently without visible lock errors or child reruns.
- [ ] Historical evidence remains `unverified`; permissive evidence blocks strict readiness.
- [ ] macOS, Linux, and Windows pass identical real-process conformance at the exact release SHA.
- [ ] The complete Rust and Python regression suites are green.

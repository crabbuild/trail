# Environment Adapter and Specification Contract

Status: partially implemented; the built-in host contract, Node/Cargo/Go/CMake/Python adapters,
restricted command recipes, and experimental isolated subprocess plugin SDK protocols v1 and v2 are
available. Native macOS, Linux, and Windows enforcement implementations exist; complete
native release evidence, signed catalogs/WASI packaging, complete typed runtime/resource
graphs, and certification remain planned.

## Current implementation

Trail now has an internal `WorkspaceEnvironmentAdapter` contract whose adapters emit a
normalized, argv-based plan. The host owns pinned source projection, staging, command
execution, immutable publication, declared-path replacement, component state, and view
generation activation. The first built-ins are:

- `trail/node@1`: npm, pnpm, Yarn, and Bun frozen dependency trees;
- `trail/cargo-target-seed@1`: conservative `cargo build --locked --offline` target
  seeds keyed by the complete source root and Rust toolchain identity;
- `trail/go-vendor@1`: single-module Go vendor trees keyed by the complete source root,
  module files, Go executable identity, and platform. Go workspaces fail closed until a
  graph-aware multi-module adapter is available;
- `trail/cmake-build@1`: provisions a `writable_private` build tree with no synthetic
  shared layer. Configure is deliberately deferred until execution inside the mounted
  lane so absolute paths in `CMakeCache.txt` name the stable lane workdir rather than a
  disposable staging directory;
- `trail/python-venv@1`: recognizes `pyproject.toml` and the common uv, Poetry, PDM,
  Pipenv, and requirements lock/manifest files, provisions a layer-free lane-private
  `.venv`, and keys it by every present dependency file plus the resolved Python
  executable. Trail automatically creates the virtual environment through an ephemeral
  candidate view at the lane's final mountpoint, so scripts and prefix metadata embed
  the correct absolute path without exposing partial state;
- `trail/oci-image@1`: reads `trail.oci.toml`, accepts only lowercase SHA-256
  digest-pinned OCI references with an explicit platform, and records provider-owned
  image identities without commands, caches, mounts, or manufactured directories;
- `trail/command@1`: repository-declared argv commands with exact/globbed byte inputs,
  one to 32 immutable-seeded or writable-private generated outputs, denied
  shell/network/scripts/secrets, macOS sandbox enforcement, a Linux
  Landlock-plus-seccomp helper, and a
  capability-free Windows AppContainer constrained by a one-process Job Object. Kernels
  without the required Landlock ABI and unsupported operating systems fail closed.
- external `trail.environment-adapter/v1` and `/v2` plugins: explicitly installed local packages
  stored by distribution digest and invoked through bounded length-prefixed CBOR. Plugins
  receive only host-selected bytes from a pinned root and return discovery or
  denied-by-default command plans through the same native sandbox. V1 proposes one
  staging command. V2 adds typed staging and mounted-initialization actions plus
  metadata-only external-artifact declarations while the host retains mount authority
  and atomic generation activation.

`trail env adapters` returns the compiled and installed adapter catalog, including versioned identity,
selectors, component kind, discovery markers, implementation provenance, stability, and
description without probing the repository or host tools. Discovery obtains candidate
manifest names from that metadata rather than a central ecosystem filename list.
`trail env sync <lane>` auto-detects an unambiguous adapter, while `--adapter` resolves
one component explicitly. `trail env discover` enumerates nested component proposals
without launching ecosystem tools or repository code; installed plugin planners run only
inside the capability-free sandbox. `trail env sync-all` builds every
proposal before atomically activating all mounts as one generation. `trail env status`
reports logical component identity separately from adapter identity. CLI, HTTP/OpenAPI,
MCP, and Rust APIs share this state. Existing `trail deps` behavior remains a Node
compatibility surface.

Command recipes and v1/v2 plugins may declare stable logical component dependencies.
`sync-all` validates missing nodes, duplicate/self edges, complete cycles, and mount
collisions before running a command; it then builds in deterministic topological order.
Legacy `depends_on` and protocol-v1 dependencies mean `build_requires`. Protocol v2 and
repository recipes can explicitly select `build_requires`, `runtime_requires`,
`binds_after`, or `invalidates_with`. Trail adds upstream keys for `build_requires` and
`invalidates_with` to the downstream canonical key; runtime/order-only edges select an
exact upstream instance in generation provenance without manufacturing a new artifact
key. Replacing a runtime/order-only provider atomically advances that generation edge
while leaving the consumer ready. Replanning an identity-bearing upstream makes only
identity descendants stale with an exact edge-level explanation. A single
component sync requires every dependency to be ready in the lane and points users to
`sync-all` otherwise; replacing an upstream component alone immediately marks mounted
identity descendants stale without rewriting the dependency keys they were built against.
`env graph` renders that finalized desired graph through CLI, Rust, HTTP/OpenAPI, and
MCP before synchronization, including topological indices, component keys, output
ownership, and exact upstream-key edges without publishing artifacts. CLI, HTTP, and MCP
page by target node with `offset`, `limit`, total node/edge counts, and `next_offset`;
the Rust API also exposes the complete in-process graph.
A repository-wide `sync-all` also treats absence as desired state: components removed
from discovery are unbound with crash-recoverable upper resets in the same activation,
and deleting the final component creates an inspectable empty generation. A scoped
`sync-all --path` never retires components outside that scope.

Large recipe graphs are batch-planned: Trail parses the complete recipe document twice
(discovery plus planning), not once per component, reuses executable identity checks by
path, and performs target, mount, and source-shadow checks with ordered prefix indexes
instead of quadratic scans. A 1,000-node recipe-chain regression exercises the public
graph path; the lower-level non-recursive resolver covers 10,000 nodes.

Bindings now contribute runtime path classification, so an adapter-declared dependency
path such as `.venv` receives a private generated upper and stays out of source
checkpoints even though it is not a hard-coded directory name.

Host execution clears the ambient process environment, supplies an isolated HOME and
temporary directory, and injects only adapter-declared cache/toolchain variables plus
PATH. Built-in keys include adapter implementation provenance and resolved tool
executable digests. Execution preserves dispatcher shim paths such as rustup's `cargo`
shim and verifies the canonical executable digest again immediately before launch.
Cargo performs a locked fetch into managed `CARGO_HOME` followed by
an offline build; MCP and HTTP classify synchronization as open-world execution because
repository-controlled Cargo build scripts and proc macros still execute. Read-only
status reads persisted state and never invokes package managers or compilers.

The initial Node adapter deliberately rejects workspace-root installs, local file/link
dependencies, pnpm workspace roots, and Yarn Berry/PnP rather than publishing an empty,
escaping, or incorrectly keyed layer. Those forms require explicit workspace-graph and
link contracts before they can graduate.

The first public Rust protocol crate is `trail-environment-adapter-sdk`. Local executable
packages remain `experimental`: Trail verifies and content-addresses them, records
append-only activation/tombstone history, revalidates their executable before every use,
and executes them without repository, database, mount, process, or network authority.
Detached Ed25519 publisher trust and immediate revocation are implemented. Remote
organization catalogs, WASI components, and independent certification promotion remain
planned.

Each successful activation now creates an immutable environment generation containing
the pinned source root, sorted component keys, optional exact layer IDs, storage-policy
identities, mount targets, and
predecessor. Failed batch activation leaves the predecessor pointer unchanged and rolls
back every prepared private upper. Retired generations pin their referenced layers so
cache collection cannot invalidate execution provenance. Lane command environments
receive `TRAIL_ENVIRONMENT_GENERATION`.

Schema v13 adds active and historical external-artifact provenance. Metadata-only
components participate in the same atomic generation transaction but have no layer ID,
mount path, output binding, or cache namespace. OCI declarations retain name, provider,
digest-pinned reference, digest, platform, and explicit `external` cleanup ownership;
Trail layer/cache GC never claims or deletes provider-owned content.

Every synchronization also owns a durable process-token attempt record. On workspace
open, Trail recovers attempts whose owner died: a component with an attached predecessor
becomes `stale`, a first build with no predecessor becomes `failed`, and a crash after
the activation transaction is recognized as completed. A second synchronization cannot
race an active attempt for the same workspace view.

Schema v6 adds named component-output bindings and generation-output provenance. One
component command can publish up to 32 directories into a single immutable physical
layer; each output is mounted from a stable layer subpath with its own lane-private
writable upper. Publication and all bindings activate atomically. Replacing a component
also transactionally retires removed output uppers, and the durable reset intent can
distinguish a committed unbind from an interrupted activation so stale private files do
not reappear after recovery. The first output remains projected through legacy singular
report fields while CLI, JSON, HTTP/OpenAPI, MCP, and generation reports expose the full
ordered output list.

Schema v7 makes output policy and storage identity explicit and permits a generation
output to have no layer. A `writable_private` binding records a content-derived private
identity plus canonical component-key provenance while its bytes remain solely in the
lane's generated upper. Compatible re-sync preserves mutations; a changed key or deleted
private root is rebuilt and replaced through the durable reset transaction. Gate reports
include only actual shared layer IDs, and every public environment surface reports the
nullable layer ID rather than inventing an empty artifact.

This document specifies how ecosystem knowledge plugs into the
[Universal Lane Environments](universal-lane-environments.md) architecture. It defines
the boundary between an adapter and the Trail host, a declarative repository format,
plugin capabilities, conformance requirements, and reference mappings for common
ecosystems.

The contract is intentionally narrower than a general extension API. Adapters describe
and inspect; the Trail host authorizes, executes, validates, publishes, attaches, and
persists.

## How to add an adapter

Trail should offer an extension ladder instead of forcing every ecosystem integration
into core. Choose the lowest tier that expresses the required behavior:

| Tier | Use it when | Distribution | Available now |
| --- | --- | --- | --- |
| Command component | One repository needs deterministic generated trees from declared inputs and one command. | Committed `trail.environment.toml` | Yes |
| Versioned profile | Several components or repositories share the same command recipe and policy. | Local included TOML today; signed organization catalogs later | Yes for local includes |
| Isolated plugin | Discovery or semantic parsing is required beyond a declarative recipe. V1 proposes one denied-by-default staging command; v2 adds typed mounted initialization for path-sensitive writable-private outputs, metadata-only pinned external artifacts, and host-managed lane-private OCI services. | Content-addressed signed or unsigned local subprocess package today; remote signed catalogs/WASI later | Yes, experimental local packages |
| Built-in adapter | Trail maintains the ecosystem, migrations, cross-platform behavior, and release fixtures. | Compiled into Trail | Yes, for Trail contributors |

Promotion is evidence-based. A useful repository recipe can become a profile; a profile
that needs semantic code can become an isolated plugin; a widely used, fully certified
plugin can be proposed as a built-in. Component and adapter identities change explicitly
when semantics change, so promotion never silently reinterprets an existing generation.

### Add a repository adapter without Rust

Use `trail/command@1` when the adapter can be represented as pinned byte inputs, an argv
command, and one or more contained generated outputs. The repository may split reusable policy
into local includes:

```toml
# trail.environment.toml
schema = "trail.environment/v1"
include = ["trail/environments/protobuf.toml"]

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "api.protobuf"
root = "services/api"
extends = ["profile.protobuf"]
```

```toml
# trail/environments/protobuf.toml
schema = "trail.environment/v1"

[profile.protobuf]
version = "1.0.0"
adapter = "trail/command@1"
kind = "generated"
inputs = [
  { path = "{root}/proto/**/*.proto", role = "identity", format = "bytes" },
]
outputs = [
  { source = "generated", target = "{root}/generated", policy = "immutable_seed_private", portability = "host" },
]

[profile.protobuf.build]
command = ["protoc", "--cpp_out=generated", "proto/service.proto"]
cwd = "{root}"
network = "deny"
scripts = "deny"
```

Includes are repository-relative committed files. Remote URLs, globs, traversal,
ambiguous duplicate profiles, include cycles, and profile inheritance cycles fail
closed. Includes resolve relative to the including file. Profiles are merged
left-to-right, followed by component overrides: scalar adapter/kind/build values replace
earlier values, inputs accumulate, and a non-empty child output list replaces the
inherited output list. `{root}` is the only template. Canonical expansion, every source
file digest, and every transitive profile version enter the layer key.

The authoring loop is:

```text
trail env discover <lane>
trail env graph <lane>
trail env plan <lane> --component api.protobuf
trail env sync <lane> --component api.protobuf
trail env status <lane>
```

Planning is read-only and shows the exact inputs, executable identity, output, key, and
capability grants. Synchronization executes only through the host sandbox and publishes
or privately installs only after output validation. This tier deliberately cannot request a shell, network,
secrets, arbitrary host reads, caches, services, or heterogeneous output policies yet.

### Add a built-in ecosystem adapter

A built-in is appropriate only when semantic planning cannot be expressed safely by a
command profile. Examples include parsing workspace graphs, choosing package-manager
strategies, separating content caches from lane-private trees, or validating ABI and
toolchain compatibility.

The implementation checklist is:

1. Add `workspace_<ecosystem>.rs` beside the existing Node, Cargo, and Go adapters.
2. Define immutable `WorkspaceEnvironmentAdapterMetadata`: canonical identity,
   selectors, contract major, implementation version, distribution digest, kind,
   storage adapter name, discovery markers, stability, and description.
3. Implement `WorkspaceEnvironmentAdapter`. `detect` must only inspect the pinned Trail
   root. `plan` must return a deterministic `WorkspaceEnvironmentPlan`; neither method
   may execute tools, write files, publish layers, attach mounts, or update the database.
4. Register the static adapter in `builtin_environment_adapters()`. The same metadata
   then drives `env adapters`, discovery, CLI, HTTP/OpenAPI, MCP, and Rust API reports.
5. Put every output-changing fact in `WorkspaceLayerKeyV1`: normalized manifest and
   lockfile content, exact executable identity, adapter implementation/distribution,
   platform/architecture/ABI, policy, and relevant configuration. Explain the same facts
   in `stale_reason`.
6. Describe staged source inputs, host-executed argv commands, cache directories, owned
   outputs and mount targets, and the narrowest portability and sandbox policy. For a
   provider-owned immutable resource, use kind `external`, include the sorted
   `external_artifact_contract` in the canonical key, and declare no action, cache, or
   filesystem output. A private service additionally declares a sorted
   `runtime_resource_contract` referencing one of those artifacts; Trail retains all
   provider, port, health, ownership, and lifecycle authority.
   Never teach the adapter about SQLite, NFS, FUSE, overlayfs, or layer publication.
7. Add discovery, deterministic-key, irrelevant-edit, changed-input, two-lane reuse,
   private-mutation, failure/no-partial-publication, crash-recovery, security, backend,
   and real-scale fixtures. Start at `experimental`; raise certification only when its
   platform matrix passes the shared conformance suite.

Adding a third-party executable to `PATH` does not register it as an adapter. A package
must be installed explicitly:

```text
trail env plugin install path/to/package
trail env adapters
trail env discover <lane>
trail env plan <lane> --adapter namespace/name@1
trail env sync <lane> --adapter namespace/name@1
trail env plugin remove namespace/name@1
```

The package contains `trail-adapter.toml`, one declared executable, and optionally a
detached `trail-adapter.sig`. `env plugin inspect` reports the canonical payload and
distribution digests before mutation. Installation verifies the executable SHA-256,
computes a digest over canonical package metadata plus executable bytes, verifies an
optional Ed25519 signature against the workspace's append-only publisher trust store,
publishes the exact authenticated bytes, and appends an activation record.
Removal appends a tombstone and retains immutable bytes for provenance and repair. A
corrupt active executable fails the catalog closed; reinstalling quarantines the corrupt
copy, while removal remains available for recovery. Trust revocation also fails signed
packages closed immediately. Unsigned packages are visibly `local-experimental`; a
signature authenticates origin but does not grant stable certification.

The adapter catalog reports each package's planner protocols, supported operating systems,
and architectures. Unsupported plugins remain inspectable but are not auto-discovered or
executed on the current host. `lane readiness` replans every installed component from
the current pinned root through the same built-in, recipe, or plugin path used by sync;
an input, semantic plan, tool, package, executable, platform, capability, command, or
output-contract change marks the attached generation stale. The read-only `env status`
shows the last persisted result and does not execute plugin code by itself.
Newly published artifacts retain the canonical key declaration, not only its digest.
`trail env explain <lane> --component <id>` compares the attached and current keys and
reports added, removed, or modified input/tool edges plus platform, architecture,
adapter, portability, and strategy changes. Values are never rendered. Large monorepo
results are paginated consistently across CLI, HTTP, and MCP; legacy layers explicitly
report incomplete provenance instead of inventing a cause.

### Plugin implementation and next target

The [Rust SDK](../../trail-environment-adapter-sdk/README.md) makes an external adapter
return discovery and component plans over the versioned
`trail.environment-adapter/v1` or `/v2` boundary. The host selects the highest protocol
declared by both package and host; packages without protocol metadata retain the exact
v1 canonical payload and behavior. The host supplies bounded pinned-file
bytes, enforces deadlines and request/response/diagnostic limits, resolves and rechecks
tools, executes approved actions itself, validates outputs, and owns all state changes.
Every file exposed to the planner must appear in its identity-input response; Trail
checks exact set equality and keys all of them, preventing inspected-but-unkeyed inputs.
Protocol, implementation, package, executable, semantic input, action phase, tool, output,
platform, and capability identities enter the layer key.

The conformance fixture verifies discovery, planning, publication, two-lane reuse,
private copy-up, v2 initialization at two distinct final lane paths, declared-input
reads, undeclared source read/write denial, predecessor preservation, input-change
staleness, active-command parent death and abandoned-candidate recovery, timeout,
memory overrun, crash, oversized
output, malformed framing, attempted child execution, executable tampering, removal,
and repair. Native launchers add process/file/memory limits to the denied-by-default
filesystem, network, shell, and child-process sandbox. Ed25519 publisher signatures and
the host trust store authenticate local packages today. Signed catalogs, independent
certification attestations, and WASI transport remain distribution work.

## Design principles

1. One contract covers package managers, compilers, build systems, containers, services,
   toolchains, and user-defined commands.
2. Adapters return structured plans rather than mutating the lane.
3. All filesystem outputs have an explicit policy and containment root.
4. Commands are argument vectors by default, not shell strings.
5. Fingerprints are deterministic and independently explainable.
6. Secret values are opaque to planning and fingerprinting.
7. Host-owned staging and publication prevent partial shared artifacts.
8. Adapter discovery is side-effect free.
9. Capabilities are denied unless declared and approved.
10. Built-ins, recipes, and isolated plugins produce the same normalized plan.

## Contract layers

The contract has three versioned layers:

| Layer | Responsibility |
| --- | --- |
| Specification schema | Repository-authored desired components, policies, and overrides. |
| Adapter protocol | Discovery, fingerprint inputs, actions, outputs, validation, and binding proposals. |
| Host execution protocol | Capability grants, staged command execution, publication, generation attachment, and reports. |

Every record carries a schema version. The host rejects unknown required fields and may
ignore only extension fields explicitly marked optional.

## Adapter identity

An adapter identity is a tuple:

```text
(namespace, name, contract_major, implementation_version, distribution_digest)
```

Examples:

```text
trail/node@1 implementation 0.4.0 builtin
trail/cargo@1 implementation 0.4.0 builtin
acme/protobuf@1 implementation 2.3.1 sha256:93d…
```

The contract major participates in normalized component identity. The implementation
version participates when it can change output. An adapter may declare a narrower
`fingerprint_compatibility` version only when conformance tests prove output-compatible
behavior.

## Host/adapter interface

The following Rust-like interface is conceptual. Concrete Rust, subprocess, and WASI
bindings use serializable request and response types with equivalent semantics.

```rust
trait EnvironmentAdapter {
    fn metadata(&self) -> AdapterMetadata;

    // Read-only and side-effect free.
    fn discover(&self, request: DiscoverRequest) -> Result<Discovery>;

    // Returns declarations used by the host's canonical fingerprint engine.
    fn fingerprint(&self, request: FingerprintRequest) -> Result<FingerprintPlan>;

    // Returns actions and output contracts; does not execute them.
    fn plan(&self, request: PlanRequest) -> Result<ComponentPlan>;

    // Optional structured probes around host-executed actions.
    fn inspect_staging(&self, request: InspectRequest) -> Result<Inspection>;

    // Returns deterministic validation rules and semantic observations.
    fn validate(&self, request: ValidateRequest) -> Result<Validation>;

    // Returns proposed bindings and runtime resources.
    fn bind(&self, request: BindRequest) -> Result<BindingPlan>;

    // Explains an observed input, incompatibility, or stale transition.
    fn explain(&self, request: ExplainRequest) -> Result<Explanation>;
}
```

There is intentionally no `publish`, `mount`, `write_database`, `resolve_secret_value`,
or `mark_ready` method. Those actions remain host-owned.

## Discovery

Discovery receives:

- repository and requested discovery root;
- a read-only view restricted to approved paths;
- filenames and small metadata already found by the host, where possible;
- platform and host capability summaries;
- organization and repository policy;
- no secret values and no network access.

It returns component proposals, input candidates, graph edges, warnings, and ambiguous
choices. Discovery must not run installers, execute repository code, contact registries,
or create files.

Each proposal contains a stable logical ID. Logical IDs are readable within a
repository, such as `web.dependencies`, `rust.toolchain`, or `dev.postgres`; they are not
global artifact identities.

When multiple adapters claim the same manifest or output path, the host reports the
conflict unless one adapter declares composition with the other. Priority alone cannot
silently change a safety policy.

## Fingerprint plan

The adapter returns inputs, not a precomputed opaque digest. The host reads and hashes
files through its own normalized path and metadata rules.

```rust
struct FingerprintPlan {
    inputs: Vec<InputDeclaration>,
    tool_probes: Vec<ToolProbe>,
    parent_edges: Vec<IdentityEdge>,
    options: CanonicalValue,
    portability: PortabilityClass,
    environment_allowlist: Vec<EnvironmentInput>,
    policy_inputs: PolicyInputs,
    output_contract_digest: Digest,
}
```

### Input declarations

An input declaration includes:

- normalized path or external identity source;
- role: identity, tool identity, policy, runtime, health, secret reference, or ignored;
- content interpretation: bytes, normalized text, parsed semantic value, tree, symlink,
  executable probe, or external digest;
- missing behavior: optional, error, or explicit sentinel;
- path case and Unicode normalization policy;
- whether mode and executable bits matter;
- a human explanation used by stale reports.

Glob expansion is performed by the host and sorted canonically. Globs cannot escape the
discovery root. Semantic parsers must be versioned because normalized parse output is an
identity input.

### Tool probes

Tool identity cannot rely on an unqualified version string alone. A probe declares:

- executable source: bound toolchain component, repository path, or approved host path;
- argument vector and expected output format;
- optional executable content digest;
- version, target, ABI, and configuration fields to extract;
- timeout and locale;
- whether the resolved absolute path is identity-bearing.

Host tools are conservative `host` portability inputs unless policy maps them to a
trusted toolchain identity.

## Component plans

A component plan is a pure description of required actions and outputs:

```rust
struct ComponentPlan {
    component_key_inputs: FingerprintPlan,
    actions: Vec<Action>,
    outputs: Vec<OutputDeclaration>,
    caches: Vec<CacheDeclaration>,
    validations: Vec<ValidationRule>,
    bindings: Vec<BindingDeclaration>,
    runtime_resources: Vec<RuntimeDeclaration>,
    capabilities: CapabilityRequest,
    recovery: RecoveryPolicy,
}
```

Plans are stable for the same canonical request. Volatile discoveries such as a free
host port belong to activation-time runtime allocation, not the component plan.

## Actions

Supported host actions begin narrowly:

- execute a command vector;
- copy or materialize declared inputs into staging;
- create a directory;
- write a generated file whose content is included in the plan;
- unpack a verified archive;
- fetch a pinned external artifact through a host provider;
- invoke another component through a graph edge.

Each command declares:

- executable binding or approved path;
- argument vector;
- working directory relative to the sandbox;
- input and output roots;
- normalized non-secret environment additions;
- secret handles required and permitted injection form;
- network mode and endpoint allowlist;
- lifecycle-script policy;
- timeout, resource class, and cancellation behavior;
- expected exit codes;
- redaction fields.

Shell strings require the `process.shell` capability and a visible policy approval.
Command previews display quoted arguments but never reconstructed secret values.

Actions execute inside host-owned staging. The host exposes source read-only unless an
output explicitly requires a private writable source projection. An action cannot
write directly into a shared artifact root or active lane binding.

## Output declarations

Every produced root declares:

```rust
struct OutputDeclaration {
    name: String,
    kind: OutputKind,
    source: RelativePath,
    policy: StoragePolicy,
    mount_target: Option<WorkspacePath>,
    portability: PortabilityClass,
    mutation_expectation: MutationExpectation,
    allowed_file_types: FileTypePolicy,
    containment: ContainmentPolicy,
    retention: RetentionPolicy,
}
```

The host validates:

- source remains inside staging;
- mount target is normalized and non-reserved;
- no undeclared output overlaps another root;
- symlinks and hard links obey containment rules;
- devices, FIFOs, sockets, setuid bits, and extended attributes obey policy;
- case-folding collisions cannot break the target platform;
- declared portability matches observed native content where detectable;
- immutable output does not contain known transient or secret paths.

`mutation_expectation` is one of `never`, `consumer_may_copy_up`, `private_mutable`, or
`tool_managed_cache`. It must agree with the selected storage policy.

## Bindings

Adapters propose bindings; the host resolves their sources to artifact or private-state
identities and detects collisions. Bindings can target:

- a workspace-relative path;
- a sandbox-only path;
- an environment variable;
- a file descriptor;
- a socket or allocated port;
- a service name;
- a container volume;
- an executable name in a composed `PATH`.

Environment composition is deterministic. For variables such as `PATH`, adapters return
typed prepend, append, set, or map-merge operations. Conflicting scalar sets are errors
unless the repository specification explicitly selects precedence.

## Validation

Validation combines host-generic and adapter-semantic rules.

Host-generic validation includes containment, tree manifesting, file-type policy,
portability observations, reserved-path checks, and secret scanning.

Adapter-semantic validation may include:

- package manager integrity or lockfile correspondence;
- compiler or linker smoke probes;
- required executable and library presence;
- CMake cache compatibility;
- OCI manifest and platform matching;
- service protocol health;
- framework-specific directory structure.

A semantic validator reads staged output through a read-only capability. It cannot
repair content. Validation produces rule IDs, results, evidence summaries, and
redaction-safe diagnostics. Non-deterministic network health probes belong to runtime
health, not immutable publication.

## Declarative repository specification

The proposed file is `.trail/environment.toml`. The final location remains an open
architecture decision, but the schema below is independent of location.

```toml
schema = "trail.environment/v1"

[environment]
name = "developer-and-agent"
discovery_roots = ["."]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "web.dependencies"
adapter = "trail/node@1"
root = "apps/web"
extends = ["profile.nextjs"]

[[component.input]]
path = "apps/web/package.json"
role = "identity"
format = "json-semantic"

[[component.input]]
path = "pnpm-lock.yaml"
role = "identity"
format = "yaml-semantic"

[[component.output]]
name = "modules"
source = "node_modules"
target = "apps/web/node_modules"
policy = "immutable_seed_private"
portability = "abi"

[[component.cache]]
name = "pnpm-store"
kind = "content"
sharing = "repository"
compatibility = ["os", "arch", "node_abi", "pnpm_major"]

[component.build]
command = ["pnpm", "install", "--frozen-lockfile"]
cwd = "apps/web"
network = { allow = ["registry.npmjs.org"] }
scripts = "repository-policy"
timeout = "20m"

[[component.validation]]
kind = "command"
command = ["node", "-e", "require('next/package.json')"]
network = "deny"

[[component]]
id = "dev.api-token"
adapter = "trail/secret@1"

[component.secret]
provider = "os-keychain"
name = "team-api-token"
inject = { env = "TEAM_API_TOKEN" }
scope = "command"

[[component]]
id = "dev.postgres"
adapter = "trail/oci-service@1"

[component.service]
image = "postgres@sha256:0123456789abcdef"
network = "lane-private"
ports = [{ container = 5432, host = "auto", export_env = "PGPORT" }]
health = { command = ["pg_isready", "-U", "postgres"], timeout = "30s" }
cleanup = "trail-owned"
```

The full example remains the target schema. The implemented experimental subset accepts
one of `trail.environment.toml` or `.trail/environment.toml`, strict unknown-field
rejection, repository-local includes, versioned profiles with inheritance and `{root}`
expansion, and `trail/command@1` components. Each command component currently requires:

- a stable component ID and root;
- optional `depends_on = ["component.id"]` compatibility edges (equivalent to
  `build_requires`) or typed `[[component.edge]]` entries with `component` and `type`;
  supported types are `build_requires`, `runtime_requires`, `binds_after`, and
  `invalidates_with`, and every target must be present in the same discovered graph for
  `sync-all`;
- exact file/directory inputs or repository-relative globs, all with `role = "identity"`
  and `format = "bytes"`;
- between one and 32 named `kind = "generated"` outputs using
  one portability class and either `immutable_seed_private` or `writable_private`.
  A component cannot mix the policies yet;
- a non-shell executable name resolved from `PATH` plus an argv vector;
- `network = "deny"` and `scripts = "deny"`;
- no sensitive arguments, environment names, host paths, secret values, caches,
  pre-commands, services, or runtime resources.

`trail env plan` exposes the canonical key, exact executable identity, inputs, output,
and grants before execution. On macOS the command runs under `sandbox-exec`. On Linux a
fresh Trail helper applies a hard-requirement Landlock filesystem allowlist and a
seccomp filter that denies socket, namespace, kernel-module, ptrace, BPF, io_uring, and
related escape syscalls before executing the selected program. On Windows the helper
creates an ephemeral capability-free AppContainer, grants its SID read/execute access to
the staged root and modify access only to output/HOME/tmp, launches the exact executable
suspended, assigns it to a kill-on-close Job Object with an active-process limit of one,
and only then resumes it. A setup failure terminates the suspended process before the
handle is released. The platform backends expose staged inputs for reading,
output/HOME/tmp for writing, the selected executable for launch, and no network or child
processes. Missing kernel facilities and unsupported operating systems fail closed;
discovery and planning remain available. Windows behavior is guarded by a native CI
verifier covering successful publication and reuse plus denied host reads, undeclared
writes, network, child processes, and shells; that native runner, rather than a
cross-compile, is the release evidence for AppContainer and Dokan integration.
When several command components share a root, `--component <id>` selects one for
`env plan` or `env sync`; `env sync-all` continues to compose every non-conflicting
component atomically in deterministic dependency order.

### Includes and profiles

Specifications can include committed files within the repository and extend named
profiles. Includes resolve relative to the including file before canonicalization and
cannot be remote URLs, globs, or traversing paths. The host limits include count, depth,
individual size, and aggregate size, and reports the complete cycle when recursion is
detected.

Profiles package ecosystem conventions without hiding policy. Examples are
`profile.nextjs`, `profile.vite`, `profile.cargo-workspace`, and
`profile.cmake-ninja`. Expansion is inspectable and contributes the profile version to
the specification digest.

Organization profiles will be referenceable by signed policy identity, but a generation
records the resolved canonical content so later reproduction does not depend on mutable
profile names.

### User-defined command components

Repositories need a general escape hatch that does not require writing a plugin:

```toml
[[component]]
id = "protobuf.generated"
adapter = "trail/command@1"

inputs = [
  { path = "proto/**/*.proto", role = "identity", format = "bytes" },
  { component = "tools.protoc", role = "tool_identity" },
]

[component.build]
command = ["protoc", "--cpp_out=out", "proto/service.proto"]
cwd = "."
network = "deny"

[[component.output]]
name = "cpp-sdk"
source = "out"
target = ".trail-generated/protobuf"
policy = "immutable_shared"
portability = "universal"
```

The generic command adapter requires complete input/output declarations and defaults to
read-only source, denied network, denied shell, denied scripts, and no secrets.

## Cache contract

An adapter declares cache semantics, not only a directory path:

```rust
struct CacheDeclaration {
    name: String,
    protocol: CacheProtocol,
    compatibility: Vec<CompatibilityDimension>,
    scope: SharingScope,
    access: CacheAccess,
    authority: CacheAuthority,
    eviction: EvictionPolicy,
}
```

- `protocol`: content store, concurrent daemon, locked index, or private seeded cache;
- `access`: read-only, single-writer, host-mediated, or tool-concurrent;
- `authority` must be `performance_only` for ordinary caches;
- compatibility includes every dimension that can make an entry unsafe;
- the host can narrow scope but cannot widen it beyond adapter certification.

Adapters must demonstrate that eviction cannot change correctness. If a tool mixes
authoritative mutable state with cached content, the adapter separates paths or marks
the root writable private.

The implemented schema-v10 built-in contract derives a `cache_<sha256>` namespace from
adapter identity, cache name, protocol, access strategy, workspace scope, and a bounded
non-secret compatibility map. Supported protocols are `content_store`,
`compiler_cache`, and `locked_index`; access is either certified `tool_concurrent` or
Trail-mediated `host_exclusive`. Commands reference symbolic cache names, never raw
writable directories. Trail creates the exact host path, records use in component and
generation provenance, holds crash-recoverable leases during execution, and coordinates
GC with an exclusive maintenance barrier. Node, Cargo, sccache, and Go use this path.
Protocol-v2 external plugins can declare the same three protocols for a staging action.
They provide only a bounded non-secret compatibility map and environment-variable to
relative-subpath bindings; the host adds the authenticated distribution digest,
negotiated protocol, OS, and architecture, resolves absolute paths, and projects only
the selected namespace into the native sandbox. External caches are conservatively
`host_exclusive`, so they are safe without trusting a tool's internal locking. Mounted
actions receive no cache access, namespace escape attempts fail in the kernel sandbox,
and planning remains side-effect-free. `tool_concurrent` remains limited to built-ins
until the exact external adapter/tool protocol has independent concurrency
certification. Repository command recipes remain cache-free because repository-authored
commands do not carry an authenticated adapter contract.

## Runtime and service contract

Runtime declarations separate immutable identity from allocation:

```rust
struct RuntimeDeclaration {
    kind: RuntimeKind,
    immutable_source: Option<ComponentRef>,
    isolation: RuntimeIsolation,
    allocation: AllocationRequest,
    health: Vec<HealthRule>,
    reuse: ReusePolicy,
    cleanup_owner: CleanupOwner,
}
```

Allocation values such as host ports, process IDs, container IDs, and socket paths do
not enter immutable component keys. They enter the environment generation and execution
record. Reuse requires matching immutable identity, compatible configuration, same lane
ownership, and successful health checks.

An adapter may return external-resource declarations, but it may not create or delete
external infrastructure unless a separate provider capability and cleanup ownership
are explicitly granted.

## Secret contract

Planning sees only:

```rust
struct SecretReference {
    provider: String,
    logical_name: String,
    version_selector: Option<String>,
    purpose: String,
    injection: SecretInjection,
    scope: SecretScope,
}
```

The host resolves values. A plugin receives a one-use handle or an already-bound file
descriptor where possible. If a consumer requires an environment value or file, the
host performs injection directly into the child process sandbox.

Adapters declare which command argument or output fields may echo credentials so the
host can add structured redaction. This declaration supplements, but never replaces,
host-wide exact-value redaction and secret scanning.

## Plugin protocol and capabilities

The implemented v1/v2 transport is a natively sandboxed subprocess using one
length-prefixed CBOR request and response. WASI components are a future transport for
the same models. Local plugins are installed by digest and declare:

- adapter identity and supported planner protocols;
- component kinds and manifest patterns;
- bounded repository-relative read patterns and file/byte limits;
- timeout and response-size limits;
- one executable digest and experimental stability.

The planner capability set is deliberately fixed rather than open-ended: bounded pinned
bytes in, discovery or plan data out, no direct repository reads or writes, no child
process, no network, no shell, no secrets, no database, and no mount/publication
authority. Host-executed actions separately receive only their declared input/output
contract plus isolated HOME/tmp. V2 adds only host-executed typed action phases. A mounted
action runs in a pinned ephemeral candidate view, reads exact declared inputs, writes
only declared writable-private outputs plus isolated HOME/tmp, and cannot activate
partial state. Commands whose runtime wrapper needs another executable fail closed;
child-runtime permission remains a future separately certified capability.

Capability examples:

```text
fs.read.repository:apps/web
fs.read.staging
process.plan
process.inspect_output
network.plan:registry.npmjs.org
secret.reference:team-registry
runtime.plan:oci-container
```

`process.plan` lets a plugin propose a command; it does not let the plugin spawn it.
Network access during discovery or a v1/v2 host-executed action is not supported.

Plugins use request IDs and deterministic serialized values. Unknown enum variants are
handled through contract negotiation rather than silently downgraded. The host applies
deadlines and output-size limits to every call.

## Ecosystem mappings

The following mappings show how one policy vocabulary fits very different toolchains.

### Node, Next.js, and Vite

| Concern | Inputs | Policy/binding |
| --- | --- | --- |
| Node toolchain | version file, resolved distribution digest, platform/ABI | `immutable_shared`, added to `PATH` |
| Installed dependencies | package manifest, lockfile, package-manager version, Node ABI, install/script policy | `immutable_seed_private` at `node_modules` when consumers may modify it; `immutable_shared` only for proven read-only layouts |
| Package download store | registry identity and package-manager compatibility | `cache_shared_content` |
| Next build state | source/config identity for readiness, but continuously mutable during dev | `writable_private` at `.next` |
| Vite cache/output | framework mode and source/config | `.vite` private or disposable; `dist` immutable only for an explicit build artifact |
| Dev server | generation plus command configuration | `runtime_private`, auto port and lane-private process |

Next.js and Vite are profiles composed over the Node adapter. They do not duplicate
package installation logic.

### Rust and Cargo

| Concern | Inputs | Policy/binding |
| --- | --- | --- |
| Rust toolchain | `rust-toolchain.toml`, distribution digest, target components | `immutable_shared` toolchain |
| Registry crates and Git checkouts | Cargo version, source registry, checksums | `cache_shared_content` with host-mediated indexes |
| Compiled dependency seed | `Cargo.lock`, features, profiles, rustc identity, target, relevant build-script environment | optional `immutable_seed_private` lower for `target` |
| Active target directory | source and commands continuously mutate it | `writable_private` at `target` |
| Compiler cache | compiler identity, target, flags | `cache_shared_compiler` through sccache protocol |
| Credentials | registry token | `secret_runtime`, injected only into Cargo command |

Cargo build scripts and proc macros make complete hermeticity difficult. Their permitted
environment and network policy are part of the fingerprint/provenance, and portability
is conservative.

### CMake, Ninja, Make, vcpkg, and Conan

| Concern | Inputs | Policy/binding |
| --- | --- | --- |
| Toolchain | compiler/linker digests, SDK/sysroot, toolchain file, generator | `immutable_shared` or `external_immutable` |
| Downloaded dependencies | lockfiles/manifests, registry identity, triplet/profile | adapter-specific shared content cache |
| Configure/build tree | `CMakeLists.txt`, presets, generator, absolute source/build paths | `writable_private`; generally host-bound |
| ccache | compiler/flag compatibility | `cache_shared_compiler` |
| Install prefix | explicit install action and full dependency identity | `immutable_shared` if relocatable and validated; otherwise host-bound seed/private |
| Test scratch | test-specific generated files | `disposable` |

CMake adapters must account for absolute paths in `CMakeCache.txt`. A private build tree
cannot be promoted to a portable immutable artifact merely because compilation
succeeded.

The implemented `trail/cmake-build@1` adapter covers the safe first slice: discovery,
deterministic host/tool compatibility identity, atomic lane-private build-directory
ownership, generation provenance, mounted-view classification, and crash recovery.
`env sync` provisions `<component>/build` but does not run configure. Run configure and
build inside the lane:

```sh
trail env sync <lane> --adapter trail/cmake-build@1
trail lane exec <lane> -- cmake -S . -B build -G Ninja
trail lane exec <lane> -- cmake --build build
```

Real Linux/FUSE and macOS/NFS conformance configures and builds the same project in two
lanes, verifies that each cache records its mounted lane path, cleans one build, and
requires the other executable to remain intact. Presets, toolchain files, ccache,
vcpkg/Conan and optional relocation-validated install prefixes remain later slices of
006.7. A native Windows/Dokan two-lane CMake verifier is wired into CI; observed release
evidence remains pending because the local cross-check cannot build `aws-lc-sys` without
the MinGW compiler and cannot substitute for native Dokan execution.

### Docker and OCI workflows

| Concern | Inputs | Policy/binding |
| --- | --- | --- |
| Base image | resolved manifest digest and platform | `external_immutable` |
| Image build | Dockerfile, context subset, build args excluding secret values, frontend version | resulting OCI digest is `external_immutable` |
| Build cache | BuildKit protocol and builder identity | tool-managed shared cache with explicit scope |
| Build secrets | provider references | `secret_runtime` mounts; never image layers or keys |
| Running container | image digest, runtime config, lane network | `runtime_private` |
| Named volume | data ownership and lifecycle | `writable_private` if Trail-owned; `external_managed` otherwise |
| Ports and networks | allocation policy | generation-bound runtime resources |

Tags are discovery inputs; resolved digests are generation identities. Trail must never
delete a user-owned image, container, volume, or remote builder cache.

### Python

The experimental `trail/python-venv@1` built-in implements the safe baseline today. It
owns `<component>/.venv` as `writable_private`, publishes no shared layer, and preserves
the directory across a compatible re-sync. Synchronization automatically runs
`python -m venv --without-pip .venv` at the lane's final mountpoint:

```sh
trail env sync <lane> --adapter trail/python-venv@1
# Optionally let a lockfile-aware tool populate the initialized private path:
trail lane exec <lane> -- uv sync --frozen
```

Mounted initialization never runs against the active lane upper. Trail constructs an
ephemeral candidate view with the pinned source root, desired immutable bindings, and
prepared private seeds; source and unrelated writes land in disposable uppers. After
the command succeeds, Trail rejects every write outside the newly prepared private
outputs, copies those outputs back to staging, and performs the ordinary atomic
generation activation. A non-zero exit, undeclared write, process kill, or host crash
therefore leaves the predecessor generation and its private upper unchanged. Recovery
also removes abandoned candidate directories.

Real Linux/FUSE, macOS/NFS, and Windows/Dokan conformance creates two virtual
environments, verifies that `sys.prefix` identifies each mounted lane, mutates one
environment, and requires the other lane to remain unchanged. It also verifies that a
compatible re-sync preserves the private environment and creates no shared layer.
Additional native fixtures cover multi-component `sync-all`, initializer failure,
undeclared source writes, kill-point recovery, and abandoned-candidate cleanup.

Sharing interpreter distributions and wheel/download content safely remains a separate
typed-cache slice; Trail must not represent a path-bearing virtual environment itself as
a portable immutable artifact without relocation validation.

| Concern | Inputs | Policy/binding |
| --- | --- | --- |
| Interpreter | distribution digest, version, ABI | immutable toolchain |
| Wheels/downloads | lockfile, index identity, platform tags | shared content cache |
| Virtual environment | lockfile, interpreter identity, install policy | immutable seed/private when tools rewrite entry points; immutable only after relocation validation |
| Bytecode/test caches | source/runtime dependent | private or disposable |
| Index credentials | provider reference | runtime secret |

### Go

| Concern | Inputs | Policy/binding |
| --- | --- | --- |
| Go toolchain | distribution digest and target | immutable toolchain |
| Module cache | `go.sum`, proxy identity, tool version | shared content cache with tool-compatible locking |
| Build cache | compiler/target/options | shared compiler cache when tool protocol permits |
| Generated outputs | generator identity plus declared source | immutable artifact or private tree according to consumption |

These mappings are defaults, not universal truths. A repository can narrow sharing or
choose a private policy when native modules, hooks, absolute paths, or tool behavior make
sharing unsafe.

## Adapter composition

Framework adapters extend lower-level adapters through typed composition:

```text
Next.js profile
  requires Node toolchain
  requires Node dependency component
  adds private .next state
  adds optional runtime dev server
  adds framework validation
```

Composition rules:

- a child may add inputs, validation, bindings, and runtime resources;
- a child may narrow storage sharing, network, scripts, and portability;
- widening capability or sharing requires explicit user/organization approval;
- output ownership is unique after expansion;
- parent adapter version and normalized profile expansion are identity inputs;
- error explanations retain the responsible adapter and source field.

## Conformance suite

Every adapter must pass a host-provided test kit before receiving a certification tier.

### Determinism

- identical canonical requests produce identical fingerprint plans and component plans;
- input ordering, directory enumeration, locale, and map ordering do not change keys;
- every observed output-changing input is declared;
- irrelevant source edits do not invalidate the component.

### Isolation

- two lanes attach the same artifact while private writes remain invisible to each
  other;
- copy-up, whiteout, nested delete, bulk replacement, and rename behave correctly;
- an adapter cannot escape staging through `..`, symlinks, hard links, archives, or
  absolute paths;
- commands cannot write repository source unless expressly granted.

### Publication and recovery

- failures and cancellation never publish partial output;
- concurrent identical builds publish one result;
- crashes at each action, validation, publication, and activation boundary recover;
- corruption makes an artifact unavailable to new generations without destroying
  evidence or old generation records.

### Security

- network, scripts, shell, host paths, and secret capabilities are denied by default;
- secret canaries do not appear in keys, manifests, logs, transcripts, diagnostics,
  checkpoints, or exports;
- credentials embedded in URLs and tool output are redacted;
- external cleanup respects ownership.

### Portability

- platform and ABI changes invalidate when required;
- symlink, executable-mode, case-collision, and absolute-path fixtures narrow or reject
  portability correctly;
- every claimed workspace backend passes the shared filesystem suite.

### Scale

- real ecosystem fixtures, not only synthetic files, cover large Next.js/Vite
  `node_modules`, Cargo registries and `target`, CMake/Ninja builds, and OCI layers;
- warm plan and attach do not recursively scan unchanged artifacts;
- metadata-heavy replacement reports progress and supports cancellation;
- physical byte accounting demonstrates cross-lane sharing.

## Certification tiers

| Tier | Meaning |
| --- | --- |
| `experimental` | Contract-valid; incomplete ecosystem and scale coverage; explicit opt-in. |
| `isolated` | Determinism, containment, secret, and multi-lane isolation suites pass. |
| `reproducible` | Hermetic or fully declared inputs, validated portability, and rebuild comparison pass on supported platforms. |
| `builtin` | Maintained with Trail, migration guarantees, all supported backends, and release-gating fixtures. |

Certification applies to an adapter version and declared platform matrix, not its name
forever. Repository status reports the active tier.

## Compatibility and evolution

- Additive optional fields preserve a contract major.
- New required semantics, changed canonicalization, or changed policy meaning requires a
  new major.
- The host can run multiple adapter majors so old generations remain inspectable.
- A migration creates a new component identity and generation; it never rewrites an old
  manifest.
- Deprecations appear in discovery and plan reports with a mechanical replacement when
  possible.
- Unknown policy classes, capabilities, and binding types fail closed.

## Minimum viable adapter SDK

The first SDK currently provides:

- strongly typed request/response models;
- deterministic v1/v2 plan, typed dependency/action, command, output, discovery, and response
  authoring helpers with local structural validation and unchanged v1 Rust/wire types;
- length-prefixed binary-safe CBOR framing with pre-allocation limits;
- declarative pinned input, discovery, command, output, portability, and error types;
- a one-request stdio server helper;
- generated-copy, mounted-initializer, direct mounted-tool, and adversarial reference adapters;
- host-side canonical path, identity, capability, resource, and provenance validation;
- host-owned `mounted_initialization` actions for trusted built-ins and authorized v2
  plugins, executed at the stable lane path through disposable candidate uppers before
  atomic activation;
- macOS/Linux and Windows-native conformance verifiers.

Still required before SDK stabilization are standard tool-probe helpers,
signed-package helpers, a standalone fixture host, golden stale-explanation
helpers, secret-canary utilities, progress events, and certification metadata.

Adapters should not need to know Trail's database schema, mount implementation, NFS
protocol, overlayfs details, or artifact-store layout.

## Contract acceptance criteria

The contract is ready for stabilization when:

- built-in Node and Cargo implementations can express existing behavior without private
  host escape hatches;
- a generic command recipe can build and attach a verified immutable artifact;
- CMake can model private build trees and optional immutable install prefixes;
- OCI can separate image identity, BuildKit cache, secret mounts, and lane runtime;
- framework profiles compose without claiming the same output twice;
- deterministic fingerprint and stale-explanation golden tests pass;
- capabilities shown in a plan exactly match those observed during execution;
- plugin failure or timeout cannot corrupt Trail state;
- unknown versions and fields fail predictably;
- the same normalized plan drives CLI, HTTP, MCP, and Rust API reports.

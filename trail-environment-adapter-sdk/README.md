# Trail environment adapter SDK

Use this crate when a new ecosystem needs semantic discovery or planning that a
repository-declared `trail.environment.toml` command recipe cannot express. An adapter
is an isolated planner: it receives bounded, pinned file bytes and returns data. Trail
resolves tools, runs commands, validates outputs, publishes layers, attaches lane
mounts, updates state, and recovers failures.

## Choose the smallest extension

1. Use a command recipe for a fixed argv command with declarative inputs and outputs.
2. Use an isolated plugin when manifests must be parsed or the command/output contract
   must be selected semantically.
3. Add a Trail built-in only for a broadly used ecosystem that needs a richer sharing
   strategy and can pass Trail's full cross-platform conformance suite.

Plugins do not receive a repository path, database handle, mount authority, network,
secrets, a shell, or permission to spawn child processes. Protocol v1 proposes one
staging command. Protocol v2 may instead propose typed `staging` and
`mounted_initialization` actions, or a metadata-only set of pinned external artifacts
and host-managed runtime services;
the Trail host decides whether and how to execute actions and never delegates mount
creation, publication, or provider cleanup ownership.

## Minimal package

A package directory contains a manifest and its declared executable. Authenticated
packages add the optional detached signature described below:

```text
my-adapter/
├── trail-adapter.toml
├── trail-adapter.sig  # optional
└── my-adapter
```

```toml
schema = "trail.environment-adapter-package/v1"

[adapter]
canonical_identity = "acme/schema-codegen@1"
implementation_version = "1.0.0"
selectors = ["acme/schema-codegen@1", "schema-codegen"]
kind = "generated"
layer_adapter_name = "schema-codegen"
discovery_markers = ["schema.codegen.toml"]
protocols = ["trail.environment-adapter/v1"]
supported_operating_systems = ["linux", "macos", "windows"]
supported_architectures = ["aarch64", "x86_64"]
stability = "experimental"
description = "Generates code from a pinned schema manifest"

[executable]
path = "my-adapter"
sha256 = "sha256:<lowercase SHA-256 of my-adapter>"

[permissions]
read_patterns = ["schema.codegen.toml", "schema/**/*.json"]
max_input_files = 4096
max_input_bytes = 8388608
timeout_ms = 5000
max_response_bytes = 4194304
```

The executable handles one length-prefixed CBOR request and writes one response. Start
from [`examples/generated-copy-adapter.rs`](examples/generated-copy-adapter.rs). Its
core shape is:

```rust,no_run
use trail_environment_adapter_sdk::{
    serve_once, AdapterRequest, AdapterResponse, AdapterResult,
};

fn main() {
    serve_once(|request| {
        let result = plan_or_discover(&request);
        AdapterResponse::for_request(&request, result)
    })
    .expect("serve one Trail adapter request");
}

# fn plan_or_discover(_: &AdapterRequest) -> AdapterResult {
#     AdapterResult::Error { code: "example".into(), message: "implement me".into() }
# }
```

For protocol v2, start from
[`examples/mounted-initializer-adapter.rs`](examples/mounted-initializer-adapter.rs).
The companion [`mounted-fixture-tool.rs`](examples/mounted-fixture-tool.rs) is a direct,
child-free executable used by the native FUSE/NFS/Dokan conformance flow.

Build plans through the SDK helpers to catch missing commands, empty fields, duplicate
inputs/dependencies/outputs, self-dependencies, and invalid output counts locally. Set-like
inputs and dependencies are sorted deterministically without changing the v1 wire format:

```rust
use trail_environment_adapter_sdk::{AdapterCommand, AdapterOutput, AdapterPlan};

let plan = AdapterPlan::builder("api.client", "generated")
    .dependency("api.schema")
    .identity_inputs(["schema.json", "generator.toml"])
    .semantic_input("strategy", "client-v1")
    .command(AdapterCommand::new("schema-codegen", ["--out", "generated"]))
    .output(AdapterOutput::immutable_seed_private(
        "client",
        "generated",
        "src/generated",
    ))
    .stale_reason("schema, generator, or strategy changed")
    .build()?;
# Ok::<(), trail_environment_adapter_sdk::AdapterPlanBuildError>(())
```

For `discover`, return either no component or a stable component ID and declared kind.
For a protocol-v1 `plan`, return:

- every supplied file path in `identity_inputs`—Trail rejects inspected-but-unkeyed
  inputs;
- bounded semantic inputs that affect output identity;
- zero or more stable logical component IDs in `dependencies`; Trail validates the full
  graph and keys each edge with the finalized upstream component key;
- an executable name plus argv, working directory, and non-sensitive environment;
- one to 32 non-overlapping owned outputs, their lane mount targets, and an explicit
  `immutable_seed_private` (default) or `writable_private` policy;
- conservative portability and an actionable stale reason.

Protocol v2 uses `AdapterPlanV2` and `AdapterAction` without changing the v1 Rust or
wire types. Select it in `trail-adapter.toml`:

```toml
[adapter]
protocols = ["trail.environment-adapter/v2"]
```

A mounted-only plan is authored as:

```rust
use trail_environment_adapter_sdk::{AdapterCommand, AdapterOutput, AdapterPlanV2};

let plan = AdapterPlanV2::builder("api.venv", "dependency")
    .identity_input("pyproject.toml")
    .build_requires("python.toolchain")
    .runtime_requires("dev.database")
    .mounted_command(AdapterCommand::new("venv-tool", ["init", ".venv"]))
    .output(AdapterOutput::writable_private("venv", ".venv", ".venv"))
    .stale_reason("manifest, tool, platform, or action changed")
    .build()?;
# Ok::<(), trail_environment_adapter_sdk::AdapterPlanBuildError>(())
```

V2 permits at most one staging action plus eight mounted actions. Any mounted action
requires every output to be `writable_private`. Trail resolves and rechecks every
executable, stages authenticated executable bytes into an isolated process directory,
mounts a pinned ephemeral candidate at the final lane path, allows reads only from
declared identity inputs, and allows writes only below declared outputs plus isolated
HOME/tmp. Network, shell, secrets, and undeclared child execution remain denied. Only
validated output trees are copied into atomic activation staging; failure leaves the
predecessor generation unchanged. A parent-death watchdog terminates the sandbox helper
if Trail is killed during an active action; backend recovery detaches an abandoned mount
before touching its path and removes the candidate attempt.

V1 `dependencies` and the v2 `.dependency(...)` compatibility helper mean
`build_requires`. V2 additionally provides `.build_requires(...)`,
`.runtime_requires(...)`, `.binds_after(...)`, and `.invalidates_with(...)`.
Build and invalidation edges contribute the exact upstream key to artifact identity;
runtime and binding-order edges are recorded with the exact upstream generation but do
not force an otherwise identical artifact to rebuild. A component ID may occur in only
one dependency declaration.

Protocol-v2 staging actions may also request performance-only host caches. Start from
[`examples/cache-adapter.rs`](examples/cache-adapter.rs); its companion
[`cache-fixture-tool.rs`](examples/cache-fixture-tool.rs) is exercised by the native
two-lane conformance flow:

```rust
use trail_environment_adapter_sdk::{
    AdapterCache, AdapterCacheProtocol, AdapterCommand, AdapterOutput, AdapterPlanV2,
};

let plan = AdapterPlanV2::builder("api.codegen", "generated")
    .cache(
        AdapterCache::host_exclusive("package-store", AdapterCacheProtocol::ContentStore)
            .compatibility_dimension("generator", "schema-codegen@1")
            .environment_variable("SCHEMA_CODEGEN_CACHE", "."),
    )
    .staging_command(AdapterCommand::new("schema-codegen", ["build"]))
    .output(AdapterOutput::immutable_seed_private(
        "client",
        "generated",
        "src/generated",
    ))
    .stale_reason("schema, generator, or strategy changed")
    .build()?;
# Ok::<(), trail_environment_adapter_sdk::AdapterPlanBuildError>(())
```

The adapter declares semantics and relative environment bindings, never a host path.
Trail adds the authenticated distribution digest, negotiated protocol, OS, and
architecture to namespace compatibility; injects the absolute namespace only while the
staging command runs; holds a crash-recoverable lease and lock; projects only that path
through the native sandbox; records it in plan/generation provenance; and coordinates
garbage collection. Mounted actions receive no cache access. External adapters are
currently restricted to `host_exclusive`; `tool_concurrent` declarations fail closed
until that exact adapter/tool cache protocol has independent concurrency certification.
Cache eviction must affect performance only, never output correctness.

Protocol v2 can also describe a provider-owned immutable identity without executing an
action or manufacturing a filesystem output:

```rust
use trail_environment_adapter_sdk::{AdapterExternalArtifact, AdapterPlanV2};

let plan = AdapterPlanV2::builder("images", "external")
    .identity_input("images.lock")
    .external_artifact(AdapterExternalArtifact::pinned_oci_image(
        "web",
        "ghcr.io/example/web@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "linux/amd64",
    ))
    .stale_reason("pinned image declaration changed")
    .build()?;
# Ok::<(), trail_environment_adapter_sdk::AdapterPlanBuildError>(())
```

External-artifact plans must use kind `external` and cannot mix actions, caches, or
outputs. Trail validates the digest/reference/platform tuple, includes the sorted
contract in the component key, persists it with each generation, and treats cleanup as
provider-owned. Registry access, tag resolution, credentials, and runtime allocation
are deliberately outside the planner.

An external plan may bind a pinned image to a lane-private service declaration. The
adapter still performs no provider calls and receives no Docker socket, network, port,
volume, or cleanup authority:

```rust
use trail_environment_adapter_sdk::{
    AdapterExternalArtifact, AdapterPlanV2, AdapterRuntimeResource, AdapterSecretReference,
};

let plan = AdapterPlanV2::builder("dev.database", "external")
    .identity_input("services.lock")
    .external_artifact(AdapterExternalArtifact::pinned_oci_image(
        "postgres-image",
        "ghcr.io/example/postgres@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "linux/amd64",
    ))
    .runtime_resource(
        AdapterRuntimeResource::oci_container("postgres", "postgres-image", 5432)
            .health_timeout_ms(45_000)
            .restart_policy("on_failure")
            .volume_target("/var/lib/postgresql/data")
            .secret(
                AdapterSecretReference::file(
                    "database-password",
                    "environment_file",
                    "DATABASE_PASSWORD_FILE",
                    "/run/secrets/database-password",
                    "authenticate the database service",
                )
                .version("rotation-7")
                .environment_variable("POSTGRES_PASSWORD_FILE"),
            ),
    )
    .stale_reason("service image or runtime contract changed")
    .build()?;
# Ok::<(), trail_environment_adapter_sdk::AdapterPlanBuildError>(())
```

Runtime declarations are sorted and keyed with the component. Each name must be unique,
must reference an artifact in the same plan, and currently describes an OCI container
with a TCP port and TCP health check. Trail allocates deterministic lane/generation
container and network names plus a logical lane-service volume; binds a persisted,
Trail-reserved host port only on
`127.0.0.1`; verifies ownership labels before adopting or stopping resources; and keeps
readiness blocked until health succeeds. `trail env sync` performs reconciliation after
activation. Operators can inspect or retry it explicitly:

```sh
trail env runtime status <lane>
trail env runtime reconcile <lane>
trail env runtime stop <lane>
```

Layered-lane commands and gates receive a deterministic `TRAIL_SERVICES_JSON`; services
whose resource name is unique also receive `TRAIL_SERVICE_<NAME>_HOST`, `_PORT`, and
`_ADDRESS`. Command launch fails closed until the active runtime generation is healthy.

Planning never resolves secret values. Service credentials and late-bound environment,
file, or descriptor injection require the separate opaque secret-reference contract.
The implemented portable provider contract accepts `file` (an absolute provider-owned
file) and `environment_file` (an environment variable containing that path). Trail does
not read the bytes. It validates a non-symlink regular file, a one-MiB bound, and private
permissions, then gives the runtime a read-only bind handle at `/run/secrets/...`.
An optional environment binding receives only that in-container file path, enabling
`*_FILE` conventions without placing the credential itself in container metadata.
Resolution status and an access audit are stored without values. Raw environment-value
and file-descriptor injection remain denied for standalone Docker because its normal
environment path persists values in inspectable container metadata.

When a provider resolves the same declaration to a different canonical file handle,
Trail compares a non-secret binding digest on the owned container and recreates it before
use. This prevents a restarted or reconciled service from retaining a stale bind mount
after credential rotation.

Do not hash files yourself or omit files that appear irrelevant after inspection. Trail
keys the exact supplied set, the package and executable digests, protocol and
implementation versions, semantic inputs, tool identity, command, output contract,
platform, architecture, and capability contract.

Dependencies are `build_requires` edges in protocol v1; they do not grant the
plugin access to another component's staged or mounted files. Use `trail env sync-all`
to construct the whole graph atomically. A single-component sync fails with an
actionable error unless every declared dependency is already ready in that lane.

Use `writable_private` for output owned by one lane. Protocol v1 must initialize it in
Trail's temporary staging directory. Protocol v2 can request mounted initialization for
path-sensitive state such as virtual environments or configure databases, subject to
the stricter mounted-action contract above. Commands requiring wrapper-spawned runtime
executables must use a future explicitly certified child-runtime capability; Trail does
not silently weaken child-process denial.

## Publisher authentication

Unsigned local packages remain supported and are reported as
`local-experimental`. For authenticated distribution, first ask Trail for the canonical
payload digest:

```sh
trail --workspace /path/to/repo --json env plugin inspect ./my-adapter
```

Sign these exact bytes with Ed25519:

```text
"trail.environment-adapter-signature/v1" || NUL || "sha256:<payload hex>"
```

Store the detached signature beside the manifest as `trail-adapter.sig`:

```toml
schema = "trail.environment-adapter-signature/v1"
publisher = "acme"
key_id = "sha256:<SHA-256 of the 32 public-key bytes>"
payload_digest = "sha256:<digest reported by inspect>"
signature = "<128 lowercase hex characters>"
```

The consuming workspace explicitly trusts the public key:

```toml
# acme-adapter-key.toml
schema = "trail.environment-adapter-publisher-key/v1"
publisher = "acme"
public_key = "<64 lowercase hex characters>"
```

```sh
trail --workspace /path/to/repo env plugin trust add ./acme-adapter-key.toml
trail --workspace /path/to/repo env plugin trust list
```

Trail verifies the payload digest, content-derived key ID, trusted publisher ownership,
and Ed25519 signature before installation. The immutable distribution identity includes
the detached attestation, while the signed payload identity remains independently
inspectable. Revoking a key immediately makes active packages authenticated by that key
fail closed:

```sh
trail --workspace /path/to/repo env plugin trust remove sha256:<key-id>
```

Publisher authentication proves origin and byte integrity; it does not by itself grant a
stable certification tier. Signed external adapters are reported as
`publisher-authenticated-experimental` until a separate conformance certification
system exists.

## Install and exercise

```sh
trail --workspace /path/to/repo env plugin install ./my-adapter
trail --workspace /path/to/repo env adapters
trail --workspace /path/to/repo env discover my-lane
trail --workspace /path/to/repo env graph my-lane
trail --workspace /path/to/repo env plan my-lane --adapter acme/schema-codegen@1
trail --workspace /path/to/repo env sync my-lane --adapter acme/schema-codegen@1
trail --workspace /path/to/repo lane readiness my-lane
```

Installation verifies the manifest and executable digest, stores immutable package
bytes by distribution digest, and activates the identity locally. Unsigned local
packages must remain `experimental`. Removal is an append-only tombstone:

```sh
trail --workspace /path/to/repo env plugin remove acme/schema-codegen@1
```

Run the repository's native conformance verifier while developing Trail or the example
adapter:

```sh
scripts/verify-environment-adapter-plugin.sh
```

Windows uses `scripts/verify-windows-environment-adapter-plugin.ps1`. Release evidence
must cover every declared OS/architecture, deterministic planning, input-change
staleness, two-lane layer reuse, private copy-up, timeout/crash/memory/output limits,
malformed framing, child/network/filesystem denial, tamper detection, and recovery.

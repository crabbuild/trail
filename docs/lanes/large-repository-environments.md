# Large-repository lane environments

Trail makes large lanes cheap by separating correctness-bearing immutable
artifacts from lane-private mutation. A lane never shares a live Cargo target,
`node_modules`, CMake tree, framework state, database, secret, runtime, port,
or process with another lane.

## Agent A → B → C

```sh
trail lane spawn agent-a --from main
trail env sync all agent-a
trail lane exec agent-a -- cargo build --locked

trail lane spawn agent-b --from agent-a
trail lane exec agent-b -- cargo test --locked

trail lane spawn agent-c --from agent-b
trail lane exec agent-c -- cargo check --locked
```

Spawn is metadata-fast and does not execute Cargo. Managed execution discovers
the desired graph, compares canonical keys, attaches verified parent/cache
layers, creates fresh private uppers, and then launches the command. A full hit
records a skipped sync phase and launches no adapter, package manager, or
compiler preparation command.

## Framework-neutral recipe

```toml
schema = "trail.environment/v1"

[[component]]
id = "codegen.api"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "schema/api.txt", role = "identity", format = "bytes" }]
outputs = [{ name = "sdk", source = "generated", target = "sdk", policy = "immutable_shared", reuse = "exact", scope = "workspace", publish = "on_sync" }]

[component.build]
command = ["cp", "schema/api.txt", "generated/generated-client.txt"]
network = "deny"
scripts = "deny"
```

Use `immutable_shared` for read-only generated content,
`immutable_seed_private` for a reusable lower plus fresh lane upper,
`writable_private` for persistent lane-only state, and `disposable` for scratch.
Never model a live mutable tree as shared.

## Executable Cargo and Node examples

The built-in adapters need no framework-specific Trail configuration. In a
Git-tracked Rust repository with `Cargo.toml` and `Cargo.lock`:

```sh
trail init --from-git
trail lane spawn cargo-a --from main
trail env plan cargo-a --adapter trail/cargo@1
trail env sync all cargo-a
trail lane exec cargo-a -- cargo test --locked
trail lane spawn cargo-b --from cargo-a
trail lane exec cargo-b -- cargo test --locked
```

The Cargo plan uses complete-source-root identity, publishes a verified target
seed as `immutable_seed_private`, and gives `cargo-b` a fresh target upper.
Cargo may reuse the seed and configured compiler cache; the two lanes never
write one live target directory.

In a Git-tracked Node repository with `package.json` and its lockfile:

```sh
trail init --from-git
trail lane spawn node-a --from main
trail env plan node-a --adapter trail/node@1
trail env sync all node-a
trail lane exec node-a -- npm test
trail lane spawn node-b --from node-a
trail lane exec node-b -- npm test
```

The immutable dependency lower is referenced by identity while consumer writes
go to each lane's private upper. Package-manager caches are performance-only
namespaces; they are never accepted as dependency correctness evidence.

The framework-neutral TOML above is executable as `trail.environment.toml`.
Create its declared input, record it, and run:

```sh
mkdir -p schema
printf 'client schema v1\n' > schema/api.txt
trail record -m "add command-recipe input"
trail lane spawn recipe-a --from main
trail env plan recipe-a --component codegen.api
trail env sync component codegen.api --lane recipe-a
trail lane exec recipe-a -- test -f sdk/generated-client.txt
```

Use a separate component for each ownership policy. A shared immutable SDK, a
seeded consumer-mutable dependency tree, a persistent private build tree, and
disposable test scratch must not be collapsed into one live mutable directory.

Every key includes adapter provenance, declared byte/Merkle identities (or the
complete source root when closure is not certified), identity-bearing upstream
keys, argv/cwd, tools, identity environment, output policy, platform,
portability, reuse, and scope. `trail env explain` reports changed dimensions.

For a private result declared with `publish = "manual"`, quiesce the lane:

```sh
trail env promote agent-a codegen.api result
```

Promotion journals the attempt, snapshots under the mutation barrier,
validates contained normalized content, publishes a sealed immutable layer,
and atomically advances a successor generation. The private source remains.

## Cache and disk controls

```sh
trail config set workspace_views.concurrent_cache_builders 4
trail config set workspace_views.cache_max_bytes 536870912000
trail config set workspace_views.cache_min_free_bytes 53687091200
trail config set workspace_views.cache_retention_secs 604800
trail config set workspace_views.prefetch_max_bytes 268435456
trail config set workspace_views.prefetch_max_entries 4096
```

`trail cache gc --dry-run` and execution share one deterministic report.
Active/retained generations, mounted views, builders, and publications are
pinned; callers can also use the Rust `pin_workspace_layer` API for an explicit
time-bounded or indefinite evidence pin. Private uppers, source, runtime, and
secrets are never cache candidates.

Successful managed commands record only bounded immutable-layer path accesses.
A later execution with the exact command fingerprint, component keys,
generation, and manifest identities may prefetch that authenticated hot set.
Prefetch is advisory and cancellable; lifecycle receipts report its entry and
byte limits, match state, cancellation, and bytes actually read.

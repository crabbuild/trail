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
trail env sync all cargo-a --path .
trail lane exec cargo-a -- cargo test --locked
trail lane spawn cargo-b --from cargo-a
trail lane exec cargo-b -- cargo test --locked
```

The Cargo plan uses complete-source-root identity, publishes a verified target
seed as `immutable_seed_private`, and gives `cargo-b` a fresh target upper.
For a source-only handoff, construction of B's new exact seed may start from
A's compatible active seed using clone/reflink; Cargo revalidates it and
recompiles affected workspace code. Manifest, lockfile, toolchain, target,
platform, and build-policy differences remain hard misses. The two lanes never
write one live target directory. `--path .` keeps independent nested Cargo
projects outside this root-component sync.

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

## Next.js and Vite composition with repository v2

Frameworks compose over dependency components; framework names do not receive
special sharing authority. This tested `trail.environment/v2` shape keeps
Next.js state private while allowing a validated Vite distribution to be
shared. Built-in Node discovery supplies the `node` dependency component.

```toml
schema = "trail.environment/v2"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "web.next-build"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["node"]
inputs = [{ path = "next-source.js", role = "identity", format = "bytes" }]
outputs = [{ name = "next-state", source = "next-output", target = ".next", policy = "writable_private", reuse = "none", scope = "lane", publish = "manual", portability = "host" }]
[component.build]
command = ["cp", "next-source.js", "next-output/server.js"]
cwd = "."
network = "deny"
scripts = "deny"

[[component]]
id = "web.vite-build"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["node"]
inputs = [{ path = "vite-source.js", role = "identity", format = "bytes" }]
outputs = [{ name = "dist", source = "dist", target = "dist", policy = "immutable_shared", reuse = "exact", scope = "workspace", publish = "on_sync", portability = "host" }]
[component.build]
command = ["cp", "vite-source.js", "dist/app.js"]
cwd = "."
network = "deny"
scripts = "deny"

[[component.validation]]
name = "dist-path-contract"
kind = "path_contract"
path = "dist"
required = true
parameters = { maximum_entries = "1000" }
```

Create the two declared input files, record them, then verify the graph and
execute the exact component closure:

```sh
trail lane spawn web-a --from main
trail env discover web-a
trail env graph web-a
trail env plan web-a --component web.vite-build
trail env sync component web.vite-build --lane web-a
trail lane exec web-a -- test -f dist/app.js
trail lane spawn web-b --from web-a
trail lane exec web-b -- test -f dist/app.js
```

Both lanes can attach the Vite content root. Their `.next`, `.vite`, incremental
compiler, and daemon state remains in fresh lane-private uppers. If generated
client code must become source, add a `[[component.source_export]]` declaration
and invoke `trail env source export`; do not point a shared output directly into
the repository tree.

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

Use `trail --format json lane space <lane>` and
`trail --format json cache gc --dry-run` for artifact-aware accounting. The
`artifact_storage` fields keep logical, authoritative CAS, physical, and
reclaimable axes separate. Cache-GC accounting describes the pre-deletion
snapshot and its `reclaimable_bytes` is the exact selected candidate set.

Successful managed commands record only bounded immutable-layer path accesses.
A later execution with the exact command fingerprint, component keys,
generation, and manifest identities may prefetch that authenticated hot set.
Prefetch is advisory and cancellable; lifecycle receipts report its entry and
byte limits, match state, cancellation, and bytes actually read. Those reads
warm the operating-system page cache without creating a persisted prefetch
store, so storage accounting reports `prefetched_bytes: 0`.

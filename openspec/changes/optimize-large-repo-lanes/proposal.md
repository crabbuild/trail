## Why

Trail can already mount transparent copy-on-write lanes, synchronize adapter-owned environments, and inherit compatible immutable layers, but users must still understand workdir modes, the `sync-all` distinction, and framework-specific cache behavior to obtain fast startup. Large repositories need one safe default workflow where a child agent receives reusable dependencies and validated build seeds immediately while mutable output remains isolated.

## What Changes

- **BREAKING** Make managed agent lanes select a qualified layered copy-on-write backend by default and fail with an actionable prerequisite report when no safe backend is available; never silently fall back to a full copied checkout for an environment-bearing execution.
- Make managed execution converge the desired environment lazily, skip work when the active generation is current, and inherit compatible parent components by reference during lane spawn.
- **BREAKING** Replace `trail env sync-all <lane>` with `trail env sync all [<lane>]` and move single-component synchronization to `trail env sync component <component> [--lane <lane>]`.
- Add a framework-neutral output contract that classifies generated directories by mutability, reuse, publication trigger, portability, and sharing scope rather than by Cargo, Node, or another ecosystem name.
- Add explicit promotion of validated lane-private output into immutable shared or immutable-seeded layers. Promotion is atomic, provenance-preserving, and never exposes a live mutable directory to another lane.
- Narrow reusable component keys to declared input closures and certified compatibility dimensions, while retaining exact-source-root matching as the safe fallback.
- Add dependency-graph parallelism, per-key singleflight, lazy layer materialization, bounded hot-set prefetch, tiered cache quotas, and deterministic cache/rebuild explanations for large repositories.
- Add reproducible scale and isolation qualification covering independent lane writes, cache hits, child inheritance, interrupted publication, garbage collection, and 10k/100k/1M-path repositories.
- Keep the database at schema v1 with no migration path: update only the fresh-schema creator and validator, and fail closed on incompatible pre-change state with backup plus `trail init --force` guidance.

## Capabilities

### New Capabilities

- `layered-lane-defaults`: Default backend selection, lazy managed preparation, parent-generation inheritance, and safe failure behavior for agent lanes.
- `environment-sync-ux`: The hard-cutover `env sync` command hierarchy, implicit lane resolution, convergence semantics, and structured reporting.
- `reusable-artifact-layers`: Framework-neutral output policies, component identity, private-output promotion, immutable publication, inheritance, and isolation.
- `large-repository-cache-scale`: Parallel and singleflight preparation, lazy materialization, cache quotas and collection, observability, and scale qualification.

### Modified Capabilities

None. The repository has no published OpenSpec capability baseline; these contracts formalize and extend behavior currently described by code and design documents.

## Impact

- Lane lifecycle and workspace views: `trail/src/db/lane/lifecycle.rs`, `workspace_view.rs`, `managed_execution.rs`, workdir backends, lane reports, and spawn/exec tests.
- Environment model: `workspace_environment.rs`, built-in adapters, command recipes, layer/cache storage, inheritance, runtime bindings, and the adapter SDK.
- Storage: the single schema-v1 creator and validator, environment generation/layer records, promotion journals, cache leases, recovery, backup/restore, doctor, fsck, and cache GC.
- Public surfaces: CLI arguments/help/exit behavior, Rust reports, JSON/NDJSON, HTTP/OpenAPI, MCP tools, documentation, examples, and changelog.
- Compatibility: the CLI rename and stored-state hard cutover are intentional breaking changes. Git refs and recorded source history remain explicit and unchanged.
- Operations: qualification must place Cargo artifacts beneath `/Volumes/Workspace/crabbuild-target` and real-repository inputs beneath `/Volumes/Workspace/Github`, with one Cargo target directory per checkout or concurrent worktree.

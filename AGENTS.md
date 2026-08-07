# AGENTS.md

This file is the repository-level operating guide for AI coding agents working on Trail. It applies to the entire tree unless a more specific `AGENTS.md` exists below the directory being changed.

## Start here

Before editing:

1. Run `git status --short` and preserve all user changes. Keep unrelated edits and generated files out of the patch.
2. Read the affected crate's `Cargo.toml`, its `src/lib.rs` or binary entry point, and the nearest unit and integration tests.
3. Find the closest existing implementation and keep the change at that ownership boundary.
4. Read [README.md](README.md) for the product model and [docs/README.md](docs/README.md) for documentation routing.
5. Read the relevant design and reference documents listed below before changing a public contract, storage behavior, lane lifecycle, or integration.
6. Check the matching GitHub Actions workflow before choosing final verification. Continuous integration (CI) is the authority for platform-specific gates.

The root Cargo workspace contains `trail` and `trail-environment-adapter-sdk`. The Agent Client Protocol (ACP) reference peer under `tools/acp-v1-reference-peer` is an independent workspace used by an interoperability script. Supporting directories such as `docs/`, `scripts/`, and `tools/` remain in scope for matching work. Other embedded product source trees are not Trail dependencies and are out of scope unless the task names them.

## Disk and external-checkout policy

The main disk is limited to 100 GB. Rust build artifacts and repositories used for real-repository Trail qualification belong on the mounted workspace volume, not in this checkout or on the main disk.

- Before any Cargo command that can compile (`build`, `check`, `test`, `clippy`, `bench`, `doc`, `install`, `package`, or a Make target that invokes one), set `CARGO_TARGET_DIR` beneath `/Volumes/Workspace/crabbuild-target`.
- Give every repository checkout and worktree its own target directory. Never share one Cargo target directory between repositories or concurrent worktrees because feature sets, build scripts, and locks can collide.
- Use a stable, descriptive directory such as `/Volumes/Workspace/crabbuild-target/trail-main` for the primary checkout and `/Volumes/Workspace/crabbuild-target/trail-<worktree_name>` for another Trail worktree. Include both the repository and checkout or worktree name for another project.
- Set the variable on every shell or tool invocation. Do not assume an export from an earlier command persists. For example:

  ```bash
  CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/trail-main \
    cargo test -p trail --locked
  ```

- Verify that `/Volumes/Workspace` is mounted and the selected directory is writable before a long build. Create only the specific per-checkout directory needed. If the volume is unavailable, stop and report it instead of falling back to a local `target/` directory.
- Do not delete, clean, or reuse another repository's target directory. Run `cargo clean` only with the intended `CARGO_TARGET_DIR` set explicitly and only when the task requires invalidating or reclaiming those artifacts.
- Look for repositories used by real-repository lane, changed-path ledger, and scale qualification under `/Volumes/Workspace/Github` first. When a task requires a missing public repository, clone it under `/Volumes/Workspace/Github/<owner>/<repository>`. Do not clone qualification repositories into the Trail tree, `/tmp`, or the main-disk GitHub directory.
- Treat external qualification repositories as read-only inputs. Do not modify, update, reset, or clean an existing checkout unless the task explicitly requires it. Keep generated `.trail/` state and qualification artifacts outside tracked source, or remove only artifacts created by the current task.

Some Makefile targets consume binaries through a literal local `target/` path after Cargo finishes. Prefer direct Cargo commands with `CARGO_TARGET_DIR` for normal verification. Before using packaging, installation, or release targets, inspect the target and ensure its artifact lookup also points at the selected external directory. Do not allow it to trigger a second local build silently.

## Product model

Trail is a native, local-first operation database for code and text worktrees. Git remains the shared source-control and publication layer. Trail records the high-frequency local work between Git commits: operations, roots, line identity, branches, lanes, sessions, turns, traces, approvals, readiness gates, handoffs, and merges.

The released package and binary are both named `trail`. The binary entry point is `trail/src/main.rs`, and reusable behavior is exposed through `trail/src/lib.rs` and the `Trail` type.

Preserve these product boundaries:

- Trail must remain usable through the command-line interface (CLI) without a daemon. Hypertext Transfer Protocol (HTTP) and Model Context Protocol (MCP) servers are opt-in integration surfaces.
- `.trail/` is private workspace state. `.trailignore` lives at the workspace root. Never record, materialize over, export, or expose internal state accidentally.
- Git interoperation must stay explicit. A Trail lane is backed by `refs/lanes/<name>` and is not an implicit Git branch or worktree.
- Durable objects and refs are the source of truth. Derived indexes may be rebuilt; they must not become the only copy of history.
- Stable file and line identity, object content addressing, operation ancestry, and provenance must survive edits, record, checkout, lane patching, merge, backup, restore, and index rebuild.
- A lane's branch record, ref, head operation, and root must agree. Prefer an explicit conflict, dirty state, stale state, or ambiguity error over invented resolution.
- CLI JSON, HTTP, MCP, and Rust callers should share typed report models. Do not duplicate domain behavior or create transport-specific meanings.
- Human terminal output is a presentation surface, not a parsing contract. Automation uses `--format json` or `--format ndjson`; deterministic log output uses `--format plain`.
- Local-first does not mean trust-free. Workspace files, patches, archives, HTTP requests, agent events, adapter packages, subprocess output, and external tool metadata are untrusted inputs.

## Ownership map

Route changes to the lowest layer that owns the behavior:

| Area | Owner |
| --- | --- |
| Public library exports and `Trail` construction | `trail/src/lib.rs`, `trail/src/db/mod.rs` |
| Stable IDs, domain objects, lane types, inspection types, and reports | `trail/src/ids.rs`, `trail/src/model/` |
| Error codes, exit categories, and public error semantics | `trail/src/error.rs` |
| Workspace initialization, status, backup, restore, doctor, fsck, and audit | `trail/src/db/core/` |
| Recording, branches, checkout, history, provenance, and map inspection | `trail/src/db/record/` |
| SQLite schema, objects, refs, prolly maps, indexes, manifests, and validation | `trail/src/db/storage/` |
| Lane identity, workdirs, patches, sessions, turns, traces, gates, readiness, rewind, environments, and retirement | `trail/src/db/lane/` |
| Branch/lane merge, conflicts, queueing, and Git export | `trail/src/db/merge/` |
| Native changed-path ledger, observers, projection, recovery, and policy | `trail/src/db/change_ledger/` |
| Shared config, path, ignore, guardrail, redaction, process, and materialization helpers | `trail/src/db/util/` |
| CLI arguments, dispatch, daemon routing, rendering, streams, and exit behavior | `trail/src/cli/` |
| HTTP transport, routes, request types, and OpenAPI descriptions | `trail/src/server/` |
| MCP protocol, capabilities, tools, resources, prompts, and responses | `trail/src/mcp/` |
| Agent Client Protocol (ACP) relay, schema, transport, capture, setup, and provider registry | `trail/src/acp/` |
| Native agent-hook discovery, parsing, and installation | `trail/src/agent_hooks/` |
| Public environment-adapter wire types and authoring helpers | `trail-environment-adapter-sdk/` |
| ACP interoperability peer | `tools/acp-v1-reference-peer/` |
| Qualification, performance, packaging, and platform scripts | `scripts/`, `.github/workflows/` |
| Product, task, design, integration, and reference documentation | `README.md`, `docs/`, `ROADMAP.md`, `plans/`, `openspec/` |

Keep `trail/src/main.rs` thin. Keep CLI, HTTP, MCP, ACP, and hook adapters focused on parsing, transport, and presentation. Put reusable mutations, validation, and report construction in the library layer.

When one feature crosses interfaces, implement the domain operation once and adapt it. Keep report fields and semantics aligned across Rust, CLI JSON, HTTP/OpenAPI, and MCP.

## Read before changing a subsystem

Use these documents as the first routing references:

| Change | Read first |
| --- | --- |
| Core architecture or a new interface | `docs/design/architecture.md`, `docs/design/data-model.md` |
| Objects, refs, SQLite, prolly maps, schema, indexes, garbage collection, backup, or restore | `docs/design/storage-and-indexing.md`, `docs/concepts/storage-indexes-and-backups.md` |
| Paths, ignore policy, patches, guardrails, redaction, daemon auth, or secrets | `docs/design/guardrails-security-and-redaction.md`, `docs/reference/patch-format.md` |
| Lanes, coordination, readiness, or merges | `docs/design/lane-coordination.md`, `docs/concepts/readiness-gates-and-merge-safety.md`, `docs/lanes/` |
| Materialized or layered lane workspaces | `docs/design/layered-lane-workspaces.md`, `docs/lanes/spawn-and-materialize-workdirs.md` |
| Lane environments or adapters | `docs/design/universal-lane-environments.md`, `docs/design/environment-adapter-contract.md`, `trail-environment-adapter-sdk/README.md` |
| ACP relay or agent capture | `docs/design/acp-relay.md`, `docs/design/native-agent-hooks-and-acp.md`, `docs/acp-v1-compatibility.md` |
| CLI arguments, terminal rendering, or exit behavior | `docs/reference/cli/`, `docs/CLI_TERMINAL_OUTPUT.md` |
| HTTP or OpenAPI | `docs/reference/http-api.md`, `docs/integrations/openapi.md` |
| MCP | `docs/reference/mcp-tools.md`, `docs/integrations/mcp.md` |
| Performance or scale claims | `docs/guides/performance-and-scale-benchmarks.md`, `.github/workflows/scale.yml` |
| Release metadata or packaging | `RELEASING.md`, `.github/workflows/release*.yml` |

Documents under `docs/superpowers/`, `plans/`, and `openspec/changes/` may describe proposed or partially implemented behavior. They are not proof that a feature ships. Verify current behavior in code, tests, reference docs, and the changelog before making a compatibility claim.

## Storage and history invariants

Trail stores durable state across filesystem sidecars, SQLite, content-addressed Concise Binary Object Representation (CBOR) objects, and prolly maps. Changes in this area must preserve a coherent state across all of them.

- Never mutate a published content-addressed object in place. Create a new object and update reachable refs through the existing transaction/publication path.
- Preserve operation parents, before/after roots, path maps, file IDs, line IDs, object kinds, codecs, hashes, and object versions.
- Treat refs and object history as durable truth. Keep derived operation, history, message, trace, and worktree indexes rebuildable.
- Acquire the existing workspace write lock for mutations. Do not confuse the write lock with advisory lane/path leases; they protect different invariants.
- Use existing SQLite transactions, staging directories, journal/recovery records, and atomic rename helpers. A failure must not expose a partial backup, restore, root, ref, generation, workdir, or adapter artifact as successful.
- Validate a staged artifact completely before publication. On failure, retain the previous active state and clean up only artifacts owned by the failed attempt.
- Preserve deterministic encoding, discovery, ordering, topological selection, pagination, and report output for equivalent inputs. Sort at contract boundaries and avoid relying on filesystem or hash-map iteration order.
- Keep all scans, traversals, payloads, archive extraction, frames, subprocess output, and caches bounded. Reaching a limit is an explicit error or truncation state, never an empty successful result.

Schema changes require special care:

1. Update the single fresh-schema creator and validator in `trail/src/db/storage/schema/`.
2. Keep `PRAGMA user_version=1` and `schema_meta` coherent.
3. Refuse every version other than schema v1 explicitly; Trail has no database migration path.
4. Make fresh creation atomic under its savepoint and workspace lock.
5. Add schema-v1 reopen, corruption, rollback, and fault-injection coverage as applicable.
6. Update reports, OpenAPI/MCP schemas, reference docs, and compatibility notes when stored meaning becomes public.

Never rewrite a user's `.trail/` state manually in a test. Existing non-v1 workspaces must fail closed with backup and `trail init --force` guidance.

## Paths, security, and process boundaries

- Use the existing path normalization and containment helpers. Reject absolute and parent-traversing paths, paths outside Unicode Normalization Form C (NFC), invisible controls, separator lookalikes, reserved or internal paths, symlink escapes, and case collisions according to the current platform policy.
- Preserve `.trail`, `.git`, and hardcoded private-path protections. Workspace ignore rules and an explicit `allow_ignored` approval must remain distinct from hard blocks.
- Apply secret rejection or redaction before durable storage. Do not log raw tokens, authorization headers, environment values, patch secrets, or adapter secret bytes.
- Keep daemon authentication enabled by default. Changes to loopback enforcement, origin/host validation, token-file handling, request framing, idempotency, or header parsing need adversarial tests.
- Build subprocesses with `Command` and separate arguments, never shell interpolation. Bound runtime and captured output, clear or control inherited environment, and make network/process/filesystem capabilities explicit.
- Treat environment adapters as planners, not executors. Trail owns source pinning, tool resolution, sandboxing, execution, validation, publication, bindings, recovery, and cleanup.
- Preserve adapter package digests, signatures, trust/revocation behavior, protocol negotiation, permission bounds, deterministic plans, and atomic environment generation activation.
- ACP forwarding stays authoritative. Capture failures, queue pressure, or projection failures must not rewrite valid frames. Preserve correlation, ordering, cancellation, backpressure, unknown valid extensions, and external-root path semantics.
- Keep unsafe Rust localized to operating-system and foreign-function interface boundaries. Document the safety invariant near each unsafe block and cover it with platform-specific tests.

Tests must use fixtures, temporary workspaces, local listeners, fake providers, or controlled subprocesses. Never use real credentials, publish artifacts, contact paid services, or mutate an existing external repository during a test.

## Rust conventions

- Use Rust Edition 2024 and preserve the workspace minimum supported Rust version of 1.89 unless the task explicitly changes it. CI uses stable Rust.
- Use `--locked` for normal agent build and test commands. Change `Cargo.lock` only when dependency resolution intentionally changes.
- Reuse dependencies from `[workspace.dependencies]`. Add crate dependencies with `.workspace = true` when the dependency is shared.
- Return `crate::error::Result` and typed `Error` variants with actionable context for expected failures. Preserve established `Error::code()` and exit-code behavior for public commands.
- Avoid new `unwrap`, `expect`, and `panic!` calls in production paths that consume user or external input. Test setup may use them when failure cannot be mistaken for product behavior.
- Prefer checked conversions, saturating arithmetic where loss is acceptable, and explicit overflow errors where it is not.
- Prefer `BTreeMap`/`BTreeSet` or explicit sorting when order is observable. Hash collections are acceptable for internal lookup when iteration order cannot escape.
- Keep serialized enums, field names, defaults, IDs, protocol constants, and version tags explicit. A source-compatible Rust change can still be a wire-format breaking change.
- Preserve platform portability. Guard Linux, macOS, and Windows code with the narrowest `cfg`, avoid UTF-8 path assumptions, and test native filesystem behavior on the owning operating system.
- Follow `rustfmt.toml`. Do not make broad formatting or mechanical cleanup changes alongside behavioral work.

## Public contracts and compatibility

Treat these surfaces as compatibility-sensitive:

- CLI command names, flags, defaults, help, stdout/stderr routing, output modes, exit codes, and side effects
- Environment variables, configuration keys, `.trailignore`, `HEAD`, refs, workdir manifests, and daemon discovery/token files
- JSON/NDJSON reports, serialized model fields, patch JSON, OpenAPI routes and schemas, MCP tools/resources/prompts, and HTTP status/error bodies
- ACP v1 frames, pinned fixtures, intentional transformations, setup files, provider registry entries, and capture semantics
- Environment-adapter protocols, package manifests, signatures, permission defaults, plan identity, and SDK construction APIs
- SQLite schema versions, object kinds/versions/codecs, content hashes, stable IDs, refs, backup archives, journals, and generation records

For a user-visible or machine-contract change:

1. Add a regression test at the lowest useful layer.
2. Add an interface-level test for each changed public surface.
3. Update the matching file under `docs/reference/`, `docs/integrations/`, or `docs/lanes/`.
4. Update `README.md` when the product model or primary workflow changes.
5. Add a `CHANGELOG.md` entry for release-visible behavior and state any required migration explicitly.
6. Update security design docs when trust, credentials, paths, subprocesses, network access, redaction, or disclosure changes.

Do not claim ACP conformance, platform isolation, durability, atomicity, or performance from design intent alone. Run the documented evidence gate and distinguish verified, failed, and skipped evidence.

## Tests and fixtures

Behavior changes require regression coverage at the lowest useful layer. User-visible changes also require a CLI, HTTP, MCP, SDK, or protocol contract test as appropriate.

- Unit tests live beside the implementation in `#[cfg(test)]` modules.
- Cross-module and public contract tests live under `trail/tests/`.
- Shared ACP and agent-hook fixtures live under `trail/tests/fixtures/`. Keep them pinned, reviewable, deterministic, and free of credentials.
- CLI tests should execute the built binary and assert exit status, stdout, stderr, filesystem side effects, and structured output where relevant.
- Storage tests should cover round trip and reopen. Add corruption, interruption, concurrent opener/writer, rollback, rebuild, and recovery cases when those risks apply.
- Lane tests should cover virtual and materialized lanes, dirty workdirs, ambiguity, conflict, readiness blockers, leases/approvals, cleanup, and retry behavior as applicable.
- Concurrency-sensitive library changes should pass with both `RUST_TEST_THREADS=1` and the default scheduler. Use the scoped test-state helpers instead of leaking global environment or hooks across tests.
- Native changed-path ledger and layered-workspace behavior must be tested on the owning operating system. Do not treat a skipped Linux, macOS, Windows, Filesystem in Userspace (FUSE), Network File System (NFS), or Dokany test as passing evidence.
- Performance changes need a correctness assertion plus the documented benchmark/threshold gate. Do not update thresholds merely to hide a regression.

Run the narrowest useful checks while iterating:

```bash
cargo test -p trail --lib test_name --locked
cargo test -p trail --test integration_test --locked
cargo test -p trail-environment-adapter-sdk --locked
```

Before finishing a Rust change, run the applicable local baseline:

```bash
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

If native dependencies make `--all-features` unavailable on the current host, run Clippy without unsupported features and report the omitted gate.

Run additional gates for the changed surface:

```bash
# CLI behavior and terminal output
cargo test -p trail --test e2e --locked
cargo test -p trail --test terminal_output_guard --locked

# ACP v1 compatibility and official-type interoperability
cargo test -p trail --test acp_conformance --locked
cargo test -p trail --test acp_faults --locked
cargo test -p trail --test acp_interop --locked
scripts/test-acp-v1-reference-interop.sh

# Current schema hard-cutover boundary
cargo test -p trail --test schema_v1_hard_cutover --locked

# Changed-path ledger on its native host
cargo test -p trail --test changed_path_ledger_linux --locked    # Linux
cargo test -p trail --test changed_path_ledger_macos --locked    # macOS

# Lane environments and execution lifecycle
cargo test -p trail --test lane_environment_inheritance --locked
cargo test -p trail --test lane_initialization --locked
cargo test -p trail --test lane_initialization_faults --locked
cargo test -p trail --test managed_execution --locked
cargo test -p trail --test lane_retirement --locked
```

Consult `.github/workflows/ci.yml`, `.github/workflows/layered-workspaces.yml`, `.github/workflows/changed-path-ledger-native.yml`, and `.github/workflows/scale.yml` for platform prerequisites and qualification scripts. Prefer direct Cargo commands that match CI. Inspect a Make target before using it, especially for packaging or cleanup, because some targets assume a literal local `target/` path.

## Documentation and planning

Keep documentation organized by reader need:

- `docs/getting-started/`: first successful workflows
- `docs/guides/`: task-oriented instructions
- `docs/concepts/`: mental models and explanations
- `docs/lanes/`: lane-specific workflows and semantics
- `docs/integrations/`: interface and ecosystem integration guidance
- `docs/reference/`: exact public contracts
- `docs/design/`: current internal design, plus explicitly labeled partial or planned designs
- `docs/superpowers/`, `plans/`, and `openspec/changes/`: planning artifacts, not shipped evidence

Use current code, tests, exported types, and interface definitions as the factual source. Keep examples executable, commands current, and statements about partial implementation explicit. When code and docs disagree, determine whether the task changes behavior or corrects documentation; do not silently choose one.

## Generated files, artifacts, and external state

- Do not hand-edit `target/`, `dist/`, release archives, checksums, benchmark output, temporary workdirs, or local `.trail/` state.
- Do not commit credentials, daemon tokens, provider state, private repository content, machine-specific paths, or generated lane materializations.
- Treat ACP v1 fixture updates as protocol changes. Update them only from the pinned source and run the drift and interoperability gates.
- Treat release workflows and `cargo-dist` configuration as generated or release-sensitive surfaces. Follow `RELEASING.md`; do not hand-create release artifacts as part of an unrelated change.
- Do not modify an existing external checkout used for scale or real-repository qualification. Use a disposable copy and keep generated Trail state outside tracked source or remove only state created by the current task.
- Do not run destructive Make targets such as `clean-all`, dependency upgrades, package publication, release tagging, or installer changes unless the task explicitly requires them.

## Completion checklist

Before handing off:

- Inspect `git diff` and `git status --short`; confirm only intended files changed.
- Confirm the change lives at the correct ownership boundary and shared behavior is not duplicated across interfaces.
- Review failure paths, rollback, recovery, cleanup, limits, determinism, and cross-platform behavior.
- Confirm public report shapes, error codes, storage versions, and documentation stay aligned.
- Add or update the required tests and fixtures.
- Run targeted checks plus the applicable baseline and platform gates.
- Report the behavior change, compatibility effect, exact verification performed, and every relevant check not run with its reason.

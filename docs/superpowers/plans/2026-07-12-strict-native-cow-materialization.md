# Strict Native-COW Materialization Implementation Plan

> **For agentic workers:** Execute inline. The user explicitly waived test-first TDD;
> add focused regression tests with each implementation slice and run them as
> verification gates.

**Goal:** Make `native-cow` strictly clone every file, add truthful
`portable-copy`, and make materialized-only `auto` restart portably when native COW
is unavailable.

**Architecture:** A materialization coordinator owns requested/resolved modes,
staging, strict fallback, publication, and reporting. A typed native-clone primitive
separates capability failures from operational errors. Existing mounted workdir modes
remain explicit and independent.

**Tech Stack:** Rust, Serde, Clap, rusqlite lane metadata, rustix
`fclonefileat`/`ioctl_ficlone`, Rayon, SHA-256 manifests, platform filesystem tests.

## Global Constraints

- Explicit `native-cow` must never byte-copy.
- `portable-copy` may clone or copy per file and must report `clone`, `mixed`, or
  `copy` from actual outcomes.
- `auto` may restart portably only for clone unsupported, cross-device, or no complete
  validated native source.
- Permission, storage, corruption, invalid-path, and I/O errors are hard failures.
- `auto` never selects FUSE, NFS, or Dokan.
- Preserve Trail's immutable-root validation, clean-workdir manifests, dirty-workdir
  rescue/refusal, and mounted-view behavior.
- Preserve unrelated untracked workspace content.
- Tests are written after each production slice by explicit user instruction.

---

### Task 1: Public modes and truthful report model

**Files:**

- Modify: `trail/src/model/reports/lane.rs`
- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/cli/command/lane_args.rs`
- Modify: `trail/src/cli/command/agent_args.rs`
- Modify: `trail/src/server/openapi/schemas/lane.rs`
- Modify: `trail/src/mcp/tools/lane.rs`
- Modify: parser/schema tests in the same modules and `trail/tests/e2e.rs`

**Interfaces:**

```rust
pub enum LaneWorkdirMode {
    Virtual,
    Sparse,
    NativeCow,
    PortableCopy,
    FuseCow,
    NfsCow,
    DokanCow,
}

pub enum WorkdirBackend {
    Clone,
    Mixed,
    Copy,
    Fuse,
    Nfs,
    Dokan,
    Virtual,
}

pub struct MaterializationReport {
    pub cloned_files: u64,
    pub cloned_bytes: u64,
    pub copied_files: u64,
    pub copied_bytes: u64,
    pub fallback_reason: Option<MaterializationFallbackReason>,
}
```

- [ ] Add `portable-copy` parsing/serialization and reject unknown aliases.
- [ ] Add requested mode, resolved mode, actual backend, and optional materialization
      details to lane spawn/workdir/sync reports.
- [ ] Keep legacy `cow_backend` readable through Serde defaults/aliases, but stop
      deriving strict evidence from the requested mode.
- [ ] Change `auto` resolution to `NativeCow` as a request policy marker handled by
      the coordinator, never a mounted backend; add an internal requested-mode value
      where persistence needs to preserve `auto`.
- [ ] Update CLI, MCP, HTTP/OpenAPI enums and focused parser/schema tests.
- [ ] Run focused model, CLI, MCP, and OpenAPI tests.
- [ ] Commit the slice.

### Task 2: Typed native clone primitive

**Files:**

- Modify: `trail/src/db/util/fs_cow.rs`
- Modify: `trail/src/db/util/mod.rs` if exports change
- Test: unit tests in `trail/src/db/util/fs_cow.rs`

**Interfaces:**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeCloneUnavailable {
    Unsupported,
    CrossDevice,
}

pub(crate) enum NativeCloneOutcome {
    Cloned,
    Unavailable(NativeCloneUnavailable),
}

pub(crate) fn clone_file_native(
    source: &Path,
    destination: &Path,
) -> Result<NativeCloneOutcome>;
```

- [ ] Replace the boolean primitive with typed clone outcomes.
- [ ] Map APFS and Linux unsupported/cross-device errno values conservatively.
- [ ] Remove `EPERM` and `EACCES` from availability classification and include Linux
      `ENOTTY` where exposed by rustix.
- [ ] Preserve create-new destination semantics and cleanup after failed ioctls.
- [ ] Keep mounted-view projection explicitly portable by translating unavailable
      outcomes into byte copies only in `clone_or_copy_projected_file`.
- [ ] Add classifier, cleanup, clone/copy, permission, and cross-device tests.
- [ ] Run fs-cow unit tests and commit the slice.

### Task 3: Materialization coordinator and accounting

**Files:**

- Create: `trail/src/db/lane/workdir/materialize.rs`
- Modify: `trail/src/db/lane/workdir.rs`
- Modify: `trail/src/db/util/fs_cow.rs`
- Modify: `trail/src/db/lane/lifecycle.rs`
- Test: unit tests in the new module and lifecycle module

**Interfaces:**

```rust
pub(crate) enum MaterializationPolicy {
    StrictNative,
    Portable,
    Auto,
}

pub(crate) struct MaterializationOutcome {
    pub resolved_mode: LaneWorkdirMode,
    pub backend: WorkdirBackend,
    pub report: MaterializationReport,
    pub stamps: BTreeMap<String, WorkdirFileStamp>,
}

impl Trail {
    pub(crate) fn materialize_lane_root_staged(
        &self,
        root_id: &ObjectId,
        destination: &Path,
        custom_workdir: bool,
        policy: MaterializationPolicy,
    ) -> Result<MaterializationOutcome>;
}
```

- [ ] Resolve a complete validated workspace source from the existing root/stamp
      checks; return typed native-source-unavailable when incomplete.
- [ ] Build strict attempts only in unique sibling stages.
- [ ] Probe source/destination device identity and native clone support, including an
      empty-root probe.
- [ ] Clone every strict file, clear untracked xattrs, apply executable mode, hash the
      destination against `FileEntry`, and produce stamps/counters.
- [ ] Abort strict mode on the first unavailable or hard failure without publishing.
- [ ] For `auto`, retire the strict stage and start portable materialization in a new
      stage only for the three eligible availability reasons.
- [ ] In portable mode, try clone per file and byte-copy only unavailable files;
      aggregate deterministic clone/copy byte and file counts.
- [ ] Verify the clean-workdir manifest before publication.
- [ ] Use bounded Rayon work already present in Trail; stop scheduling new work after
      a recorded failure where practical.
- [ ] Add strict success/failure, mixed/copy, fallback, no-source, empty-root,
      hardlink-independence, source-mutation, and counter tests.
- [ ] Run coordinator tests and commit the slice.

### Task 4: Lifecycle, staging publication, and recovery ownership

**Files:**

- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/db/lane/workdir/sync.rs`
- Modify: `trail/src/db/lane/patching.rs`
- Modify: `trail/src/db/lane/control/turn_setup.rs`
- Modify: lane metadata/recovery helpers under `trail/src/db/lane/workdir/`
- Test: lifecycle/sync tests and `trail/tests/e2e.rs`

- [ ] Route spawn, lazy ensure, full sync, rewind refresh, patch refresh, and the
      large-root path through an explicit materialization policy.
- [ ] Persist requested mode separately from resolved mode/backend and refresh actual
      reporting after every full rematerialization.
- [ ] Publish initial workdirs with an atomic no-overwrite primitive and make a losing
      concurrent creator clean only its own stage.
- [ ] Preserve registered backup/restore behavior for full replacement.
- [ ] Add an operation-owned stage record with preparing/materializing/verified/
      published/failed states using existing Trail database transaction patterns.
- [ ] Reconcile interrupted registered stages/backups without scanning or deleting
      unregistered similarly named paths.
- [ ] Ensure legacy best-effort native workdirs have no new verified backend until a
      full rematerialization succeeds.
- [ ] Add crash-point, destination-race, cleanup-ownership, backup-restore,
      large-root, dirty-workdir, and legacy-metadata tests.
- [ ] Run lifecycle and end-to-end tests and commit the slice.

### Task 5: User-facing contracts and documentation

**Files:**

- Modify: `trail/src/cli/command/render/lane/identity/basic.rs`
- Modify: `trail/src/cli/command/render/lane/work.rs`
- Modify: relevant HTTP/MCP response schemas and tests
- Modify: `docs/lanes/spawn-and-materialize-workdirs.md`
- Modify: `docs/reference/cli/lanes.md`
- Modify: `docs/reference/data-types.md`
- Modify: additional current docs found by repository scan

- [ ] Render requested mode, resolved mode, actual backend, counts, and bounded
      fallback reason without raw persisted OS messages.
- [ ] Document strict native semantics, portable behavior, materialized-only `auto`,
      platform support, and explicit mounted modes.
- [ ] Update examples and API schemas from `cow_backend` to `workdir_backend`.
- [ ] Add CLI JSON, HTTP, MCP, and legacy decode regression coverage.
- [ ] Scan current sources for stale claims that `native-cow` may copy silently.
- [ ] Run focused contract tests and commit the slice.

### Task 6: Complete verification and review

- [ ] Run `cargo fmt --all -- --check` from `trail/`.
- [ ] Run focused fs-cow, materialization, lifecycle, CLI, MCP, OpenAPI, and e2e tests.
- [ ] Run `cargo clippy -p trail --all-targets -- -D warnings`.
- [ ] Run `cargo test -p trail`.
- [ ] Run native APFS integration tests on the current macOS/APFS host.
- [ ] Record Btrfs and XFS-reflink suites as external platform gates when those
      filesystems are not present locally; do not claim they ran locally.
- [ ] Inspect `git diff --check`, `git status --short`, and the complete diff for
      unrelated changes.
- [ ] Perform a requirements-to-diff review against the design spec and repair every
      critical or important finding.
- [ ] Commit the verified implementation on `main` as explicitly requested.

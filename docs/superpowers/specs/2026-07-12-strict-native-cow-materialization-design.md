# Strict Native-COW Materialization Design

## Objective

Make Trail's materialized workdir names describe guarantees rather than aspirations.
`native-cow` becomes strict: every regular file is created by a successful
filesystem-native clone operation and Trail fails instead of copying bytes. A new
`portable-copy` mode provides reliable materialization with opportunistic clones and
byte-copy fallback. `auto` tries strict native COW in an isolated stage and, only when
native materialization is unavailable, restarts from scratch as `portable-copy`.

The design borrows Rift's useful platform primitives—APFS `fclonefileat` and Linux
`FICLONE`—while retaining Trail's immutable-root validation, staged publication, and
workdir lifecycle. Trail does not adopt Rift's whole-tree APFS cloning, Btrfs
subvolume snapshots, two-rename publication, or unowned cleanup behavior.

## Public contract

The public lane workdir modes are:

```text
auto
virtual
sparse
native-cow
portable-copy
fuse-cow
nfs-cow
dokan-cow
```

Their contracts are:

| Requested mode | Resolved mode | Guarantee |
| --- | --- | --- |
| `native-cow` | `native-cow` | Every regular file was created by a successful native clone; otherwise fail. |
| `portable-copy` | `portable-copy` | Materialize every file, cloning when possible and copying bytes otherwise. |
| `auto` | `native-cow` or `portable-copy` | Try strict native COW in staging; on an eligible availability failure, discard the stage and restart portably. |
| `sparse` | `sparse` | Preserve the explicit partial-materialization behavior and report actual per-file clone/copy results. |
| `fuse-cow` | `fuse-cow` | Explicit FUSE-mounted COW view. |
| `nfs-cow` | `nfs-cow` | Explicit loopback-NFS COW view. |
| `dokan-cow` | `dokan-cow` | Explicit Dokan-mounted COW view. |
| `virtual` | `virtual` | No materialized workdir. |

`auto` is materialized-only. It never selects FUSE, NFS, or Dokan. Those modes remain
explicit because mounting has materially different lifecycle, privilege, and failure
semantics. New materialized lanes default to `auto`.

`native-cow` initially supports APFS on macOS and reflink-capable Linux filesystems.
Other hosts and filesystems report native COW as unavailable. `portable-copy` remains
subject to ordinary operational failures such as invalid paths, insufficient
permissions, exhausted storage, or I/O errors; "portable" means it has no COW
capability requirement.

## Strategy architecture

Materialized workdirs use a small strategy boundary independent of mounted views:

```text
MaterializationRequest
    requested mode + root_id + destination
                    |
                    v
MaterializationCoordinator
    source resolution + staging + fallback + publication + reporting
                    |
          +---------+----------+
          |                    |
          v                    v
StrictNativeStrategy     PortableStrategy
APFS fclonefileat        clone each file
Linux FICLONE            copy bytes on CloneUnavailable
all files or failure     aggregate clone/mixed/copy
```

The native clone primitive returns a typed result:

```rust
enum NativeCloneOutcome {
    Cloned,
    Unavailable(NativeCloneUnavailable),
}

enum NativeCloneUnavailable {
    Unsupported,
    CrossDevice,
}
```

Operational errors remain `Err(Error)` and are never collapsed into
`Unavailable`. In particular, permission failures are errors. The current classifier
must stop treating `EPERM` and `EACCES` as unsupported. Platform-specific tests define
the exact errno mapping; Linux ioctl-not-supported responses include `ENOTTY`.

The coordinator, rather than the file primitive, owns strictness and fallback. This
keeps the primitive reusable by strict native, portable, sparse, and mounted-view
projection code without letting a low-level helper silently choose product semantics.

## Native source eligibility

Strict native materialization requires a complete filesystem projection matching the
requested immutable `root_id`. Initially, candidates are:

1. the primary workspace when Trail validates every target path against the root; and
2. an existing complete, clean Trail workdir validated against the same root.

Trail may select candidates deterministically in that order. Object-store blobs,
generated temporary source trees, and partial sparse workdirs are not native sources.
If no complete candidate exists, the attempt returns `NativeSourceUnavailable`.
Explicit `native-cow` fails; `auto` may restart as portable materialization.

Each source file is opened without following symlinks and validated against its
`FileEntry`. The clone uses the open descriptor. Trail then hashes the staged clone and
checks its content and executable bit against the immutable root before publication.
This destination verification closes the validation/clone race even if a workspace
file changes concurrently.

Trail's root model contains independent regular-file paths, not inode relationships.
Consequently, each path receives its own clone operation. Accidental hardlinks in the
source are deliberately not preserved, because editing one workdir path must not
modify another. Directory structure is derived from normalized root paths. Existing
Trail behavior for executable modes and removal of untracked cloned xattrs is
preserved.

## Strict staging and portable restart

Every materialized creation or full refresh is built in a unique, Trail-owned stage
beside the final destination. Keeping stage and destination under the same parent
filesystem permits atomic publication. Before writing, Trail records an operation ID,
the intended destination, stage path, requested mode, and lifecycle state.

The strict attempt performs these steps:

1. validate the requested root and resolve a complete native source;
2. create and register an owned sibling stage;
3. compare source and destination filesystem or volume identity;
4. behavior-probe the native clone primitive inside staging, including for an empty
   root;
5. clone every target file independently, with bounded parallelism;
6. verify every staged file and the completed clean-workdir manifest against
   `root_id`; and
7. publish the verified directory without overwriting an independently created
   destination.

For non-empty roots, successful per-file clone calls are the strict sharing proof. For
empty roots, Trail combines source/destination filesystem identity with an in-stage
clone probe so explicit strict mode still detects unsupported storage.

When `auto` receives `CloneUnsupported`, `CrossDevice`, or
`NativeSourceUnavailable`, it closes the strict attempt and removes its entire stage.
It creates a new operation-owned stage and runs `portable-copy` from the beginning.
Files cloned successfully during the failed strict attempt are never reused. Failure
to cleanly retire the strict stage is an operational error rather than permission to
continue with ambiguous ownership.

Portable materialization tries the same native primitive independently for each file.
`Unavailable` causes that file to be byte-copied; operational errors abort. It may
therefore report all-clone, mixed, or all-copy. Copying preserves existing Trail
content, executable-mode, durability, and manifest behavior.

## Publication, concurrency, and recovery

Initial creation uses a no-overwrite publication primitive. Linux uses
`renameat2(RENAME_NOREPLACE)` and macOS uses the equivalent exclusive rename where
available; other platforms use the strongest no-replace primitive they expose. Trail
never implements initial publication as "check destination, then ordinary rename."
A concurrent creator may win, but the loser removes only its registered stage.

Full refresh retains Trail's clean-workdir, force/rescue, and replacement semantics.
It constructs and verifies a sibling stage before moving the old clean workdir to a
registered backup and publishing the replacement. If publication fails, Trail
restores the registered backup. Dirty workdirs are refused or rescued according to
the existing command contract before materialization begins.

The operation lifecycle is:

```text
preparing -> materializing -> verified -> published
                         \-> failed
```

Recovery reconciles operation records rather than scanning and deleting paths by name:

- `preparing` or `materializing`: remove the registered incomplete stage;
- `verified`: publish if the destination precondition still holds, otherwise retire
  the stage and report the collision;
- replacement interrupted after backup: restore the registered backup when the
  destination is absent;
- `published` without the final metadata commit: validate the destination marker and
  finish the database transition; and
- nonexistent registered paths: mark the operation retired without touching other
  filesystem entries.

Unregistered similarly named directories are never removed. Custom workdir parents
receive the same ownership checks; Trail does not assume that every hidden sibling is
its property.

## Metadata and reporting

Reports separate user intent, resolved policy, and actual mechanism:

```json
{
  "requested_workdir_mode": "auto",
  "workdir_mode": "portable-copy",
  "workdir_backend": "mixed",
  "materialization": {
    "cloned_files": 842,
    "cloned_bytes": 914358272,
    "copied_files": 3,
    "copied_bytes": 16384,
    "fallback_reason": "clone-unsupported"
  }
}
```

`requested_workdir_mode` preserves the selection made by the caller.
`workdir_mode` is the resolved mode for the current complete materialization.
`workdir_backend` reports actual results and has these values:

```text
clone
mixed
copy
fuse
nfs
dokan
virtual
```

The mode/backend combinations are constrained:

- explicit `native-cow` is always `native-cow` plus `clone`;
- `auto` resolves to `native-cow` plus `clone`, or `portable-copy` plus `clone`,
  `mixed`, or `copy`;
- explicit `portable-copy` may report `clone`, `mixed`, or `copy`;
- sparse workdirs remain `sparse` and report actual clone/copy counters; and
- mounted and virtual modes do not fabricate per-file materialization counters.

The bounded fallback reasons are `clone-unsupported`, `cross-device`, and
`native-source-unavailable`. Raw OS errors remain diagnostic context and are not
persisted as schema values.

`cow_backend` is replaced in new reports by `workdir_backend`. Legacy records remain
readable, but their historical `cow_backend: clone` is not proof that every file was
cloned under the old best-effort implementation. `workdir_backend` remains absent for
such an existing materialization until a full sync rematerializes and verifies it
under the new policy. Existing requested modes are preserved; the default change
applies to new selections.

## Error behavior

Only native availability failures are eligible for `auto` restart:

| Condition | Explicit `native-cow` | `auto` |
| --- | --- | --- |
| clone API/filesystem unsupported | fail `CloneUnsupported` | restart portable |
| different source/destination filesystem | fail `CrossDevice` | restart portable |
| no complete validated source tree | fail `NativeSourceUnavailable` | restart portable |
| permission denied | hard failure | hard failure |
| out of space/quota | hard failure | hard failure |
| corrupt source or staged hash mismatch | hard failure | hard failure |
| invalid/colliding path | hard failure | hard failure |
| ordinary I/O failure | hard failure | hard failure |

An explicit strict request never copies bytes. Portable fallback is visible in both
the resolved mode and materialization report.

## Performance constraints

Clone, verification, and portable-copy work use bounded parallelism rather than one
thread per file. The existing small-tree and streaming paths must call the same
strategy coordinator, so crossing the large-root threshold cannot change semantics.
Counters are reduced deterministically after workers complete. A first failure stops
scheduling new work where practical, while already running workers finish safely
before stage cleanup.

Strict verification reads every staged byte to prove the immutable root; it does not
write file data and therefore does not break block sharing. Native COW is expected to
reduce allocation and write amplification, not eliminate correctness reads. Future
optimizations require an equally strong immutable-source proof.

Trail-owned Btrfs snapshot caches are explicitly deferred. They may later provide
another strict native source/strategy, but this change does not create or adopt
foreign subvolumes.

## Test strategy

Implementation follows red-green-refactor cycles.

### Unit and contract tests

- parse and serialize `portable-copy`, strict `native-cow`, and materialized-only
  `auto`;
- reject removed aliases while preserving the already-approved hard cutover;
- classify clone availability separately from permission and operational errors;
- verify exact requested/resolved/backend combinations and counters;
- prove legacy `cow_backend` does not become new strict evidence; and
- exercise empty roots, empty files, source hardlinks, sparse source files,
  executable modes, xattr cleanup, case collisions, and invalid paths.

### Strategy and fallback tests

- all native clones succeed and strict publication reports `clone`;
- the Nth clone is unsupported: explicit native fails without publication;
- the Nth clone is unsupported: `auto` discards the strict stage, restarts portable,
  and reports the portable result;
- a permission, storage, corruption, or I/O failure never falls back;
- absence of a complete native source is eligible only for `auto` restart;
- concurrent source mutation is caught by staged hash verification; and
- the large-root streaming route cannot silently bypass strict policy.

### Filesystem integration matrix

- macOS/APFS with `fclonefileat`;
- Linux/Btrfs with `FICLONE`;
- Linux/XFS with reflink enabled;
- Linux ext4 or XFS without reflink for strict failure and automatic portable
  restart; and
- cross-device source/destination behavior.

Successful platform tests modify source and destination independently after cloning
to confirm snapshot isolation. Scheduled privileged CI may use loopback Btrfs/XFS and
extent inspection; ordinary CI exercises real clone calls when available and the
typed fallback contract otherwise.

### Publication and recovery tests

Fault injection covers every lifecycle transition, the Nth file operation,
destination races, stage cleanup failure, replacement-backup restoration, and a
crash between filesystem publication and metadata commit. Tests assert that no
unregistered path is removed and no partial destination is published.

Every materialization entry point is covered: lane spawn, lazy ensure, full sync,
rewind, patch refresh, sparse hydration, and the large-root streaming path.

## Rollout

1. Add the strategy types, new mode/report schema, and portable behavior while leaving
   the current default unchanged.
2. Route every full and streaming materialization entry point through the coordinator.
3. Make explicit `native-cow` strict and invalidate claims from legacy best-effort
   materializations until their next verified full sync.
4. Run the APFS, Btrfs, XFS-reflink, unsupported-filesystem, cross-device,
   concurrency, and recovery gates.
5. Change new materialized selections to `auto`.
6. Keep mounted modes explicit and implement Btrfs snapshot caches separately.

## Completion criteria

The change is complete only when:

1. explicit `native-cow` cannot byte-copy through any materialization path;
2. `portable-copy` reports all-clone, mixed, and all-copy results truthfully;
3. `auto` restarts portably only for the three bounded availability conditions;
4. publication and recovery operate only on registered Trail-owned paths;
5. reports distinguish requested mode, resolved mode, and actual backend;
6. legacy best-effort materializations are not presented as verified strict clones;
7. focused unit, integration, fault-injection, formatting, lint, and complete Trail
   regression checks pass; and
8. native APFS, Btrfs, and XFS-reflink release gates pass on their supported runners.

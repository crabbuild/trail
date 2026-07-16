# COW Workdir Mode Hard-Cutover Design

## Objective

Replace ambiguous lane workdir mode names everywhere with mechanism-accurate names:

- `fuse-cow` for the FUSE implementation on Linux and macOS;
- `dokan-cow` for the Dokan implementation on Windows;
- `nfs-cow` for the loopback NFS implementation on macOS; and
- `native-cow` for ordinary materialized directories using clone/reflink COW when
  available.

This is a hard cutover. Trail will not accept, migrate, alias, emit, document, or
suggest the removed `overlay-cow` or `full-cow` modes after this change.

## Product contract

The public workdir modes are:

```text
auto
virtual
sparse
native-cow
fuse-cow
nfs-cow
dokan-cow
```

Terminal agent commands accept the materialized modes:

```text
auto
native-cow
fuse-cow
nfs-cow
dokan-cow
```

Platform selection is explicit:

| Mode | Platform | Transport |
| --- | --- | --- |
| `native-cow` | all supported platforms | ordinary directory; native clone/reflink when available, safe copy fallback otherwise |
| `fuse-cow` | Linux; macOS builds with macFUSE | FUSE mount |
| `nfs-cow` | macOS | loopback NFSv3 mount |
| `dokan-cow` | Windows | Dokan mount |

Requesting a mode on an unsupported platform fails before lane creation with a
mode-specific diagnostic. `auto` chooses `dokan-cow` on Windows, `nfs-cow` on macOS
when available, `fuse-cow` on Linux when FUSE is available, and otherwise follows the
existing native-COW/large-root policy.

## Hard-cutover rules

1. `LaneWorkdirMode::FullCow` becomes `LaneWorkdirMode::NativeCow`; `full-cow` and
   `full_cow` are rejected rather than retained as aliases.
2. `LaneWorkdirMode::OverlayCow` becomes `LaneWorkdirMode::FuseCow`.
3. Windows branches that previously overloaded `OverlayCow` become an explicit
   `LaneWorkdirMode::DokanCow`.
4. `LaneWorkdirMode::parse` accepts `native-cow`/`native_cow` and the backend-specific
   mounted modes. It accepts neither removed mode spelling.
5. CLI value parsers, HTTP/OpenAPI enums, MCP schemas, Rust reports, JSON output,
   diagnostics, suggestions, examples, and documentation emit only the new names.
6. Lane metadata written before the cutover is not migrated. Opening or operating on a
   lane whose metadata contains `overlay-cow` or `full-cow` returns an unsupported
   workdir-mode error instructing the operator to remove and recreate the lane.
7. Persistent transparent-COW state remains backend-neutral under
   `.trail/views/<view-id>`, while mount identifiers use `trail-fuse-cow-*` and Dokan
   ownership reports `dokan`. Old `.trail/overlay-cow` state is ignored and never
   adopted.
8. Rust symbols use `NativeCow`/`native_cow`, `FuseCow`/`fuse_cow`, and
   `DokanCow`/`dokan_cow` for their respective mechanisms.
   Generic overlay terminology may remain only where it describes the filesystem
   algorithm rather than the removed product mode.
9. Script filenames, environment defaults, Docker volume names, workflow filters, and
   benchmark labels use `fuse-cow` when they exercise FUSE.
10. No compatibility warnings or deprecation period are added.

## Internal architecture

The workdir mode identifies the user-visible mechanism. The mounted-view semantic core
remains shared.

```text
LaneWorkdirMode
├── NativeCow   -> materialized directory
├── FuseCow   -> FUSE transport -> ViewCore
├── NfsCow    -> NFS transport  -> ViewCore
└── DokanCow  -> Dokan transport -> ViewCore
```

Mode predicates replace the old two-mode assumptions:

```rust
pub fn is_transparent_cow(&self) -> bool {
    matches!(self, Self::FuseCow | Self::NfsCow | Self::DokanCow)
}

pub fn cow_backend(&self) -> Option<&'static str> {
    match self {
        Self::NativeCow => Some("clone"),
        Self::FuseCow => Some("fuse"),
        Self::NfsCow => Some("nfs"),
        Self::DokanCow => Some("dokan"),
        Self::Sparse => Some("clone"),
        Self::Virtual => None,
    }
}
```

Mount dispatch must match the mode directly. Dokan must not pass through a method or
module named for FUSE. Shared behavior continues through `ViewCore`, not through an
ambiguous public mode.

## Metadata and error handling

New lane metadata stores the exact selected mode in `workdir_mode` and the concrete
mechanism (`clone`, `fuse`, `nfs`, or `dokan`) in `cow_backend`.

When old metadata is encountered, parsing fails with a stable error equivalent to:

```text
unsupported lane workdir mode `overlay-cow`; this build uses the hard-cutover modes
`fuse-cow` and `dokan-cow`; remove and recreate the lane with the platform-appropriate
mode
```

The removed materialized name similarly reports that `full-cow` was renamed to
`native-cow` and instructs the operator to remove and recreate the lane.

Trail must not silently infer FUSE versus Dokan from the current host because doing so
could reinterpret copied or restored workspace metadata.

## Test strategy

The implementation follows test-driven development.

### Parser and serialization

- `native-cow` and `native_cow` parse as `NativeCow`.
- `fuse-cow` and `fuse_cow` parse as `FuseCow`.
- `dokan-cow` and `dokan_cow` parse as `DokanCow`.
- both spellings of `full-cow` and `overlay-cow` fail.
- reports serialize the exact new names and backends.

### CLI and API contracts

- lane and agent CLI parsers accept the new values and reject both removed names.
- OpenAPI and MCP schemas contain `native-cow`, `fuse-cow`, and `dokan-cow` and do not
  contain either removed name.
- JSON lane spawn reports emit the new mode.

### Backend dispatch

- automatic Linux selection returns `FuseCow` when `/dev/fuse` is available.
- automatic Windows selection returns `DokanCow`.
- FUSE mount helpers reject non-FUSE modes.
- Dokan mount helpers reject non-Dokan modes.
- environment initialization, gates, record/checkpoint, update, and agent launch mount
  the correct transport.

### Hard-cutover coverage

- lane metadata containing `overlay-cow` or `full-cow` fails with an explicit
  recreation diagnostic;
- a repository-wide source scan, excluding Git object history and generated build
  output, contains no removed public spelling or Rust identifier; and
- renamed FUSE/Dokan scripts and platform workflows run through their native release
  gates.

## Relationship to lane hardening

This naming cutover is the first bounded change in the broader lane-hardening program.
It does not make `native-cow` secondary. All four COW mechanisms remain first-class and
will be required to satisfy the same environment, process-isolation, cache-provenance,
gate, lifecycle, and reuse contracts. The explicit names make that backend conformance
matrix auditable.

## Completion criteria

The cutover is complete only when:

1. all production code, tests, schemas, scripts, current documentation, and checked-in
   skill references use the new names;
2. the old spellings and symbols for both renamed modes are rejected or absent rather
   than accepted as aliases;
3. FUSE and Dokan dispatch are distinct and platform-correct;
4. focused parser/CLI/API/backend tests pass;
5. the complete Trail regression passes; and
6. native FUSE, NFS, and Dokan release gates are green on their supported platforms.

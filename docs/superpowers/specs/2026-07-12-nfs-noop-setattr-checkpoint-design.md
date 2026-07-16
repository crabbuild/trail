# NFS No-op `SETATTR` Checkpoint Safety

## Problem

Trail's macOS NFS-COW adapter forwards only file size and mode from an NFS
`SETATTR` request. Requests that contain only unsupported metadata, such as
timestamps or ownership, therefore reach `ViewCore::setattr` with both values
absent.

`ViewCore::setattr` currently journals every call as a source metadata
mutation. Checkpoint recovery treats each journaled source path as a candidate
and scans the source upper layer for its contents. A metadata-only request does
not copy the lower file into that upper layer, so the checkpoint recorder
mistakes the absent upper file for a deletion.

## Chosen Design

Make `ViewCore::setattr` treat a call with neither size nor mode as a no-op. It
will return the file's current visible attributes without copying the file into
the upper layer and without appending a mutation-journal record.

Calls that provide size, mode, or both retain their current behavior: Trail
copies up the file when needed, applies the requested change, and records the
metadata mutation. The NFS adapter continues to ignore unsupported ownership
and timestamp fields; this change only prevents those ignored fields from
creating false source changes.

The guard belongs in `ViewCore` rather than only in the NFS adapter so every
filesystem adapter shares the invariant that an empty supported-attribute
update cannot dirty a workspace view.

## Data Flow

1. The NFS adapter receives `sattr3` and extracts optional size and mode.
2. It calls `ViewCore::setattr` with those two supported values.
3. If both values are absent, `ViewCore` returns the current attributes without
   starting a logical mutation.
4. Otherwise, the existing copy-up, mutation, synchronization, and journal
   behavior runs unchanged.
5. Checkpoint recovery therefore sees only paths with actual supported
   mutations, explicit writes, creations, renames, or whiteouts.

## Error Handling

The no-op path still resolves the inode and current visible attributes. Invalid
or stale inode errors therefore remain observable. It does not suppress errors
for real size or mode changes.

## Tests

Add a regression test using the real `ViewCore` fixture:

1. Create a lower-layer source file and look up its inode.
2. Call `setattr(inode, None, None)`.
3. Assert that the visible attributes are returned.
4. Assert that the source upper layer does not contain the file.
5. Assert that checkpoint candidates do not contain the path.

The test must fail before the implementation change by observing the spurious
checkpoint candidate. After the minimal guard is added, run the focused
workspace-view and NFS-COW tests, followed by the broader Trail test suite.

## Non-goals

- Implementing timestamp, ownership, ACL, or extended-attribute mutation.
- Changing checkpoint recovery semantics for real metadata mutations.
- Recovering already-corrupted lanes; recovery remains a separate operator
  action.

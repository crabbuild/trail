### Spec Compliance

- Issues in schema validation and mkdir/open allocation binding.
- Cannot verify native/crash/durability results from diff.

### Strengths

- Journal ordering, retry convergence, retained quiescence, and earlier protections are strong.

### Issues

#### Critical

None.

#### Important

1. New allocation/deletion tables are omitted from structural table/FK/row-level validation, and independent FKs do not enforce same scope/epoch/segment join. Malformed v18 can reach mutable open. Fix: complete column/PK/index/FK/row validation, global semantic join/state query, and byte-invariant read-only rejection tests for orphan/cross-wired rows.

2. `mkdirat` then `openat` is separable; substituted directory can be adopted as Trail-owned. Fix: use creator-bound primitive, preferably nonce-keyed no-replace file quarantine directly under retained descriptor authority, and exact hook/regression between creation/open for old path.

#### Minor

None.

### Assessment

Task quality: Needs fixes.

### Allocation approval fixes

Both Important findings are fixed.

1. Immutable v18 preflight now validates the allocation and deletion tables' exact columns, primary keys, named/partial unique indexes, and foreign keys. A single read-only semantic query rejects orphan allocations/deletions, non-bound allocations with deletion rows, bound allocations without exactly one deletion, cross-wired scope/epoch/segment/leaf/parent/inode identities, invalid allocation/deletion states, and duplicate active source ownership. New malformed orphan, cross-wired, and state-invalid fixtures prove rejection before mutable open and byte-for-byte invariance of the complete `.trail` tree.

2. Retirement now journals the exact source inode and a nonce-keyed quarantine leaf directly under the retained scope directory, then uses same-directory atomic no-replace rename. Recovery reopens through the retained descriptor, requires exact journaled source/published inode identity, authenticates the full segment bytes, synchronizes the file and parent directory, and only then publishes allocation/binding state. Existing targets are retained and audited; unsupported no-replace platforms fail closed. The retirement path performs no pathname unlink or directory removal.

Regression coverage includes the old mkdir/open substitution window, source substitution immediately before rename, target substitution immediately after rename, existing direct-target collisions, and two-segment SIGKILL/reopen recovery at the journal barrier, direct rename, exact verification, file/parent fsync, per-segment setup, transaction commit, and WAL boundaries.

Verification:

- `cargo test -p trail --lib`: 568 passed, 1 ignored.
- `cargo test -p trail --test changed_path_ledger_recovery`: 29 passed.
- `cargo test -p trail --release --test schema_v18_hard_cutover`: 9 passed.
- `cargo check -p trail --release --lib`: passed.
- `cargo test -p trail --test e2e`: 204 passed; the pre-existing newer-schema diagnostic wording assertion remains the sole failure.
- The recovery integration target itself is debug-only (`trail::test_support` is gated by `debug_assertions`), so it cannot be compiled as a release integration test; the production release library compiles cleanly and all hook-driven races run in debug.

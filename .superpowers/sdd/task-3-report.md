# Task 3 report: durable observer segment protocol

## Status

Implemented the crate-private durable changed-path observer segment protocol. The module is dormant:
it does not wire snapshot, status, diff, record, acknowledgement, baseline advancement, or any public
authority path.

Task commit: `11a2913c1524818e2026be82b971577aa6936ac0`

## Files

- `trail/src/db/change_ledger/log.rs`: façade, shared protocol types, limits, recovery errors, and
  crate-private exports.
- `trail/src/db/change_ledger/log/codec.rs`: version-1 header/record codec, checksums, hash linkage,
  bounded recovery, SQLite metadata validation, and orphan detection.
- `trail/src/db/change_ledger/log/writer.rs`: dedicated SQLite control connection, exclusive lease,
  file-only append path, group durability publication, heartbeat, rotation, and fault seam.
- `trail/src/db/change_ledger/log/tests.rs`: direct crate-private codec, recovery, lease, schema, and
  fault tests.
- `trail/src/db/change_ledger/mod.rs`: dormant module declaration and crate-private re-exports.

No dependency change was needed: `serde_cbor`, `sha2`, `rusqlite`, `hex`, and `tempfile` were already
declared. No integration test was created because the protocol deliberately has no public black-box
behavior yet; exposing it solely for `trail/tests/changed_path_ledger_log.rs` would violate the
resolved boundary.

## Exact version-1 format

All integers in framing are unsigned big-endian. CBOR values are decoded, canonically re-encoded,
and compared byte-for-byte; non-canonical encodings fail closed. Binary identities, tokens, cursors,
and hashes are CBOR byte strings on disk. SQLite identities and hashes are lowercase hexadecimal.

Segment header:

```text
magic[8] = "TRAILCPL"
format_version: u16 = 1
header_payload_length: u32 (<= 1 MiB)
header_payload: canonical CBOR array[
  scope_id: bytes(32),
  epoch: uint (> 0),
  owner_token: bytes(32),
  provider_cursor: bytes,
  previous_segment_hash: bytes(32)
]
```

Record:

```text
record_body_length: u32
sequence: u64 (> 0, strictly monotonic)
source: u8 (observer=1, intent=2, reconciliation=3, git_advisory=4)
payload: canonical CBOR array[
  normalized_path: text,
  evidence_flags: uint,
  provider_cursor: bytes
] (<= 1 MiB before CBOR allocation)
previous_record_hash: bytes(32)
checksum: bytes(32) = SHA-256(sequence | source | payload | previous_record_hash)
```

The record checksum is also the next record's linkage hash. Record linkage resets to zero in each
segment. Segment lineage uses SHA-256 over the complete durable prior segment and is present in both
the next header and SQLite metadata.

Persisted recovery limits are explicit and default to the schema values:

- total log: 268,435,456 bytes;
- segment: 16,777,216 bytes;
- unfolded tail: 65,536 records;
- record payload: fixed protocol cap of 1,048,576 bytes.

## Lease and append protocol

`SegmentWriter::acquire` opens its own SQLite connection from the supplied database path. It never
accepts or borrows Trail's primary connection. Acquisition uses an IMMEDIATE transaction, verifies
the exact scope epoch and persisted limits, rejects a live active lease, and inserts/replaces only an
expired/revoked/error owner. The 32-byte owner token is stored as lowercase hex.

The first header is created, written, file-synced, and directory-synced before segment metadata is
published. Its durable header offset is safe to publish at acquisition.

Every append batch:

1. rejects a retired in-memory writer;
2. acquires an IMMEDIATE control transaction;
3. validates `(scope_id, scope epoch, owner epoch, owner_token, active, unexpired)`;
4. encodes and bounds the complete batch, requiring exact sequence increments;
5. enforces both persisted segment and total-log byte limits;
6. writes only the segment file (no high-rate SQLite mutation);
7. commits the lease serialization transaction.

The IMMEDIATE transaction serializes owner replacement against validation plus the actual append,
so a committed replacement fences the old writer before another append.

Any append, disk-full/write, lease, flush, sync, rotation, or heartbeat error retires authority in
memory immediately. A best-effort, token-guarded owner error update cannot revoke a replacement.

## Durable flush ordering

`flush_durable` holds the dedicated control transaction across lease validation, `sync_data`, a
second validation, and the guarded metadata update:

```text
validate full lease tuple
sync record bytes
validate full lease tuple
publish segment durable_end_offset + last_sequence
commit control transaction
```

The fault test records the pre-flush durable offset, injects sync failure, and proves the SQLite
offset remains unchanged. Task 2 offset ownership is preserved: this code updates only segment
`durable_end_offset`; it does not manufacture scope offsets or alter acknowledgement/baseline logic.

## Rotation ordering

The production order is:

1. validate the lease;
2. sync the old tail;
3. hash and seal old segment metadata, including its durable end;
4. sync the segment directory;
5. create the next file;
6. write its header;
7. sync the next header;
8. sync the segment directory;
9. validate the lease again;
10. publish next metadata and previous-segment ID/hash lineage;
11. switch the in-memory writer only after publication succeeds.

Injected failures cover old sync, seal publication, first directory sync, next create, next write,
next header sync, second directory sync, and next metadata publication. Every failure retires the
writer and a subsequent append is rejected. A header created but not published is detected as an
orphan and makes recovery require reconciliation.

## Recovery behavior

Recovery first queries ordered segment metadata through a separate read connection. It bounds
metadata durable byte totals and filesystem lengths before allocating. It validates owner/scope/
epoch, format, canonical CBOR, sequence, record checksums, record linkage, segment IDs/hashes,
first/last sequence metadata, sealed hashes, open-segment placement, and lineage.

- A partial final record returns all prior checked records and `requires_reconciliation=true`.
- Partial/corrupt data before the final segment fails closed.
- Unsupported format, wrong identity/owner, non-monotonic sequence, checksum/linkage failure,
  sealed trailing bytes, metadata mismatch, or cap excess returns `RecoveryError` with
  `requires_reconciliation=true`.
- Unpublished bytes in the final open segment and orphan `.cpl` files conservatively require
  reconciliation.
- Records are never skipped after corruption.

This establishes process-crash ordering only. It does not claim power-loss authority; the global
WAL `synchronous=NORMAL` rule and later activation/reconciliation work remain authoritative.

## TDD evidence

Initial RED, with the configured compiler wrapper disabled after sccache stalled:

```text
env PATH="$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
  RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER= \
  cargo test -p trail --lib db::change_ledger::log::tests --no-run

error[E0432]: unresolved imports log::recover_segments, log::DurableCut,
log::ObserverRecord, log::PersistedLogLimits, log::RecoveredTail, log::SegmentWriter
```

Further observed RED cases before their fixes:

- missing lease/writer/fault APIs (25 expected compile errors);
- persisted total-log cap test accepted the second segment;
- exact version-1 record test found an extra payload-length field (`left: 0`, `right: 131`);
- sealed-middle trailing corruption returned a recoverable result instead of failing closed.

GREEN focused result after the controller-authorized module split and final changes:

```text
running 16 tests
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 454 filtered out
```

Coverage includes clean/torn/corrupt recovery, exact framing, canonical/lossless identity, wrong
owner/scope/epoch, non-monotonic sequence, record/count/segment/log bounds, exclusive/stale owner,
disk-full append, durable sync ordering, heartbeat failure, all rotation boundaries, orphan header,
broken segment lineage, sealed-middle corruption, and fresh schema-v18 SQL compatibility.

## Regression and quality evidence

Task 2 state suite:

```text
cargo test -p trail --lib db::change_ledger::store::tests -- --nocapture
test result: ok. 22 passed; 0 failed
```

Hard-cutover suite:

```text
cargo test -p trail --test schema_v18_hard_cutover -- --nocapture
test result: ok. 8 passed; 0 failed
```

Bounded library split:

```text
CARGO_NET_OFFLINE=true CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER= \
  cargo test -p trail --lib --quiet -- \
  --skip cargo_adapter_builds_once_and_reuses_one_immutable_target_seed \
  --skip automatic_detection_rejects_ambiguous_polyglot_roots

test result: ok. 467 passed; 0 failed; 1 ignored; 2 filtered out
```

Both skipped adapter tests then passed exactly with sccache hidden from `PATH` (1/1 each).

`cargo clippy -p trail --lib --no-deps` exits zero. It reports the repository's existing large
`Error`/`result_large_err` warning family and other pre-existing crate-wide warnings. The two new
actionable warnings (manual range containment and an error-kind MSRV mismatch) were fixed. Formatting,
`git diff --check`, and the focused build are clean.

## Self-review

- Protocol surface remains crate-private and dormant.
- No primary Trail connection is borrowed by observer callbacks.
- No Prolly file or Gradle state was modified.
- No migration, legacy compatibility, public authority, acknowledgement, baseline, or scope-offset
  behavior was added.
- SQLite publication is always after the relevant file/directory sync step.
- Raw on-disk identity bytes and lowercase SQLite hex are round-tripped by tests.
- Recovery allocates only after persisted and fixed protocol bounds are checked.
- A full fresh schema-v18 fixture exercises acquire, append, flush, and rotation against actual
  CHECK/UNIQUE/foreign-key constraints.

## Concerns and deliberate limits

- The protocol is intentionally not activated; later tasks must fold records and reconcile before
  any trusted read can consume it.
- Recovery conservatively marks an open unpublished tail or orphan header for reconciliation; it
  does not infer durability beyond published offsets.
- Power-loss continuity is explicitly not claimed.
- No public integration test exists by design; direct protocol tests remain within the crate-private
  module until a real public behavior is activated.

## Review fixes

Status: all Task 3 review findings were addressed while keeping the protocol crate-private and
dormant. This is process-crash ordering evidence only; it does not add or imply a power-loss claim.

### RED/GREEN evidence

The pre-fix focused baseline was green at 16 tests. The review regression suite then established RED
with missing bounded-filename/no-follow support, no post-write lease-expiry boundary, and no writer
runtime-pragma evidence:

```text
error[E0425]: cannot find value `MAX_SEGMENT_FILENAME_BYTES` in this scope
error[E0599]: no method named `runtime_pragmas` found for struct `SegmentWriter`
error[E0599]: no variant named `AppendPostWriteLeaseExpiry` found for enum `FaultPoint`
```

During GREEN, the real sidecar test exposed a second RED: plain SQLite read-only recovery recreated
`trail.db-wal`. Recovery now uses an immutable read-only URI when both sidecars are absent, uses a
normal read-only connection only when both WAL sidecars already exist, and fails closed on incomplete
sidecar state. The final focused result is:

```text
running 27 tests
test result: ok. 27 passed; 0 failed; 0 ignored; 454 filtered out
```

The focused suite now proves:

- a different owner cannot replace expired/revoked/error ownership in the same global scope epoch;
  the real full-v18 owner rows, segment rows, and segment files remain byte-for-byte unchanged;
- after an explicit authoritative scope-epoch increase, replacement succeeds against the full v18
  schema and the new epoch starts at sequence 1 while the prior epoch metadata remains intact;
- acquisition and rotation count every header against segment and cumulative byte caps before
  authoritative metadata publication;
- metadata is streamed with `query.next()`, segment count is bounded before row strings/records,
  text fields are length-checked before allocation, and totals use checked arithmetic;
- SQLite stores only the exact derived relative `<segment-id>.cpl` filename; traversal, oversized
  metadata, and symlink final components fail closed;
- the current macOS host executes real directory `fsync` and `O_NOFOLLOW` symlink rejection tests;
  a Linux-only direct no-follow test remains compiled under `cfg(target_os = "linux")` for Task 7;
- append validates the unexpired lease immediately before and after file I/O; injected post-write
  expiry leaves sequence/hash/durable metadata unpublished and recovery requires reconciliation;
- the dedicated writer connection has WAL, synchronous NORMAL, foreign keys ON, and temp store
  MEMORY; recovery is read-only/non-creating and does not create WAL/SHM just to inspect;
- flush rejects a synchronized file whose exact length differs from the claimed current offset;
- independently crafted noncanonical CBOR is rejected, and an independent parser checks the six
  framing fields and checksum boundary.

### Atomic rotation crash evidence

Rotation now owns one IMMEDIATE transaction from initial lease validation through file work and both
SQL mutations. It syncs/hashes the old file, writes and syncs the new header and directory, revalidates
the unexpired lease, updates the old row to sealed, inserts the new open row, and commits. The
in-memory file/segment switch occurs only after commit.

For every injected boundary (`RotationOldSync`, `FirstDirectorySync`, `NextHeaderCreate`,
`NextHeaderWrite`, `NextHeaderSync`, `SecondDirectorySync`, `SealPublication`, and
`NextMetadataPublication`), the test runs `recover_segments` against the actual resulting v18 SQLite
and filesystem state. Every pre-commit failure leaves the old metadata open and/or an orphan file and
returns `requires_reconciliation=true`; the writer is retired and cannot append. A clean rotation
atomically exposes one sealed old row and one open new row with verified hash lineage.

### Exact verification commands

```text
cargo test -p trail --lib db::change_ledger::log::tests -- --nocapture
  27 passed; 0 failed

cargo test -p trail --lib db::change_ledger::store::tests -- --nocapture
  22 passed; 0 failed

cargo test -p trail --test schema_v18_hard_cutover -- --nocapture
  8 passed; 0 failed

CARGO_NET_OFFLINE=true CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER= \
  cargo test -p trail --lib --quiet -- \
  --skip cargo_adapter_builds_once_and_reuses_one_immutable_target_seed \
  --skip automatic_detection_rejects_ambiguous_polyglot_roots
  478 passed; 0 failed; 1 ignored; 2 filtered out

env PATH="$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
  CARGO_NET_OFFLINE=true CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER= \
  cargo test -p trail --lib cargo_adapter_builds_once_and_reuses_one_immutable_target_seed
  1 passed; 0 failed

env PATH="$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
  CARGO_NET_OFFLINE=true CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER= \
  cargo test -p trail --lib automatic_detection_rejects_ambiguous_polyglot_roots
  1 passed; 0 failed

cargo clippy -p trail --lib --no-deps
  exit 0; repository-wide pre-existing warning families only

cargo fmt --all -- --check
git diff --check
  clean
```

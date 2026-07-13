# Activated Changed-Path Ledger

Status: approved implementation design

## Purpose

Make the common clean and small-change paths for Trail workspaces and
materialized lanes proportional to authoritative changed-path candidates rather
than repository size, without treating incomplete observation as proof that a
worktree is clean.

This design complements the persistent case-fold path index. That index makes
immutable-root mutation validation local. The changed-path ledger makes mutable
filesystem observation local only when a qualified evidence provider proves
continuity. Linux and macOS ship with qualified observers and activate the
ledger by default. Other platforms retain the direct full-observation command
path and cannot persist ledger trust.

## Non-negotiable invariants

Only a `trusted` scope may prove clean or supply a complete candidate set.
`reconciling`, `overflow`, `untrusted_gap`, `stale_baseline`, and `corrupt`
never report clean.

A callback watermark is not an observation fence. Generic `notify` callbacks
prove only which events have arrived, not that all writes before a snapshot cut
have been delivered. Clean proof requires one of:

- a continuously owned live provider with complete sequencing and a
  linearizable delivery fence;
- a provider with a durable cursor and linearizable delivery fence when trust
  is resumed across owner or process restart;
- a controlled writer whose intent and authoritative publication are covered
  by Trail's transaction protocol;
- a documented external-writer quiescence protocol held through a full
  reconciliation cut; or
- an exact Git/VCS comparison whose baseline and semantics equal the ledger
  baseline.

Observer owner loss or restart rotates the scope epoch and requires
reconciliation unless a qualified durable cursor resumes without a gap. There
is no generic cross-platform shortcut after arbitrary writes occur while no
qualified observer is active.

Activation is one hard release gate. Schema, recovery, reconciliation, policy
dependency tracking, every controlled producer, both qualified Linux and macOS
adapters, automatic daemon lifecycle, and read-path integration must land and
pass together. There is no partially authoritative rollout and no feature mode
that promotes trust before all activation gates pass.

## Scope and filesystem identity

Every evidence record binds to:

- stable scope ID, kind, and owner (`workspace`, `materialized_lane`, or
  `workspace_view`);
- pinned canonical scope-root handle/identity and platform-specific filesystem
  identity;
- mutable ref name, generation, change ID, and baseline root ID;
- compiled Trail policy fingerprint and persisted dependency manifest;
- ledger epoch and qualified provider cursor/fence state;
- observer capability record and owner token.

Evidence from a different root, policy, filesystem, epoch, provider, or owner
cannot prove clean. Root replacement, mount change, case-sensitivity change, or
scope directory replacement invalidates the epoch.

Watcher paths are normalized relative to a pinned no-follow scope root. Case
aliases and case-only renames preserve exact spelling while using the existing
folded-path collision rules. Unsupported/non-UTF-8 names make the affected
scope or provider unqualified; they are never silently dropped.

## Trust states

- `trusted`: all controlled producers and a qualified external provider prove
  the candidate set complete through the stored cut.
- `reconciling`: a persisted staged reconciliation exists but is not published.
- `overflow`: a qualified provider reported lost evidence or bounds exceeded.
- `untrusted_gap`: no qualified evidence covers an interval.
- `stale_baseline`: ref, root, policy, filesystem, or provider identity changed.
- `corrupt`: ledger, journal, intent, or schema validation failed.

Only `trusted` can return `Clean` or an authoritative bounded candidate set.
An exact dirty path/prefix may remain useful while a scope is untrusted, but it
cannot prove that no other path changed.

## Observer capability tiers

Each adapter persists capabilities rather than relying on its name:

```text
durable_cursor
linearizable_fence
rename_pairing
overflow_scope
filesystem_supported
clean_proof_allowed
power_loss_durability
```

- Generic `notify` has no durable cursor or generic delivery fence and remains
  advisory.
- Linux uses a native recursive inotify adapter with explicit watch coverage,
  rename cookies, queue-overflow scope, directory-race detection, and a
  sentinel delivery fence. It qualifies only while the owner is live and after
  a fence-covered reconciliation; restart without a continuous owner requires
  reconciliation.
- macOS uses FSEvents file events with persisted event IDs, root identity,
  `MustScanSubDirs`/gap handling, and synchronous stream flushing. It resumes a
  cursor only when the platform proves continuity; otherwise it reconciles.
- Unknown, NFS, SMB, FUSE, overlay, network, or replaced filesystems default to
  reconciliation-required unless an adapter explicitly qualifies them.
- Process-crash durability and host/power-loss durability are separate
  capabilities and test claims.

Actual ext4 and APFS adapters/filesystems are exercised in scheduled jobs;
injected callback tests alone cannot qualify a provider.

## Persistence and schema completeness

Schema v18 is a hard cutover, not a migration. Only an explicit fresh
`trail init` may create it. Before opening the mutable stores, every existing
workspace is preflighted through a read-only SQLite connection. Any version
other than 18, or any partial/malformed v18 shape, returns stable
`SCHEMA_REINITIALIZE_REQUIRED` guidance and leaves the database and sidecars
byte-for-byte unchanged. Nothing is repaired on open.

Fresh creation writes the final v18 schema in one savepoint, records both
`PRAGMA user_version` and schema metadata only after all objects exist, and
validates the exact tables, columns, defaults, checks, foreign keys, uniqueness,
index order/collation, and log-format bounds before release. There are no
legacy columns, upgrade functions, backfills, compatibility views, or
`CREATE IF NOT EXISTS` repair paths for an existing workspace.

Fresh scopes begin untrusted and can become `trusted` only after qualified
observation and a successfully published reconciliation.

### Scope and candidate tables

`changed_path_scopes` stores stable scope/owner identity, filesystem identity,
ref generation/change/root, policy fingerprint/dependency generation, trust
state/reason, epoch, provider capabilities/cursor, durable/folded offsets,
single-writer owner token, heartbeat/error state, and timestamps. It has a
unique `(scope_kind, owner_id)` constraint.

`changed_path_entries` stores exact normalized paths, event flags, source mask,
first/last sequence, and provider/intent ownership. `changed_path_prefixes`
stores coalesced dirty prefixes with completeness reason and sequence bounds.
Neither stores all clean paths. Exact and prefix queries use indexed binary
ranges, not escaped `LIKE` scans.

`changed_path_policy_dependencies` stores every policy input needed to reuse a
compiled policy without repository-wide rediscovery: normalized dependency
path or external identity, kind, content/metadata identity, observability,
generation, and last source sequence. Raw dependency events invalidate trust
before normal ignore filtering.

### Controlled intent tables

`changed_path_intents` stores producer, expected scope epoch/ref generation and
root, immutable target change/root/operation, start cursor, and lifecycle:
`prepared`, `filesystem_applied`, `published`, `acknowledged`, `aborted`.
`changed_path_intent_paths` and `changed_path_intent_prefixes` store affected
evidence. Prepared and published targets are GC roots until recovery reaches a
terminal state.

### Reconciliation staging

`changed_path_reconciliations` stores attempt ID, scope/epoch, expected ref,
filesystem/policy/provider identity, start fence/cursor, mode/reason,
completeness class, staged store location, and state. A crash-safe temporary
table/store streams observed entries and deletions without retaining all files
in memory. Publication is one CAS transaction; abandoned attempts are safe to
discard.

### Observer segment metadata

SQLite stores log format version, epoch, owner token, segment ID, first/last
sequence, durable end offset, folded end offset, hash linkage, state, and
rotation lineage. A retired writer cannot append evidence to a reused scope.

## Versioned observer log protocol

High-rate callbacks never acquire the workspace lock or primary `Trail` SQLite
connection. A qualified adapter owns an exclusive epoch/owner lease and appends
versioned length-prefixed records to
`.trail/index/change-ledger/<scope>/<epoch>/`.

Every segment header contains format, scope, epoch, owner, provider cursor, and
previous-segment hash. Records have monotonic sequences, normalized payload,
checksum/hash linkage, and explicit rename/prefix semantics. The writer
publishes a durable end offset only after the configured fsync/group-commit
boundary. Rotation fsyncs the old tail and directory, publishes the next
header, then updates ownership metadata.

Append/flush/disk-full/checksum/lease/heartbeat/callback failure revokes clean
proof. Because the same I/O failure may prevent persisting an error marker, a
clean-returning snapshot must obtain a fresh fence nonce/cut from the live epoch
owner after that owner successfully flushes and validates its append path.
Failure to contact, fence, flush, or validate the owner returns untrusted.
Persisted error markers and heartbeat expiry are diagnostic fallbacks, never
the sole revocation mechanism. A snapshot must also fold or conservatively
merge the durable tail; it cannot trust only an older SQLite folded offset.
Read-only clients either obtain this proven live snapshot, fold/merge a
provider-qualified safe tail, or return reconcile-required.

Observer logs and intent files are explicitly synced before their durability
offset or lifecycle transition is published. SQLite remains WAL
`synchronous=NORMAL`, so a dead owner or host restart always revokes trust and
forces reconciliation unless a platform durable cursor independently proves
continuity. No power-loss clean-proof capability is advertised without a
stronger transaction/fsync path and fault tests.

## Deep module API

One `ChangedPathLedger` owns trust, evidence, CAS, and fallback semantics:

```text
begin_scope(scope, baseline, policy, filesystem, provider)
snapshot(scope, expected_ref, required_capabilities)
prepare_intent(scope, expected, target, paths, prefixes)
mark_filesystem_applied(intent, verified_cut)
publish_intent(intent, ref_and_ledger_transaction)
acknowledge(snapshot, owned_evidence, covered_source_sequences)
mark_prefix_dirty(scope, complete_prefix_proof)
mark_untrusted(scope, state, reason)
begin_reconciliation(scope, expected, mode)
publish_reconciliation(attempt, expected_fence)
advance_baseline(scope, expected, target, covered_evidence)
recover(scope)
```

Snapshots include epoch, ref/root, policy/filesystem identity, capabilities,
provider cursor/fence, durable/folded offsets, exact paths/prefixes, source and
last sequence, and trust state.

## Module boundaries

The implementation is split so persistence, observation, recovery, and command
integration can be audited independently:

- `change_ledger/types.rs`: typed identities, capabilities, trust states, cuts,
  evidence, intents, and snapshots;
- `change_ledger/store.rs`: exact SQLite persistence, CAS transitions, binary
  range queries, and schema-independent state validation;
- `change_ledger/log.rs`: versioned records, checksums, hash chains, rotation,
  durable offsets, and safe-tail recovery;
- `change_ledger/observer/linux.rs`: qualified inotify ownership, recursive
  coverage, event normalization, overflow, rename, and fence protocol;
- `change_ledger/observer/macos.rs`: qualified FSEvents stream, event-ID
  continuity, gap flags, root identity, and flush protocol;
- `change_ledger/reconcile.rs`: streamed full/prefix observation and CAS
  publication;
- `change_ledger/intent.rs`: controlled filesystem mutation state machines;
- `change_ledger/recovery.rs`: leases, segments, intents, mirrors, and GC roots;
- `change_ledger/policy.rs`: compiled policy dependency persistence and raw
  invalidation;
- `change_ledger/snapshot.rs`: live fencing, tail folding, candidate snapshots,
  and sequence-aware acknowledgement.

Daemon startup and `status`/`diff`/`record` integrations remain thin consumers
of this module. No read path reimplements trust decisions.

## Automatic per-workspace daemon

The first `status`, `diff`, or `record` securely discovers or starts one
per-workspace daemon. Startup uses an exclusive process lock, a random
authenticated local endpoint, owner nonce, executable identity, and atomic
endpoint publication. Concurrent clients converge on the same owner and wait
for a bounded readiness result. Stale endpoint files and dead owners are
replaced only after process-identity validation.

The daemon acquires the observer lease, runs recovery, establishes platform
watch coverage, reconciles when trust is absent, and publishes readiness only
after a valid fence. Failure to start or reconcile returns
`CHANGE_LEDGER_RECONCILE_REQUIRED`; it never exposes an older persisted cache
as authoritative.

An automatic daemon is an optimization owner, not a correctness dependency.
If it exits, the next command starts a new owner and reconciles any interval
that cannot be proven continuous.

## Read command flow

`status` and `diff` follow one flow:

1. ensure the authenticated daemon is ready;
2. recover unfinished state and validate scope/ref/root/policy/filesystem/
   provider identities;
3. fold the durable observer tail through fence `c1`;
4. automatically run full reconciliation and retry if trust is unavailable;
5. load only exact paths and complete dirty prefixes;
6. compare candidates with pinned no-follow reads and batched root lookups;
7. fence/fold through `c2`, retaining or retrying for events newer than `c1`;
8. return clean only when the complete candidate set is empty at the proven
   cut.

On Linux and macOS, the existing direct full-walk and Git paths are
reconciliation/oracle implementations, not permissive ledger clean-proof
fallbacks. Unsupported platforms continue to use direct full observation and
never publish a trusted ledger scope.

## Controlled and observed mutation classes

Every filesystem producer is assigned to exactly one state machine.

### Observed record/checkpoint

Manual record, materialized-lane record, and native-agent checkpoint observe
external filesystem state and then publish a new immutable root:

1. Acquire the workspace lock and capture expected ref/scope epoch.
2. Obtain qualified provider fence `c1` (or hold documented quiescence).
3. Read/hash candidates through pinned no-follow handles.
4. Obtain fence `c2`; retain all evidence newer than `c1`.
5. Build/store immutable target root/operation from the observation.
6. In one SQLite transaction, index the operation, CAS ref, advance the ledger
   baseline, and acknowledge only evidence source-sequenced through `c1`.
7. Repair mirrors and retain later/different-source same-path evidence.

### Ref-advancing controlled projection

A controlled command that both knows the intended target and advances a ref:

1. Lock and capture expected ref/scope epoch.
2. Compute/store immutable target root/operation.
3. Durably prepare exact/prefix intent referencing the target.
4. Apply and sync files/directories; verify intended content/mode/type.
5. Fence the qualified provider and retain unrelated/later evidence.
6. In one SQLite transaction, index operation, CAS ref, publish intent, advance
   baseline, and acknowledge only intent-owned/covered evidence.
7. Repair mirrors and reach a terminal intent state.

### Projection-only alignment

Checkout materialization, sparse hydration, and workdir sync often project an
existing ref/root without creating an operation or advancing a ref. Their
intent references that existing target, applies/syncs/verifies the filesystem,
then fences/flushes the qualified provider, retains unrelated or later
source-sequenced evidence, and atomically publishes only the scope
baseline/marker and terminal intent under unchanged ref/epoch/provider-cut CAS.

An external same-path event with a later or different source sequence is never
cleared by path equality. A crash leaves a false-positive candidate, not a
missed change. Recovery runs before any later mutation or GC and compares
intent, filesystem verification, ref, operation, and ledger states to finish,
retain candidates, or reconcile.

## VFS/COW producer exception

VFS callbacks are a separate producer class because they already hold the
shared view barrier. They acquire only that barrier and append/fsync a per-view
intent record before semantic upper/whiteout mutation. They never acquire the
workspace lock or primary SQLite connection.

Checkpointing acquires workspace lock, then exclusive view barrier, folds the
per-view journal, verifies candidates, builds immutable targets, and publishes
ref/ledger state. Group-commit acknowledgement must not expose a filesystem
mutation before its intent is durable under the claimed crash model.

First implement journal/whiteout correctness, active-handle generation
ownership, and crash recovery. Only then compact: replay records after the
clean cut, rotate at checkpoint, roll generations when no active handle uses
the old upper, and retire old upper/whiteouts. `upper_recovery_walks=0` is
permitted only for a qualified trusted journal; untrusted, corrupt, or gapped
views explicitly scan and reconcile.

Whiteouts use an incremental map/log with atomic mutation semantics rather than
rewriting all class arrays.

## External observation and snapshot cuts

Generic `w1`/`w2` callback flushing prevents clearing already-received later
events but is not a delivery fence. A qualified provider must supply a durable
cursor/fence proving all changes through the cut. Without that, external
writers must be quiesced through enumeration/hash/publication or the scope
remains `untrusted_gap`.

For a qualified provider:

1. fence and fold through cut `c1`;
2. read/hash authoritative candidates;
3. fence/fold through `c2`;
4. publish/ack only evidence whose source sequence is covered by `c1`;
5. retain later/different-source evidence against the next baseline.

Delayed delivery, callback backlog, cursor discontinuity, owner death, lease
replacement, and ambiguous rename tests must prove fail-closed behavior.

## Git adapter

Git is authoritative only when the ledger baseline root exactly equals the
mapped Git HEAD/index tree under equivalent file-mode, symlink, sparse,
submodule, and ignore semantics. A mapping to an arbitrary Git state is not
enough: after Trail records ahead of HEAD, a reversion toward HEAD can be clean
to Git but dirty to Trail.

Use porcelain v2 `-z`, retain both rename endpoints, and bind evidence to HEAD,
index/split/shared-index identity, exact mapped Trail root, filesystem identity,
and Trail policy equivalence. `assume-unchanged`, `skip-worktree`, unresolved
sparse/submodule behavior, racy/untrusted fsmonitor, or policy mismatch disables
Git clean proof. Git may remain a conservative advisory source.

Fsmonitor and untracked-cache improve locality only when their continuity is
qualified. Metrics expose Git subprocess count, index refresh/Trace2 work, and
whether Git internally performed global work.

## Materialized lanes and deployment contract

Replace the N-entry JSON manifest only after explicit reconciliation with a
compact v2 marker containing scope ID, filesystem identity, ref/generation/root,
policy fingerprint, epoch, and provider cut. Marker and trusted scope identity
publish at the same logical cut; mismatch is stale.

Materialized-lane O(k) operation requires the automatically managed qualified
observer or a durable COW journal that proves continuity. Owner loss marks the
scope untrusted and the next command automatically reconciles it. Once trusted,
lane status, record, readiness, merge checks, and ordinary preview consume
ledger candidates. Full recursive ignored/risk scans remain explicit audit or
reconciliation operations.

## Trail policy and adapter equivalence

`RecordingPolicySnapshot` is Trail's authoritative semantic policy. It is
shared by Trail walkers and candidate checks; each external adapter declares
whether its semantics are equivalent or only conservative.

Reconciliation discovers and persists a dependency manifest covering built-in
rule version, configuration, all relevant nested `.trailignore`/`.gitignore`,
`.git/info/exclude`, `core.excludesFile`, included Git config, system/global
inputs, normalization, mode/symlink rules, and case sensitivity. Observers see
raw policy-file events before ignore filtering. Unobserved or unresolved
external dependencies mark the scope stale. Fast paths reuse the persisted
dependency manifest rather than traversing the tree to rediscover it.

## Path/prefix semantics

- File deletion: exact candidate; absence against baseline means deletion.
- File rename: both endpoints with source/provider sequence.
- Deleted directory: complete dirty prefix; expand baseline Prolly range.
- Added directory: scan only the new subtree when provider proof is complete.
- Directory rename: expand old baseline prefix and scan new subtree.
- Conservative complete ambiguous event: dirty parent prefix, still trusted.
- Actual event loss/unknown overflow: global untrusted state.
- Coalesce overlapping prefixes without losing completeness provenance.

Prefix reconciliation may promote trust only when persisted provider proof says
the invalidation is fully contained in those prefixes. User-supplied prefixes
can refresh data but cannot repair global overflow, owner loss, unknown gap,
global policy change, or corruption; those require a full scope reconciliation.

## Correctness-grade reconciliation

Add `CHANGE_LEDGER_RECONCILE_REQUIRED` with stable exit code, structured scope,
state/reason, and exact recovery commands:

```text
trail index reconcile
trail index reconcile --lane <lane>
```

The error/report propagates through human and JSON CLI, daemon RPC, REST/OpenAPI,
and MCP. Read-only surfaces never ignore an unread log tail. Normal
`status`/`diff`/`record` first attempt automatic full reconciliation; this error
is returned only when daemon startup, observation, or reconciliation cannot
establish trust.

Gap recovery enumerates the required scope and baseline Prolly range, hashes
all relevant regular-file content through pinned no-follow handles, validates
before/after identity to detect replacement, and finds deletions by comparing
both sides. Cached stamps can optimize only when a gap-free qualified journal
proves they cover the interval.

The staged attempt publishes only if ref/root, scope epoch, filesystem identity,
policy/dependency generation, provider capabilities, and a real fence/cursor
still match. Races retry or remain untrusted.

Every normal command automatically performs and labels full reconciliation
when required. Authorization to run the command permits this expense but does
not create an observation fence. Reconciliation promotes trust only with a
qualified provider fence or documented external-writer quiescence; otherwise
the command fails with the stable recovery action. No O(N) run may be
represented as a trusted fast path, and reconciliation metrics are distinct
from warm command metrics.

## Concurrency and lock order

Ordinary writers/checkpointing use workspace write lock, then applicable
exclusive scope/view barrier, then a short ledger/authoritative SQLite
transaction. Filesystem reads occur outside long SQL transactions while pinned
to a snapshot identity.

VFS callbacks use only the shared view barrier and per-view append protocol.
External watcher callbacks use only the observer append path and bounded memory.
Neither acquires the workspace lock or primary connection. Snapshot and
baseline changes use ref/epoch/provider-cut CAS.

## Backup, restore, deletion, and GC

Backup either fences/folds and snapshots matching SQL+segment state or marks
backed-up scopes untrusted. Active views require their barriers or restore as
untrusted. Restore always rotates epochs, clears owner/cursor state, discards
unmatched tails, rebinds filesystem identity, and marks scopes
reconcile-required.

Prepared/published intent target roots and operations are GC roots until
recovery reaches a terminal state. Lane/view deletion transactionally retires
scopes and revokes owners before segments are removed. Tests cover crash during
backup rotation, restore to another filesystem, orphan segments, and GC before
and after intent recovery.

## Structural observability

Track per operation:

- full scope walks/root ranges/SQLite full-index rows;
- authoritative input candidate count versus final changed output;
- candidate reads, hashes, root point lookups, and ledger rows;
- marker bytes, log bytes/segments, folded/tail records, backlog;
- upper recovery walks and journal compaction work;
- policy dependency work and adapter equivalence;
- observer capability/cursor/fence/trust and reconciliation reason;
- Git subprocess/index-refresh/Trace2 global work;
- wall time and peak RSS.

## Acceptance gates

Fresh-schema tests prove exact v18 creation. Hard-cutover tests snapshot every
database and sidecar byte, attempt to open v17, version 0 outside explicit
initialization, newer versions, and partial/malformed v18, and prove rejection
without mutation. Existing-schema open contains no DDL or repair path.

CI uses 1k; scheduled Linux and macOS jobs use 100k on qualified ext4 and APFS
environments; nightly jobs use 1M, cold reconciliation, daemon restart, and
crash matrices. Exercise input candidate counts `k=0,1,100` separately from
final changes for workspace status/diff/record, materialized lane record,
structured patch maintenance, and COW checkpoint.

Warm trusted assertions:

```text
full_scope_walks = 0
root_full_ranges = 0
sqlite_full_index_rows = 0
full_manifest_bytes_read = 0
full_manifest_bytes_written = 0
upper_recovery_walks = 0
external_adapter_global_work = 0
policy_dependency_full_discovery = 0
observer_tail_records_folded <= configured_tail_bound
candidate_and_prefix_rows <= configured_scope_caps
observer_log_and_segment_bytes <= configured_scope_caps
candidate_reads <= authoritative_candidate_count + bounded prefix expansion/output
root_point_lookups <= small constant * (authoritative candidates + expanded output)
ledger_rows_touched <= small constant * authoritative candidates
```

Every candidate, prefix, segment-byte, and unfurled-tail cap is persisted and
enforced; exceeding one fails closed. Scheduled 100k/1M jobs also enforce
calibrated wall-time and peak-RSS regression budgets. Cold start, owner restart,
and cursor-gap scenarios expect labeled reconciliation instead of the warm
bound. A full-scan correctness oracle runs after and outside each measured fast
path.

State-machine and property tests cover every trust transition, CAS race,
evidence coalescing rule, and acknowledgement boundary. Linux and macOS real
adapter suites cover recursive directory races, rename storms, delayed
delivery, backlog, overflow/gap flags, owner death, cursor replacement,
filesystem replacement, and fence ordering.

Scenarios include create/content/mode/delete, file/directory/case-only rename,
reverted candidates, ignored subtrees, nested/global policy changes, delayed
delivery, callback backlog, overflow, disk full, log tears, daemon death,
epoch replacement, concurrent same-path writes, backup/restore/GC, and process
kill at every object, intent, filesystem sync, observer sync, ref, ledger, and
mirror boundary. Corruption tests cover checksums, hash chains, offsets,
unsupported formats, and partial SQLite state. Property tests permute event
orders and prove false positives are allowed but false clean is not.

Activation is compiled on by default only after the controlled-producer
inventory and both real Linux/macOS adapter suites pass. Other platforms use
the direct full-observation command path and never persist trusted ledger
proof.

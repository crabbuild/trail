# Correctness-First Changed-Path Ledger

Status: approved implementation design

## Purpose

Make the common clean and small-change paths for Trail workspaces and
materialized lanes proportional to authoritative changed-path candidates rather
than repository size, without treating incomplete observation as proof that a
worktree is clean.

This design complements the persistent case-fold path index. That index makes
immutable-root mutation validation local. The changed-path ledger makes mutable
filesystem observation local only when a qualified evidence provider proves
continuity.

## Non-negotiable invariants

Only a `trusted` scope may prove clean or supply a complete candidate set.
`overflow`, `untrusted_gap`, `stale_baseline`, `legacy_reconcile_required`, and
`corrupt` never report clean.

A callback watermark is not an observation fence. Generic `notify`/inotify
callbacks prove only which events have arrived, not that all writes before a
snapshot cut have been delivered. Clean proof requires one of:

- a provider with a durable cursor and linearizable delivery fence;
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

The ledger stays dormant and feature-disabled until schema, recovery,
reconciliation, policy dependency tracking, every controlled producer, and at
least one qualified external adapter are installed. Partial rollout cannot
promote a scope to `trusted` or alter existing read behavior.

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
- `legacy_reconcile_required`: migrated caches have not been reconciled.
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

- Generic `notify`/inotify has no durable cursor or generic delivery fence.
- Platform journal adapters (for example FSEvents/USN-like facilities) qualify
  only for filesystems and cursor semantics proven by adapter tests.
- Unknown, NFS, SMB, FUSE, overlay, network, or replaced filesystems default to
  reconciliation-required unless an adapter explicitly qualifies them.
- Process-crash durability and host/power-loss durability are separate
  capabilities and test claims.

The actual ext4/APFS/NTFS adapters and filesystems are exercised in scheduled
jobs; injected callback tests alone cannot qualify a provider.

## Persistence and schema completeness

At implementation time, bump the then-current SQLite schema by exactly one
(v16→v17 if no earlier migration lands; v17→v18 if the pending path-index
derived-repair migration lands first). The fast path must verify `PRAGMA user_version`,
schema metadata, exact required tables, critical columns, foreign keys,
uniqueness, and required indexes—not merely table names. Tests cover a dropped
index, missing/wrong column, wrong uniqueness, partial migration, and newer or
malformed log formats.

New and migrated scopes start `legacy_reconcile_required`; `begin_scope` cannot
manufacture trust.

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

The implementation documents whether an intent/log sync survives process
crash only or host power loss. SQLite currently uses WAL `synchronous=NORMAL`,
so power-loss guarantees require an explicit stronger transaction/fsync path
and fault tests rather than wording alone.

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
permitted only for a qualified trusted journal; legacy/corrupt/gapped views
explicitly scan and reconcile.

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

Materialized-lane O(k) operation requires a qualified long-lived observer or a
durable journal/Git adapter that proves continuity. CLI-only scopes become
`untrusted_gap` when the command exits and reconcile on next use. This is a
user-visible deployment prerequisite, not an implicit optimization.

Legacy manifests may seed explicit reconciliation but cannot prove complete
candidates after a gap. Once trusted, lane status, record, readiness, merge
checks, and ordinary preview consume ledger candidates. Full recursive
ignored/risk scans remain explicit audit/reconciliation operations.

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
can refresh data but cannot repair global overflow, legacy migration, owner
loss, unknown gap, global policy change, or corruption; those require a full
scope reconciliation.

## Correctness-grade reconciliation

Add `CHANGE_LEDGER_RECONCILE_REQUIRED` with stable exit code, structured scope,
state/reason, and exact recovery commands:

```text
trail index reconcile
trail index reconcile --lane <lane>
```

The error/report propagates through human and JSON CLI, daemon RPC, REST/OpenAPI,
and MCP. Read-only surfaces never ignore an unread log tail.

Gap recovery enumerates the required scope and baseline Prolly range, hashes
all relevant regular-file content through pinned no-follow handles, validates
before/after identity to detect replacement, and finds deletions by comparing
both sides. Cached stamps can optimize only when a gap-free qualified journal
proves they cover the interval.

The staged attempt publishes only if ref/root, scope epoch, filesystem identity,
policy/dependency generation, provider capabilities, and a real fence/cursor
still match. Races retry or remain untrusted.

For compatibility, interactive `status` may perform and label an automatic
reconciliation; `--no-reconcile` reports unknown. Authorization permits the
expense but does not create an observation fence. Automatic or authorized
reconciliation promotes trust only with a qualified provider fence or the
documented external-writer quiescence protocol; otherwise it returns a labeled
point-in-time scan and leaves `untrusted_gap`. Expensive mutating operations
such as record/checkpoint fail with the stable recovery action unless the user
or daemon explicitly authorizes reconciliation. No O(N) run may be represented
as a trusted fast path.

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
- legacy manifest bytes, log bytes/segments, folded/tail records, backlog;
- upper recovery walks and journal compaction work;
- policy dependency work and adapter equivalence;
- observer capability/cursor/fence/trust and reconciliation reason;
- Git subprocess/index-refresh/Trace2 global work;
- wall time and peak RSS.

## Acceptance gates

CI uses 1k; scheduled Linux/macOS/Windows jobs use 100k and 1M on qualified
ext4/APFS/NTFS-like environments. Exercise input candidate counts `k=0,1,100`
separately from final changes for workspace status/diff/record, materialized
lane record, structured patch maintenance, and COW checkpoint.

Warm trusted assertions:

```text
full_scope_walks = 0
root_full_ranges = 0
sqlite_full_index_rows = 0
legacy_manifest_bytes_read = 0
legacy_manifest_bytes_written = 0
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

Scenarios include create/content/mode/delete, file/directory/case-only rename,
reverted candidates, ignored subtrees, nested/global policy changes, delayed
delivery, callback backlog, overflow, disk full, log tears, daemon death,
epoch replacement, concurrent same-path writes, legacy migration, backup/
restore/GC, and process kill at every intent/fsync/ref/ledger/mirror boundary.
Property tests permute event orders and prove false positives are allowed but
false clean is not.

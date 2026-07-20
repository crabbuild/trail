# SQLite Lane Initialization Coordination Design

## Status

Approved architecture. This design replaces the unmerged filesystem-shard
singleflight implementation on `codex/trail-superset-scale-verification`.

## Problem

Concurrent identical lane-spawn requests must publish one lane and replay one
durable result. The current feature branch tries to serialize those requests
with persistent filesystem anchors. Repeated review found that it cannot
simultaneously provide all required properties:

- reject live pathname or directory replacement without split authority;
- recover after first-publication crashes and partial writes;
- remain portable across quiescent workspace copies and volume moves;
- behave identically on Unix and Windows; and
- avoid a global filesystem-lock bottleneck at 64 or 128 concurrent lanes.

The Trail SQLite database is already the durable authority for the lane
initialization identity and phase. Ownership belongs in that same transaction
domain.

## Considered approaches

### 1. SQLite owner rows with generation fencing — selected

Use a short SQLite `BEGIN IMMEDIATE` transaction to claim one initialization,
then release the database writer lock while materialization runs. Every durable
phase transition is fenced by the exact owner token and generation. Contenders
poll durable state, replay a terminal result, or take over only after proving
the recorded process identity is dead.

This keeps unrelated lanes concurrent, makes copies portable when quiescent,
and uses the same cross-platform SQLite semantics Trail already depends on.

### 2. Hold one SQLite write transaction for the whole spawn — rejected

This is mechanically simple but serializes all lane creation, blocks unrelated
database writers during large materializations, and cannot meet the scale goal.

### 3. Keep filesystem locks and bind them to a SQLite epoch — rejected

SQLite could authenticate each filesystem inode generation, but correctness
would still span two independently mutable authorities. Recovery, relocation,
and replacement would require a distributed commit protocol without improving
the user-visible model.

## Schema v20

Schema v20 adds a separate ownership table rather than overloading the durable
lane initialization phase:

```sql
CREATE TABLE lane_initialization_owners (
    initialization_id TEXT PRIMARY KEY
        REFERENCES lane_initializations(initialization_id) ON DELETE CASCADE,
    owner_token TEXT NOT NULL CHECK (
        length(owner_token) = 64 AND owner_token NOT GLOB '*[^0-9a-f]*'),
    owner_generation INTEGER NOT NULL CHECK (owner_generation > 0),
    owner_pid INTEGER NOT NULL CHECK (owner_pid > 0),
    owner_process_start_identity TEXT NOT NULL
        CHECK (length(owner_process_start_identity) > 0),
    acquired_at INTEGER NOT NULL,
    heartbeat_at INTEGER NOT NULL
);
```

`owner_token` is a cryptographically random 256-bit value. Generation starts at
one and increases on every takeover. The pair `(owner_token,
owner_generation)` is the fencing identity.

The v19-to-v20 migration creates this table and its validation contract in one
transaction. Existing initialization rows receive no owner. Existing terminal
rows remain replayable; existing nonterminal rows are claimable. A v18 database
continues through the existing v18-to-v19 migration and then v19-to-v20 before
normal opening completes. Migration failure rolls back the entire schema
transition.

## Claim protocol

The caller first resolves the canonical initialization ID and request
fingerprint, then starts `BEGIN IMMEDIATE`:

1. If no lane initialization row exists, insert the `reserved` row and its
   generation-one owner row atomically.
2. If the request fingerprint conflicts, return the existing stable conflict
   without changing either table.
3. If the initialization is terminal (`observer_ready` or
   `repair_required`), commit the read transaction and replay the durable
   result.
4. If a matching nonterminal row has no owner, insert a fresh generation-one
   owner. The random token still fences any previously released owner.
5. If it has an owner whose PID and process-start identity still match a live
   process, commit and enter the contender wait path.
6. If the recorded owner is provably dead or has a mismatched process-start
   identity, replace it with a fresh token and incremented generation using a
   compare-and-swap predicate over the complete old owner identity.
7. If liveness cannot be determined safely, treat the owner as live. Time alone
   never authorizes takeover.

Only short claim, heartbeat, phase-transition, release, and replay transactions
hold SQLite write authority. Materialization, observer startup, and other slow
work happen outside a database transaction.

## Owner fencing and lifecycle

Every phase-changing write includes an `EXISTS` predicate for the exact owner
token and generation in the same transaction. A zero-row update is a stable
lost-ownership error and the stale worker must stop before any later
publication.

The owner updates `heartbeat_at` during every durable transition and before and
after each long phase. Heartbeat age is diagnostic only; it never permits
takeover while the process identity is live.

Terminal transitions atomically delete the owner row. A pre-association error
conditionally deletes only the caller's exact owner row, leaving the durable
phase resumable. A post-association failure atomically records
`repair_required` when possible and deletes the owner. If the repair-state
write fails, the existing `COMMITTED_REPAIR_REQUIRED` envelope reports the
actual last-known durable phase; release is best-effort and never changes the
committed result contract.

Crash recovery needs no filesystem cleanup. The next caller observes the stale
owner row, proves its process identity dead, increments the generation, and
resumes from the durable phase. A stale worker cannot commit after takeover
because its token/generation fence no longer matches.

## Contender wait and replay

A contender never materializes or publishes. It polls the single initialization
and owner rows with exponential backoff from 10 ms to 250 ms, adding bounded
jitter to avoid synchronized wakeups.

- Terminal state is replayed immediately.
- A missing/dead owner triggers a new claim attempt.
- A live owner continues to be observed.
- After 30 minutes, a still-live owner produces a stable
  `LANE_INITIALIZATION_IN_PROGRESS` conflict containing the lane,
  initialization ID, owner PID, durable phase, and retry command. The timeout
  does not revoke ownership.

Tests use an injected clock/backoff policy and do not sleep for production
durations.

## Filesystem and relocation behavior

Lane initialization coordination creates no lock, anchor, identity, or
candidate files. The `lane-initialization-locks` and
`lane-initialization-publication.anchor` code paths are removed. Because this
filesystem mechanism has never reached `main`, Trail does not migrate or trust
its artifacts; any local artifacts left by development builds are inert.

A quiescent workspace copy is portable because terminal initializations have no
owner rows. Copying while an initialization is active remains unsupported and
is documented as non-quiescent. A copied stale active row is recoverable only
when the destination can prove the recorded process identity is not its live
owner; otherwise it fails closed with the in-progress contract.

## Scale behavior

Different initialization IDs contend only for short SQLite write transactions.
No lock is held during APFS clone, sparse materialization, environment setup,
or observer startup. The 64- and 128-lane scale gates must show:

- one owner row at most per nonterminal initialization;
- no owner rows for terminal initializations;
- one lane/ref/spawn event per initialization;
- identical-request replay without duplicate filesystem publication;
- no new `.lock`, `.anchor`, `.identity`, or candidate resources; and
- bounded wait/backoff state independent of repository file count.

## Error handling

- Fingerprint mismatch retains the existing lane initialization conflict.
- Live/indeterminate owner timeout returns
  `LANE_INITIALIZATION_IN_PROGRESS`; it never steals ownership.
- Lost owner fence returns a stable internal retry/replay path and cannot be
  exposed as an ordinary uniqueness or SQLite error.
- Post-association failures retain `COMMITTED_REPAIR_REQUIRED` and actual
  durable phase semantics.
- Corrupt owner rows fail schema/record validation closed and produce repair or
  reinitialization guidance; they are never silently deleted.

## Verification contract

Implementation must use red-green TDD and cover:

1. Sixteen or more same-request threads and independent processes return one
   durable result with one lane/ref/event.
2. Sixty-four unrelated initializations remain concurrent and complete without
   a global long-held lock.
3. Crashes at every durable phase leave a claimable or replayable state.
4. A stale token/generation cannot transition phase after takeover.
5. A live owner is never stolen solely because heartbeat or wall-clock time
   advances.
6. PID reuse is rejected by process-start identity.
7. Owner-release failure preserves the correct committed outcome.
8. Schema v19-to-v20 migration is atomic, idempotent through reopen, and exact
   under schema-shape validation.
9. Quiescent workspace copy replays successfully without filesystem repair.
10. Unix and Windows compile the same SQL state machine; platform-specific code
    is limited to existing process-liveness probes.
11. The real-repository resource inventory remains unchanged and still detects
    a synthetic genuine runtime lock leak.
12. Formatting, strict Trail all-target Clippy, serial library tests, lane
    initialization integration tests, changed-path producer tests, and the
    blocking large-repository lane/handoff gates pass or are reported with exact
    evidence and no hidden retries.

## Non-goals

- General distributed coordination across hosts sharing one SQLite database.
- Taking ownership from a process that is still provably alive.
- Migrating development-only filesystem authority artifacts.
- Changing lane merge semantics, Git export semantics, or environment adapter
  behavior outside what the coordination tests require.

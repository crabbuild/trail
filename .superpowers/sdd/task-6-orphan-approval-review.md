### Spec Compliance

- Issue in `recovery.rs:763-777` and `recovery.rs:779-854`.
- Cannot verify tests/native/power-loss behavior from diff.

### Strengths

- No production pathname removal; pre-existing namespaces retained/rejected; substitutions preserved.
- Earlier protections remain intact.

### Issues

#### Critical

None.

#### Important

1. Quarantine directories are created before deletion rows/scope retirement are durable. Crash/error after mkdir but before durable row/WAL leaves deterministic Trail-owned orphan; retry treats it foreign and permanently rejects retirement. Fix: durably journal allocation ownership before filesystem creation with recoverable allocation state + unique attempt identity; never adopt unjournaled namespace; retain ambiguous abandoned namespaces while safely allocating a new journaled one. Add kill/error coverage after mkdir, before row insertion, between segments, before commit, before WAL barrier.

#### Minor

None.

### Assessment

Task quality: Needs fixes.

### Orphan allocation review fix

- Added a nonce-keyed `changed_path_segment_quarantine_allocations` journal with explicit
  `allocating`, `allocated`, `bound`, and `abandoned` states. Every segment allocation is
  committed and WAL-barriered before its quarantine directory is created.
- Recovery resumes only an absent journaled `allocating` namespace or an exact-identity
  `allocated` namespace. A present namespace without a published identity, a missing or
  mismatched allocated namespace, and a create collision are retained and durably marked
  `abandoned`; a fresh nonce and directory leaf are then allocated. Pre-existing legacy
  namespaces are likewise retained and recorded as abandoned instead of being adopted.
- The final deletion transaction binds each prepared deletion row to its exact allocation
  nonce and filesystem identities. Retired-scope retry validates the allocation/deletion
  join before reopening deletion authority.
- Extended the real SIGKILL matrix to two segments and covered the allocation journal
  barrier, post-mkdir ambiguity, identity publication, the between-segment boundary,
  completed allocation setup, pre-commit, post-commit, post-WAL, and the existing
  quarantine/quiescence boundaries. The journal-barrier case injects foreign namespaces
  and verifies they remain present and audited while fresh attempts complete.
- Verification: exact schema suite passed (8 tests), changed-path recovery integration
  suite passed (29 tests), and the two-segment SIGKILL matrix passed all phases.

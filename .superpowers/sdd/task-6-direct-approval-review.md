### Spec Compliance

- Critical WAL-blind preflight and three Important retirement/validation gaps.
- Cannot verify reported native/crash results from diff.

### Issues

#### Critical

1. Existing-schema preflight opens main DB with immutable=1, ignores WAL, then opens originals mutably. Crash-retained WAL can contain unvalidated malformed generation. Fix: under writer exclusion, make a byte-preserving consistent snapshot of main+WAL/SHM; allow recovery only on snapshot; validate snapshot; retain exclusion through mutable original open. Add persistent-WAL malformed schema/row rejection with original bytes unchanged.

#### Important

2. Retirement moves/authenticates segments before revoke/retire. Retained writer FD can append after auth/quiesced. Fix: durably revoke/retire first, establish explicit writer quiescence/fence proving FD closed, then move/auth/bind. Add retained-FD concurrent append regression.

3. Semantic query must span scope→segment→allocation→deletion, require exact retired coverage/state/leaf/owner/sequence/durable/hash/source metadata. Add missing-child and cross-wire fixtures.

4. `changed_path_policy_dependencies` missing structural/FK/row checks. Include exact validation/global FK check and byte-invariant orphan fixture.

### Assessment

Task quality: Needs fixes.

### Direct approval fixes

- Replaced immutable-main preflight with a WAL-aware private snapshot validated under a stable OS shared exclusion. The generation token covers main, WAL, SHM, and rollback journal identity/size/timestamps; the durable main+WAL image is copied while SHM is regenerated because it is volatile shared-memory state. The exact token is retained through mutable connection/store construction and rechecked at handoff. Generation-keyed in-process singleflight and a cross-process leader lock prevent duplicate validation and propagate leader failures; 100 unchanged-generation opens perform one validation within the five-second budget.
- All workspace writers now acquire the stable exclusion exclusively. Open-time recovery is performed under that writer exclusion, and the final Trail SQLite connection disables close-time checkpointing and drops after cloned Prolly connections so no last-connection WAL checkpoint escapes the exclusion. Concurrent two-process ACP relay passed 10 consecutive focused runs.
- Retirement now durably revokes the owner and marks the scope retiring before it waits for an exclusive segment-file quiescence lock, then authenticates/moves/binds the retired generation. A retained-writer regression proves an open FD cannot append into the retired generation.
- Expanded retirement validation across scope, segment, allocation, and deletion metadata, requiring exact child coverage plus matching state, leaf, owner, sequence, durable offsets, hashes, and source metadata. Missing-child, orphan-allocation, and cross-wired fixtures are rejected.
- Added exact `changed_path_policy_dependencies` structure, foreign-key, and canonical-row validation, including a byte-invariant orphan fixture.
- Fresh verification: `cargo test -p trail --lib -q` passed 572 tests with 1 ignored and 0 failures; schema hard-cutover passed 13/13 in debug and release; changed-path recovery passed 30/30; debug and release `cargo check -p trail` passed. Full e2e passed 204/205; the sole failure is the pre-existing stale assertion for the newer-schema error wording, while the concurrent-relay and workspace-lock regressions pass.

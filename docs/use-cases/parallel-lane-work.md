# Use Case: Parallel Lane Work

Use separate lane branches and advisory leases when multiple agents may work on the same workspace.

## Spawn Lanes

```sh
crabdb lane spawn docs-bot --from main
crabdb lane spawn tests-bot --from main
```

## Claim Paths

```sh
crabdb lane claim docs-bot README.md --ttl-secs 600
crabdb lease list
```

`lane claim` creates a best-effort write lease. Conflicting claims return conflict information instead of silently taking ownership.

Claims do not hard-prevent writes. Treat them as coordination metadata, then use
readiness, diff review, conflict sets, and the merge queue as the authoritative
safety checks before accepting work.

## Work Separately

Lanes can apply patches to their own branch-backed refs:

```sh
crabdb lane apply-patch docs-bot --patch docs.patch
crabdb lane apply-patch tests-bot --patch tests.patch
```

Or use materialized workdirs:

```sh
crabdb lane spawn docs-bot --materialize=true --paths docs
crabdb lane workdir docs-bot
crabdb lane record docs-bot -m "record docs workdir"
```

## Merge Safely

```sh
crabdb merge-queue add docs-bot --into main
crabdb merge-queue add tests-bot --into main
crabdb merge-queue run
```

## Recover a Bad Attempt

If an agent goes sideways, rewind it to a known-good change or root while keeping
the failed attempt inspectable:

```sh
crabdb lane rewind docs-bot --to <known-good-change> --record-current --sync-workdir
```

The preserved branch keeps the bad attempt available for review without moving
the shared target branch.

## Code Facts Used

- Lane lifecycle: `crates/crabdb/src/db/lane/lifecycle.rs`
- Leases: `crates/crabdb/src/db/lane/leases.rs`
- Rewind: `crates/crabdb/src/db/lane/rewind.rs`
- Tests: `advisory_leases_coordinate_lane_paths`, `lane_claims_are_soft_leases_across_cli_api_and_mcp`

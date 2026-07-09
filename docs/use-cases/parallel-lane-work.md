# Use Case: Parallel Lane Work

Use separate lane branches and advisory leases when multiple agents may work on the same workspace.

## Spawn Lanes

```sh
trail lane spawn docs-bot --from main
trail lane spawn tests-bot --from main
```

## Claim Paths

```sh
trail lane claim docs-bot README.md --ttl-secs 600
trail lease list
```

`lane claim` creates a best-effort write lease. Conflicting claims return conflict information instead of silently taking ownership.

Claims do not hard-prevent writes. Treat them as coordination metadata, then use
readiness, diff review, conflict sets, and the merge queue as the authoritative
safety checks before accepting work.

## Work Separately

Lanes can apply patches to their own branch-backed refs:

```sh
trail lane apply-patch docs-bot --patch docs.patch
trail lane apply-patch tests-bot --patch tests.patch
```

Or use materialized workdirs:

```sh
trail lane spawn docs-bot --materialize=true --paths docs
trail lane workdir docs-bot
trail lane record docs-bot -m "record docs workdir"
```

## Merge Safely

```sh
trail merge-queue add docs-bot --into main
trail merge-queue add tests-bot --into main
trail merge-queue run
```

## Recover a Bad Attempt

If an agent goes sideways, rewind it to a known-good change or root while keeping
the failed attempt inspectable:

```sh
trail lane rewind docs-bot --to <known-good-change> --record-current --sync-workdir
```

The preserved branch keeps the bad attempt available for review without moving
the shared target branch.

## Code Facts Used

- Lane lifecycle: `crates/trail/src/db/lane/lifecycle.rs`
- Leases: `crates/trail/src/db/lane/leases.rs`
- Rewind: `crates/trail/src/db/lane/rewind.rs`
- Tests: `advisory_leases_coordinate_lane_paths`, `lane_claims_are_soft_leases_across_cli_api_and_mcp`

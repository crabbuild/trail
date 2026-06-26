# Use Case: Parallel Agent Work

Use separate agent branches and advisory leases when multiple agents may work on the same workspace.

## Spawn Agents

```sh
crabdb agent spawn docs-bot --from main
crabdb agent spawn tests-bot --from main
```

## Claim Paths

```sh
crabdb agent claim docs-bot README.md --ttl-secs 600
crabdb lease list
```

`agent claim` creates a best-effort write lease. Conflicting claims return conflict information instead of silently taking ownership.

## Work Separately

Agents can apply patches to their own branches:

```sh
crabdb agent apply-patch docs-bot --patch docs.patch
crabdb agent apply-patch tests-bot --patch tests.patch
```

Or use materialized workdirs:

```sh
crabdb agent spawn docs-bot --materialize=true --paths docs
crabdb agent workdir docs-bot
crabdb agent record docs-bot -m "record docs workdir"
```

## Merge Safely

```sh
crabdb merge-queue add docs-bot --into main
crabdb merge-queue add tests-bot --into main
crabdb merge-queue run
```

## Code Facts Used

- Agent lifecycle: `crates/crabdb/src/db/agent/lifecycle.rs`
- Leases: `crates/crabdb/src/db/agent/leases.rs`
- Tests: `advisory_leases_coordinate_agent_paths`, `agent_claims_are_soft_leases_across_cli_api_and_mcp`


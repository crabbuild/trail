# Spawn and Materialize Workdirs

Agent branches can stay virtual or be materialized into a filesystem workdir.

## Spawn Without Materialization

```sh
crabdb agent spawn doc-bot --from main --no-materialize
```

The default is controlled by `agent.default_materialize`, and large roots default agents to no materialization.

## Spawn With Materialization

```sh
crabdb agent spawn doc-bot --from main --materialize=true
```

Use a custom workdir:

```sh
crabdb agent spawn doc-bot --from main --materialize=true --workdir /tmp/doc-bot
```

Custom workdirs must be empty or absent and cannot be symlinks.

## Sparse Materialization

```sh
crabdb agent spawn doc-bot --from main --materialize=true --paths docs README.md
```

Use `--include-neighbors` when selected files should include nearby context.

Sparse workdirs contain CrabDB manifest files under their own `.crabdb` directory so CrabDB can track what was materialized.

## Read and Hydrate Files

```sh
crabdb agent read doc-bot docs/README.md
crabdb agent read doc-bot docs/README.md --no-hydrate
crabdb agent read doc-bot docs/README.md --hydrate --include-neighbors
```

Reads hydrate sparse workdirs by default unless `--no-hydrate` is passed.

## Sync a Workdir

```sh
crabdb agent sync-workdir doc-bot
crabdb agent sync-workdir doc-bot --paths docs --include-neighbors
crabdb agent sync-workdir doc-bot --force
```

Dirty workdirs require recording or force refresh.

## Code Facts Used

- Spawn/read/sync args: `crates/crabdb/src/cli/command/agent_args.rs`
- Workdir lifecycle: `crates/crabdb/src/db/agent/lifecycle.rs`, `crates/crabdb/src/db/agent/workdir`
- Tests: `agent_spawn_supports_custom_and_configured_workdirs`, `large_roots_default_agents_to_no_materialize`, `agent_workdir_sync_refuses_dirty_and_force_refreshes`


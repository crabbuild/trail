# Spawn and Materialize Workdirs

Lane branches can stay virtual or be materialized into a filesystem workdir.

## Spawn Without Materialization

```sh
crabdb lane spawn doc-bot --from main --no-materialize
```

The default is controlled by `lane.default_materialize`, and large roots default lanes to no materialization.

## Spawn With Materialization

```sh
crabdb lane spawn doc-bot --from main --materialize=true
```

Use a custom workdir:

```sh
crabdb lane spawn doc-bot --from main --materialize=true --workdir /tmp/doc-bot
```

Custom workdirs must be empty or absent and cannot be symlinks.

## Sparse Materialization

```sh
crabdb lane spawn doc-bot --from main --materialize=true --paths docs README.md
```

Use `--include-neighbors` when selected files should include nearby context.

Sparse workdirs contain CrabDB manifest files under their own `.crabdb` directory so CrabDB can track what was materialized.

Sparse hydration writes only missing or explicitly forced paths. When the live
workspace already has matching file bytes and the filesystem supports
copy-on-write file cloning, CrabDB clones that file into the lane workdir;
otherwise it hydrates the path from CrabDB objects.

## Read and Hydrate Files

```sh
crabdb lane read doc-bot docs/README.md
crabdb lane read doc-bot docs/README.md --no-hydrate
crabdb lane read doc-bot docs/README.md --hydrate --include-neighbors
```

Reads hydrate sparse workdirs by default unless `--no-hydrate` is passed.

## Sync a Workdir

```sh
crabdb lane sync-workdir doc-bot
crabdb lane sync-workdir doc-bot --paths docs --include-neighbors
crabdb lane sync-workdir doc-bot --force
```

Dirty workdirs require recording or force refresh.

## Code Facts Used

- Spawn/read/sync args: `crates/crabdb/src/cli/command/lane_args.rs`
- Workdir lifecycle: `crates/crabdb/src/db/lane/lifecycle.rs`, `crates/crabdb/src/db/lane/workdir`
- Tests: `lane_spawn_supports_custom_and_configured_workdirs`, `large_roots_default_lanes_to_no_materialize`, `lane_workdir_sync_refuses_dirty_and_force_refreshes`

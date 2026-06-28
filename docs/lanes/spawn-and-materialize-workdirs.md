# Spawn and Materialize Workdirs

Lane branches can stay virtual or be materialized into a filesystem workdir.
New commands and JSON reports expose this as `workdir_mode`:

- `virtual`: no workdir; branch state changes through patches or API calls.
- `sparse`: materialize selected paths and hydrate more paths explicitly.
- `full-cow`: materialize the full root, using filesystem clone COW when safe.
- `overlay-cow`: reserved for a future transparent write-time COW backend.

Current COW means safe file clone during materialization or hydration. It does
not intercept arbitrary writes to unhydrated paths.

## Spawn Without Materialization

```sh
crabdb lane spawn doc-bot --from main --no-materialize
crabdb lane spawn doc-bot --from main --workdir-mode virtual
```

The default is controlled by `lane.default_materialize`, and large roots default lanes to no materialization.

## Spawn With Materialization

```sh
crabdb lane spawn doc-bot --from main --materialize=true
crabdb lane spawn doc-bot --from main --workdir-mode full-cow
```

Use a custom workdir:

```sh
crabdb lane spawn doc-bot --from main --materialize=true --workdir /tmp/doc-bot
```

Custom workdirs must be empty or absent and cannot be symlinks.

## Sparse Materialization

```sh
crabdb lane spawn doc-bot --from main --materialize=true --paths docs README.md
crabdb lane spawn doc-bot --from main --workdir-mode sparse --paths docs README.md
```

Use `--include-neighbors` when selected files should include nearby context.

Sparse workdirs contain CrabDB manifest files under their own `.crabdb`
directory so CrabDB can track what was materialized. CrabDB also stores the
sparse path boundary in lane metadata, so `lane.enforce_sparse_paths=true` can
still reject writes outside the sparse selection if the workdir sparse manifest
is missing and can recreate the manifest after a valid sparse update.

Sparse hydration writes only missing or explicitly forced paths. When the live
workspace already has matching file bytes and the filesystem supports
copy-on-write file cloning, CrabDB clones that file into the lane workdir;
otherwise it hydrates the path from CrabDB objects.

## Read and Hydrate Files

```sh
crabdb lane read doc-bot docs/README.md
crabdb lane read doc-bot docs/README.md --no-hydrate
crabdb lane read doc-bot docs/README.md --hydrate --include-neighbors
crabdb lane hydrate doc-bot docs/README.md --include-neighbors
```

Reads hydrate sparse workdirs by default unless `--no-hydrate` is passed.
Use `lane hydrate` when a tool is about to edit paths through the filesystem.

## Sync a Workdir

```sh
crabdb lane sync-workdir doc-bot
crabdb lane sync-workdir doc-bot --paths docs --include-neighbors
crabdb lane sync-workdir doc-bot --force
```

Dirty workdirs require recording or force refresh.

Full workdir refreshes materialize into a hidden sibling staging directory,
write and verify the workdir manifest there, then replace the visible workdir.
If staging fails, the existing visible workdir is left in place.

When `--force` overwrites dirty materialized workdir content or replaces a
non-directory file at the lane workdir path, CrabDB first saves recoverable
regular files under `.crabdb/lane-workdir-rescue/...` and returns that path as
`rescue_workdir`. The rescue directory also contains a `manifest.json` with the
dirty path summary or replaced-path summary and any paths that could not be
copied, such as deleted files.

## Preview a Record

```sh
crabdb lane record doc-bot --preview
```

Record preview does not advance the lane. It reports changed paths, ignored
paths, risky workdir entries such as nested `.git`, nested `.crabdb`, symlinks,
hardlinks, or external mounts, oversized changed files, and whether current lane
policy would allow the record.

## Code Facts Used

- Spawn/read/sync args: `crates/crabdb/src/cli/command/lane_args.rs`
- Workdir lifecycle: `crates/crabdb/src/db/lane/lifecycle.rs`, `crates/crabdb/src/db/lane/workdir`
- Tests: `lane_spawn_supports_custom_and_configured_workdirs`, `large_roots_default_lanes_to_no_materialize`, `lane_workdir_sync_refuses_dirty_and_force_refreshes`

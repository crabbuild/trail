# Spawn and Materialize Workdirs

Lane branches can stay virtual or be materialized into a filesystem workdir.
New commands and JSON reports expose this as `workdir_mode`:

- `virtual`: no workdir; branch state changes through patches or API calls.
- `sparse`: materialize selected paths and hydrate more paths explicitly.
- `full-cow`: materialize the full root, using filesystem clone COW when safe.
- `fuse-cow`: create an empty mountpoint and use a FUSE overlay view at
  runtime; reads come from Trail objects and writes land in a per-lane upper
  directory.
- `nfs-cow`: on macOS, expose the same lower/upper model through a loopback
  NFSv3 mount without macFUSE.

For `full-cow` and `sparse`, COW means safe file clone during materialization or
hydration. It does not intercept arbitrary writes to unhydrated paths.

FUSE COW is different: the visible workdir is mounted while a terminal agent
is running. The mountpoint starts empty on disk, the writable upper layer lives
under `.trail/views/<view-id>/source-upper`, and Trail records through the mounted
view before unmounting. On macOS this requires macFUSE; on Linux it requires
FUSE access such as `/dev/fuse`.

NFS COW stores writes under `.trail/views/<view-id>/source-upper`, filters macOS
metadata sidecars, records the upper-layer delta, and unmounts automatically.

## Spawn Without Materialization

```sh
trail lane spawn doc-bot --from main --no-materialize
trail lane spawn doc-bot --from main --workdir-mode virtual
```

The default is controlled by `lane.default_materialize`, and large roots default lanes to no materialization.

## Spawn With Materialization

```sh
trail lane spawn doc-bot --from main --materialize=true
trail lane spawn doc-bot --from main --workdir-mode full-cow
```

Use a custom workdir:

```sh
trail lane spawn doc-bot --from main --materialize=true --workdir /tmp/doc-bot
```

Custom workdirs must be empty or absent and cannot be symlinks.

## FUSE COW For Terminal Agents

```sh
trail agent start --provider codex --workdir-mode fuse-cow
trail agent start --provider custom --workdir-mode fuse-cow -- my-agent --flag
trail agent start --provider codex --workdir-mode nfs-cow
```

`fuse-cow` lets a terminal agent see a normal filesystem tree without first
copying every file into the lane workdir. Lower files are served from Trail
objects. The first write, create, rename, or delete for a path is captured in
the lane upper layer and then recorded as the agent checkpoint.

The lane mount exists only for the duration of the terminal run. If mounting
fails, Trail reports the FUSE setup error instead of silently falling back to a
full copy.

On macOS with Docker Desktop, verify the Linux path with:

```sh
scripts/verify-linux-fuse-cow-docker.sh
```

The script runs a privileged Linux container with `/dev/fuse`, builds Trail,
starts a terminal task with `--workdir-mode fuse-cow`, and asserts that the
agent saw a FUSE filesystem and recorded modified, added, and deleted paths.

## Sparse Materialization

```sh
trail lane spawn doc-bot --from main --materialize=true --paths docs README.md
trail lane spawn doc-bot --from main --workdir-mode sparse --paths docs README.md
```

Use `--include-neighbors` when selected files should include nearby context.

Sparse workdirs contain Trail manifest files under their own `.trail`
directory so Trail can track what was materialized. Trail also stores the
sparse path boundary in lane metadata, so `lane.enforce_sparse_paths=true` can
still reject writes outside the sparse selection if the workdir sparse manifest
is missing and can recreate the manifest after a valid sparse update.

Sparse hydration writes only missing or explicitly forced paths. When the live
workspace already has matching file bytes and the filesystem supports
copy-on-write file cloning, Trail clones that file into the lane workdir;
otherwise it hydrates the path from Trail objects.

## Read and Hydrate Files

```sh
trail lane read doc-bot docs/README.md
trail lane read doc-bot docs/README.md --no-hydrate
trail lane read doc-bot docs/README.md --hydrate --include-neighbors
trail lane hydrate doc-bot docs/README.md --include-neighbors
```

Reads hydrate sparse workdirs by default unless `--no-hydrate` is passed.
Use `lane hydrate` when a tool is about to edit paths through the filesystem.

## Sync a Workdir

```sh
trail lane sync-workdir doc-bot
trail lane sync-workdir doc-bot --paths docs --include-neighbors
trail lane sync-workdir doc-bot --force
```

Dirty workdirs require recording or force refresh.

Full workdir refreshes materialize into a hidden sibling staging directory,
write and verify the workdir manifest there, then replace the visible workdir.
If staging fails, the existing visible workdir is left in place.

When `--force` overwrites dirty materialized workdir content or replaces a
non-directory file at the lane workdir path, Trail first saves recoverable
regular files under `.trail/lane-workdir-rescue/...` and returns that path as
`rescue_workdir`. The rescue directory also contains a `manifest.json` with the
dirty path summary or replaced-path summary and any paths that could not be
copied, such as deleted files.

## Preview a Record

```sh
trail lane record doc-bot --preview
```

Record preview does not advance the lane. It reports changed paths, ignored
paths, risky workdir entries such as nested `.git`, nested `.trail`, symlinks,
hardlinks, or external mounts, oversized changed files, and whether current lane
policy would allow the record.

## Code Facts Used

- Spawn/read/sync args: `trail/src/cli/command/lane_args.rs`
- Workdir lifecycle: `trail/src/db/lane/lifecycle.rs`, `trail/src/db/lane/workdir`
- Tests: `lane_spawn_supports_custom_and_configured_workdirs`, `large_roots_default_lanes_to_no_materialize`, `lane_workdir_sync_refuses_dirty_and_force_refreshes`

# Git Integration

Git interop is available through initialization, import-update, export, and mapping inspection.

## Import

```sh
trail init --from-git
trail git import-update -m "sync git state"
```

`init --from-git` imports tracked files at initialization. `git import-update` records the current tracked snapshot later.

## Export Patch or Commit

```sh
trail git export main..scratch
trail git export main..scratch --output change.patch
trail git export main..scratch -m "export change"
```

With `-m`, Trail creates a Git commit object and mapping. Without `-m`, it prints or writes a patch.

## Mappings

```sh
trail git mappings --limit 30
```

Mappings connect Git head/dirty state to Trail changes and roots.

## Code Facts Used

- Git args/handler: `trail/src/cli/command/maintenance_args.rs`, `trail/src/cli/command/handler/maintenance.rs`
- Git storage: `trail/src/db/storage/git.rs`
- Reports: `trail/src/model/reports/worktree.rs`


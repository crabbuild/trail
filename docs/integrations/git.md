# Git Integration

Git interop is available through initialization, import-update, export, and mapping inspection.

## Import

```sh
crabdb init --from-git
crabdb git import-update -m "sync git state"
```

`init --from-git` imports tracked files at initialization. `git import-update` records the current tracked snapshot later.

## Export Patch or Commit

```sh
crabdb git export main..scratch
crabdb git export main..scratch --output change.patch
crabdb git export main..scratch -m "export change"
```

With `-m`, CrabDB creates a Git commit object and mapping. Without `-m`, it prints or writes a patch.

## Mappings

```sh
crabdb git mappings --limit 30
```

Mappings connect Git head/dirty state to CrabDB changes and roots.

## Code Facts Used

- Git args/handler: `crates/crabdb/src/cli/command/maintenance_args.rs`, `crates/crabdb/src/cli/command/handler/maintenance.rs`
- Git storage: `crates/crabdb/src/db/storage/git.rs`
- Reports: `crates/crabdb/src/model/reports/worktree.rs`


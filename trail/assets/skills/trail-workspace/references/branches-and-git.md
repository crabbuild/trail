# Branches and Git Handoff

Trail branches are long-lived local code refs. They do not create, checkout, or publish Git branches.

## Preview Trail State Changes

Inspect the current refs and command help before changing branch state. Preview materialization and merges:

```sh
trail checkout <ref> --dry-run
trail merge <source> --into <target> --dry-run
```

Stop on dirty-worktree, stale-state, ambiguity, or conflict reports. Do not invent a resolution or overwrite unrecorded work.

## Cross the Git Boundary Explicitly

```sh
trail git import-update -m "Sync current Git-tracked snapshot"
trail git export main..scratch --output change.patch
trail git mappings --limit 30
```

`trail git export <range> -m <message>` creates a Git commit object and cannot be combined with `--output`. Inspect Git status and the dry-run or patch result before creating or advancing Git history. For a managed Trail agent task, use the `trail-agent-tasks` apply workflow instead.

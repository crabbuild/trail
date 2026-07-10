# Core Workspace Workflows

Use core Trail commands for local operation history outside the high-level agent-task interface.

## Initialize and Inspect

Choose one initial root:

```sh
trail init --from-git
trail init --working-tree
trail init
```

`--from-git` imports Git-tracked paths. `--working-tree` imports visible current files. Plain `init` starts empty. Never reinitialize an existing `.trail` workspace.

Inspect without mutation:

```sh
trail --json status
trail diff --dirty --patch
trail timeline --limit 10
```

Trail discovers `.trail` by walking upward. Use `--workspace <root>` or `TRAIL_WORKSPACE` when commands run from another directory.

## Record Local Work

Review dirty state first, then record one meaningful operation:

```sh
trail status
trail diff --dirty --patch
trail record -m "Describe why this change exists"
```

Prefer selective recording when unrelated edits exist:

```sh
trail record --paths README.md docs -m "Update documentation"
```

Do not absorb unrelated user changes into the same operation. Ignore rules come from `.trailignore`, `.gitignore`, internal protections, and private-path protections. Inspect with `trail ignore list` and `trail ignore check <path>`.

## Query History and Provenance

```sh
trail why README.md:2
trail history README.md
trail timeline --limit 20
trail show <change-id>
trail code-from <message-session-or-lane-id>
```

For precise agent edits and review, include stable identities:

```sh
trail diff --dirty --patch --show-line-ids
```

Use Trail provenance for recorded local operations; use Git blame/log for committed shared history. Explain which layer answered the question.

## Branches, Checkout, and Merge

Trail branches are long-lived local code refs. Lanes are short-lived task containers. Inspect help and current refs before creating, renaming, checking out, or deleting branches.

Preview materialization or merge before changing files or refs:

```sh
trail checkout <ref> --dry-run
trail merge <source> --into <target> --dry-run
```

Never present a Trail branch operation as a Git checkout. Preserve dirty user work; stop when Trail reports a dirty-worktree or conflict blocker.

## Git Interop

Synchronize intentionally:

```sh
trail git import-update -m "Sync current Git-tracked snapshot"
trail git export main..scratch
trail git export main..scratch --output change.patch
trail git mappings --limit 30
```

`trail git export <range> -m <message>` creates a Git commit object and cannot be combined with `--output`. Prefer the high-level `trail agent apply --dry-run`/`apply` flow for agent-task handoff to the current Git branch.

## Maintenance

Start read-only:

```sh
trail doctor
trail fsck
trail gc --dry-run
```

Rebuild derived indexes only when diagnostics indicate it:

```sh
trail index rebuild
trail index rebuild --rich-text
```

Back up before invasive recovery:

```sh
trail backup create /path/to/backup
trail backup verify /path/to/backup
```

Restore, non-dry-run garbage collection, and force/overwrite options require explicit intent and a verified backup.

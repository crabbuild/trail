# CLI Reference: Branching, Merging, and Conflicts

## `checkout`

```text
crabdb checkout <TARGET> [--force] [--record-dirty] [--dry-run] [--workdir <PATH>]
```

Materializes a branch, ref, operation, or root into the workspace or alternate workdir.

## `branch`

```text
crabdb branch
crabdb branch <NAME> [--from <REF>]
crabdb branch --delete <NAME>
crabdb branch --rename <OLD> --to <NEW>
```

## `merge`

```text
crabdb merge <SOURCE> --into <TARGET> [--strategy <STRATEGY>] [--dry-run]
```

Allowed strategies are `conservative`, `line-id-aware`, and `line_id_aware`.

## `merge-agent`

```text
crabdb merge-agent <AGENT> [--into <BRANCH>] [--strategy <STRATEGY>] [--dry-run]
```

Default target is `main`.

## `merge-queue`

```text
crabdb merge-queue add <SOURCE> --into <TARGET> [--priority <N>]
crabdb merge-queue list
crabdb merge-queue run [--limit <N>]
crabdb merge-queue remove <SELECTOR>
```

Default priority is 0.

## `conflicts`

```text
crabdb conflicts list
crabdb conflicts show <CONFLICT_SET_ID>
crabdb conflicts resolve <CONFLICT_SET_ID> --take source
crabdb conflicts resolve <CONFLICT_SET_ID> --take target
crabdb conflicts resolve <CONFLICT_SET_ID> --manual <JSON_FILE>
```

Manual conflict files can be plain text values or objects with `content`, `delete`, and `executable`.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/worktree_args.rs`, `crates/crabdb/src/cli/command/collaboration_args/merge.rs`
- Merge logic: `crates/crabdb/src/db/merge`
- Conflict reports: `crates/crabdb/src/model/reports/merge.rs`


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

## `merge-lane`

```text
crabdb merge-lane <LANE> [--into <BRANCH>] [--strategy <STRATEGY>] [--dry-run]
```

Default target is `main`.

## `merge-queue`

```text
crabdb merge-queue add <SOURCE> --into <TARGET> [--priority <N>]
crabdb merge-queue list
crabdb merge-queue explain <SELECTOR>
crabdb merge-queue run [--limit <N>]
crabdb merge-queue remove <SELECTOR>
```

Default priority is 0.

`merge-queue explain` resolves a queue id, lane name/ref, or branch name/ref and
reports readiness blockers, dry-run merge conflicts, preflight errors, warnings,
and suggested next steps without mutating refs or recording conflict state.

## `conflicts`

```text
crabdb conflicts list
crabdb conflicts show <CONFLICT_SET_ID> [--limit <N>]
crabdb conflicts resolve <CONFLICT_SET_ID> --take source
crabdb conflicts resolve <CONFLICT_SET_ID> --take target
crabdb conflicts resolve <CONFLICT_SET_ID> --manual <JSON_FILE>
```

`conflicts show` includes a deterministic explanation section with source/target operation provenance, best-effort logical line evidence, conservative resolution recommendations, and next steps. The default explanation limit is 50. Each path also includes a conflict class such as `modify/modify`, `delete/modify`, `rename/rename`, or `binary`, so reviewers can triage by risk.

Conflict records store the base, target, and source root snapshots captured when the merge paused. `conflicts show` and `conflicts resolve` use those snapshots rather than re-reading moving refs, so explanations and manual resolutions stay tied to the same merge input.

Manual conflict files can be plain text values or objects with `content`, `delete`, and `executable`.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/worktree_args.rs`, `crates/crabdb/src/cli/command/collaboration_args/merge.rs`
- Merge logic: `crates/crabdb/src/db/merge`
- Conflict reports: `crates/crabdb/src/model/reports/merge.rs`

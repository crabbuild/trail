# CLI Reference: Branching, Merging, and Conflicts

## `checkout`

```text
trail checkout <TARGET> [--force] [--record-dirty] [--dry-run] [--workdir <PATH>]
```

Materializes a branch, ref, operation, or root into the workspace or alternate workdir.

## `branch`

```text
trail branch
trail branch <NAME> [--from <REF>]
trail branch --delete <NAME>
trail branch --rename <OLD> --to <NEW>
```

## `merge`

```text
trail merge <SOURCE> --into <TARGET> [--strategy <STRATEGY>] [--dry-run]
```

Allowed strategies are `conservative`, `line-id-aware`, and `line_id_aware`.

## `lane merge`

```text
trail lane merge <LANE> [--into <BRANCH>] [--strategy <STRATEGY>] [--dry-run] [--direct]
```

Default target is `main`. Non-dry-run direct merges into the workspace default
branch are rejected by default so shared targets flow through
`lane merge-queue`.
Use `--dry-run` to preview or `--direct` for an explicit one-off immediate
merge.

## `lane merge-queue`

```text
trail lane merge-queue add <LANE> --into <TARGET> [--priority <N>]
trail lane merge-queue list
trail lane merge-queue explain <SELECTOR>
trail lane merge-queue run [--limit <N>]
trail lane merge-queue remove <SELECTOR>
```

Default priority is 0.

`lane merge-queue explain` resolves a queue id, lane id, or lane name and
reports readiness blockers, dry-run merge conflicts, preflight errors, warnings,
and suggested next steps without mutating refs or recording conflict state.

## `conflicts`

```text
trail conflicts list
trail conflicts show <CONFLICT_SET_ID> [--limit <N>]
trail conflicts resolve <CONFLICT_SET_ID> --take source
trail conflicts resolve <CONFLICT_SET_ID> --take target
trail conflicts resolve <CONFLICT_SET_ID> --manual <JSON_FILE>
```

`conflicts show` includes a deterministic explanation section with source/target operation provenance, best-effort logical line evidence, conservative resolution recommendations, and next steps. The default explanation limit is 50. Each path also includes a conflict class such as `modify/modify`, `delete/modify`, `rename/modify`, `binary`, `mode`, or `same_insertion_gap`, so reviewers can triage by risk.

Conflict records store the base, target, and source root snapshots captured when the merge paused. `conflicts show` and `conflicts resolve` use those snapshots for conflict content, so explanations and manual resolutions stay tied to the same merge input even if the source lane advances. If the target ref moves, resolution stops with a stale-target error instead of overwriting newer target work; rerun the merge after reviewing the new target head.

Manual conflict files can be plain text values or objects with `content`,
`delete`, and `executable`. Unknown keys in manual resolution objects are
rejected rather than ignored.

## Code Facts Used

- Args: `trail/src/cli/command/worktree_args.rs`, `trail/src/cli/command/lane_args.rs`
- Merge logic: `trail/src/db/merge`
- Conflict reports: `trail/src/model/reports/merge.rs`

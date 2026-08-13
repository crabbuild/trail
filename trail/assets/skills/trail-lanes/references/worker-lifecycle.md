# Lane Worker Lifecycle

Work only in the assigned lane. A lane is a local Trail ref and isolated work container, not a Git branch.

## Orient

When Trail launched the current process, inspect the assigned identifiers and state:

```sh
trail --workspace "$TRAIL_WORKSPACE" --format json lane status "$TRAIL_LANE"
trail --workspace "$TRAIL_WORKSPACE" --format json env status "$TRAIL_LANE"
```

If `TRAIL_VIEW` is set, the environment and workdir are already mounted. Use the current directory and normal project commands. Do not call `env sync`, `lane mount`, or nested `lane exec` from that active view.

When working in a non-mounted materialized lane, use `trail lane workdir <lane>` to locate it. Keep all file edits inside that workdir. For sparse lanes, hydrate a path before editing it:

```sh
trail --workspace <root> lane hydrate <lane> path/to/file
```

## Coordinate Scope

Honor the assigned path scope and active claims. If new work crosses another lane's scope, stop and ask the coordinator to reassign or sequence it. Do not use an ignored path, stale patch, or force flag to cross a coordination boundary.

For sensitive structured edits in a virtual lane, use a patch with the current `base_change`, stable `line_id`, and `expected_text`. Let Trail reject stale or unsafe edits rather than setting `allow_stale` or `allow_ignored` automatically.

## Edit and Check

Make the smallest coherent source change. In an already mounted managed lane, run checks normally. From the coordinator workspace, commands can be run with the exact lane environment through:

```sh
trail lane exec <lane> -- <command>
trail lane test <lane> --suite <suite> -- <test-command>
trail lane eval <lane> --suite <suite> -- <eval-command>
```

`lane test` and `lane eval` create durable gate evidence. Do not claim a gate passed from an ordinary command transcript alone.

## Record Materialized Work

Mounted Trail agent runs checkpoint their source changes through the owning workflow. For an ordinary materialized lane edited by an external process, preview and record from the original workspace:

```sh
trail lane record <lane> --preview
trail lane record <lane> -m "Describe the bounded change"
```

Never force-sync a dirty workdir before its edits are recorded or deliberately rescued.

## Review and Hand Off

Before declaring completion:

```sh
trail lane diff <lane> --patch
trail lane review <lane>
trail lane readiness <lane>
trail lane handoff <lane>
```

Readiness may block on dirty work, conflicts, approvals, stale environments, or missing/failing test and eval suites. Report the exact blocker and remediation; do not merge or bypass it.

Return a handoff containing:

- lane name and assigned scope;
- changed paths and the intent of the change;
- tests/evals run and whether Trail recorded them as gates;
- open conflicts, approvals, environment staleness, or other blockers;
- exact safe next command, usually a missing gate, readiness recheck, or merge dry-run.

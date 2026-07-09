# Selectors and Ref-Like Inputs

Several commands accept selectors: path selectors, line selectors, object IDs, operation IDs, root IDs, branch names, and lane names.

## Path and Line Selectors

Use `path:line` for current file-line provenance:

```sh
trail why README.md:2
```

Use a stable line ID when available:

```sh
trail why --line-id <change-id>:<local-seq>
```

Use `history` with a path, file ID, or line ID:

```sh
trail history README.md
trail history --file-id <file-id>
trail history --line-id <line-id>
```

## Ref-Like Selectors

Commands such as `checkout`, `diff`, `merge`, `timeline`, and `show` resolve values through the ref and object stores. Depending on the command, a selector may be:

- A branch name.
- A full ref such as `refs/branches/main`.
- A lane name or lane ref.
- A change ID.
- A root ID.
- A message ID or object ID for `show`.

## Ranges

`diff` supports a positional range:

```sh
trail diff main..scratch --patch
```

It also supports root ranges:

```sh
trail diff --root <left-root>..<right-root>
```

Daemon-backed `diff` enforces exactly one of positional range, `--root`, or `--dirty`.

## Code Facts Used

- Inspect args: `crates/trail/src/cli/command/inspect_args.rs`
- Diff args and daemon validation: `crates/trail/src/cli/command/worktree_args.rs`, `crates/trail/src/cli/command/handler/daemon_rpc.rs`
- Ref resolution: `crates/trail/src/db/storage/refs.rs`
- Tests: `refish_aliases_accept_branch_lane_and_root_selectors`, `timeline_branch_scope_accepts_command_flag_and_ref_aliases`

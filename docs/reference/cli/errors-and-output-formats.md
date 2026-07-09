# CLI Reference: Errors and Output Formats

## Output Formats

Human output is the default:

```sh
trail status
```

JSON output:

```sh
trail --json status
trail --format json status
```

NDJSON is accepted by the global formatter and currently used by `index watch`:

```sh
trail --format ndjson index watch --iterations 3
```

## Error Shape

When JSON errors are enabled:

```json
{
  "error": {
    "code": "INVALID_INPUT",
    "message": "invalid input: ...",
    "exit_code": 2
  }
}
```

## Exit Codes

| Code | Error category |
| ---: | --- |
| 1 | Default/internal categories not otherwise mapped. |
| 2 | Invalid input or workspace exists. |
| 3 | Workspace not found. |
| 4 | Database corrupt. |
| 5 | Dirty worktree. |
| 6 | Merge conflict. |
| 7 | Patch rejected. |
| 8 | Stale branch or workspace locked. |
| 9 | Invalid path. |
| 10 | Git interop error. |
| 11 | Daemon unavailable or unauthorized. |
| 12 | Operation not found. |
| 13 | Ref not found. |
| 14 | Ignored path. |

## Stable Error Codes

Examples include:

- `WORKSPACE_NOT_FOUND`
- `WORKSPACE_EXISTS`
- `INVALID_PATH`
- `IGNORED_PATH`
- `REF_NOT_FOUND`
- `OPERATION_NOT_FOUND`
- `ROOT_NOT_FOUND`
- `OBJECT_NOT_FOUND`
- `DIRTY_WORKTREE`
- `WORKSPACE_LOCKED`
- `MERGE_CONFLICT`
- `PATCH_REJECTED`
- `STALE_BRANCH`
- `DATABASE_CORRUPT`
- `GIT_ERROR`
- `INVALID_INPUT`
- `DAEMON_UNAVAILABLE`
- `DAEMON_ERROR`

## HTTP Error Mapping

The HTTP daemon maps selected errors to:

- `400`: invalid input, invalid path, ignored path.
- `404`: missing ref, operation, or root.
- `409`: conflict, dirty worktree, patch rejected, stale branch, or workspace lock.
- `500`: other errors.

## Code Facts Used

- Error enum and exit codes: `crates/trail/src/error.rs`
- CLI rendering: `crates/trail/src/cli/command/handler/errors.rs`
- HTTP error responses: `crates/trail/src/server/route/utils.rs`
- Tests: `cli_json_errors_are_machine_readable`


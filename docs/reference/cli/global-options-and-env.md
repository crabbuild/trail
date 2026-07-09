# CLI Global Options and Environment

All commands share the global options defined on the top-level `trail` parser.

## Usage

```text
trail [OPTIONS] <COMMAND>
```

## Global Options

| Option | Purpose |
| --- | --- |
| `--workspace <WORKSPACE>` | Select the workspace root. |
| `--db <DB>` | Select the `.trail` database directory directly. |
| `--branch <BRANCH>` | Select the default branch for commands that use branch context. |
| `--json` | Render structured JSON output and JSON errors. |
| `--quiet` | Suppress human-oriented success output where renderers support it. |
| `--verbose` | Accepted global flag for verbose mode. |
| `--trace` | Accepted global flag for trace mode. |
| `--no-color` | Accepted global flag for color suppression. |
| `--format <FORMAT>` | `human`, `json`, or `ndjson`. |
| `--daemon-url <URL>` | Route supported hot commands to a daemon URL. |
| `--daemon-token <TOKEN>` | Token for daemon-routed commands. |

## Environment Variables

| Variable | Used For |
| --- | --- |
| `TRAIL_WORKSPACE` | Default workspace root. |
| `TRAIL_DIR` | Default `.trail` directory. |
| `TRAIL_BRANCH` | Default branch. |
| `TRAIL_FORMAT` | `human`, `json`, or `ndjson`; `json` also enables JSON errors. |
| `TRAIL_DAEMON_URL` | Default daemon URL. |
| `TRAIL_DAEMON_TOKEN` | Default daemon token and daemon startup token source. |

## Workspace Discovery

If neither `--workspace` nor `--db` is supplied, Trail discovers a workspace by walking upward from the current directory until it finds `.trail`.

If `--db` is supplied without `--workspace`, the parent directory of the database directory is treated as the workspace.

## JSON Errors

Parse errors and runtime errors are rendered as JSON when `--json`, `--format json`, or `TRAIL_FORMAT=json` is used.

```json
{
  "error": {
    "code": "WORKSPACE_NOT_FOUND",
    "message": "workspace not found from /path",
    "exit_code": 3
  }
}
```

## NDJSON

`--format ndjson` is accepted globally. The current command path that emits newline-delimited JSON is `trail index watch`, which prints one `WorktreeIndexReport` per iteration.

## Code Facts Used

- Parser: `crates/trail/src/cli/command.rs`
- Runtime resolution: `crates/trail/src/cli/command/handler/runtime.rs`
- Error rendering: `crates/trail/src/cli/command/handler/errors.rs`
- Tests: `cli_json_errors_are_machine_readable`, `cli_env_defaults_select_workspace_db_branch_and_format`


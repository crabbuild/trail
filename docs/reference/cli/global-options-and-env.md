# CLI Global Options and Environment

All commands share the global options defined on the top-level `crabdb` parser.

## Usage

```text
crabdb [OPTIONS] <COMMAND>
```

## Global Options

| Option | Purpose |
| --- | --- |
| `--workspace <WORKSPACE>` | Select the workspace root. |
| `--db <DB>` | Select the `.crabdb` database directory directly. |
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
| `CRABDB_WORKSPACE` | Default workspace root. |
| `CRABDB_DIR` | Default `.crabdb` directory. |
| `CRABDB_BRANCH` | Default branch. |
| `CRABDB_FORMAT` | `human`, `json`, or `ndjson`; `json` also enables JSON errors. |
| `CRABDB_DAEMON_URL` | Default daemon URL. |
| `CRABDB_DAEMON_TOKEN` | Default daemon token and daemon startup token source. |

## Workspace Discovery

If neither `--workspace` nor `--db` is supplied, CrabDB discovers a workspace by walking upward from the current directory until it finds `.crabdb`.

If `--db` is supplied without `--workspace`, the parent directory of the database directory is treated as the workspace.

## JSON Errors

Parse errors and runtime errors are rendered as JSON when `--json`, `--format json`, or `CRABDB_FORMAT=json` is used.

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

`--format ndjson` is accepted globally. The current command path that emits newline-delimited JSON is `crabdb index watch`, which prints one `WorktreeIndexReport` per iteration.

## Code Facts Used

- Parser: `crates/crabdb/src/cli/command.rs`
- Runtime resolution: `crates/crabdb/src/cli/command/handler/runtime.rs`
- Error rendering: `crates/crabdb/src/cli/command/handler/errors.rs`
- Tests: `cli_json_errors_are_machine_readable`, `cli_env_defaults_select_workspace_db_branch_and_format`


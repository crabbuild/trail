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
| `--json` | Compatibility shorthand for `--format json`; renders structured JSON output and JSON errors. |
| `--format <FORMAT>` | `human`, `plain`, `json`, or `ndjson`. Human is adaptive; plain is deterministic ASCII. |
| `--color <POLICY>` | `auto`, `always`, or `never`. `auto` honors TTY detection, `NO_COLOR`, and `TERM=dumb`. |
| `--pager <POLICY>` | `auto`, `always`, or `never`. Only eligible long-form human review output can page. |
| `--quiet` | Suppress successful human/plain output; diagnostics remain on stderr. |
| `--verbose` | Reveal full identifiers and secondary technical evidence. |
| `--trace` | Accepted global flag for trace mode. |
| `--daemon-url <URL>` | Route supported hot commands to a daemon URL. |
| `--daemon-token <TOKEN>` | Token for daemon-routed commands. |

## Environment Variables

| Variable | Used For |
| --- | --- |
| `TRAIL_WORKSPACE` | Default workspace root. |
| `TRAIL_DIR` | Default `.trail` directory. |
| `TRAIL_BRANCH` | Default branch. |
| `TRAIL_FORMAT` | `human`, `plain`, `json`, or `ndjson`; structured modes also enable JSON errors. |
| `TRAIL_DAEMON_URL` | Default daemon URL. |
| `TRAIL_DAEMON_TOKEN` | Default daemon token and daemon startup token source. |

## Workspace Discovery

If neither `--workspace` nor `--db` is supplied, Trail discovers a workspace by walking upward from the current directory until it finds `.trail`.

If `--db` is supplied without `--workspace`, the parent directory of the database directory is treated as the workspace.

## JSON Errors

Parse errors and runtime errors are rendered as JSON when `--json`, `--format json`, `--format ndjson`, or the corresponding `TRAIL_FORMAT` is used.

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

`--format ndjson` is accepted globally. It is a record-stream contract: supported streaming commands (currently `trail index watch` and lane watch commands) emit one report per line. Commands that return a single report use `--format json` instead.

## Human-output cutoff

Trail's human output is deliberately not a compatibility API. The former
`--no-color` flag was removed; use `--color never`. Scripts and integrations
must select `--format json` or `--format ndjson`, because human and plain
layouts may evolve for readability. See [the terminal output contract](../../CLI_TERMINAL_OUTPUT.md)
for paging, progress, raw-content exceptions, and examples.

## Code Facts Used

- Parser: `trail/src/cli/command.rs`
- Runtime resolution: `trail/src/cli/command/handler/runtime.rs`
- Error rendering: `trail/src/cli/command/handler/errors.rs`
- Tests: `cli_json_errors_are_machine_readable`, `cli_env_defaults_select_workspace_db_branch_and_format`

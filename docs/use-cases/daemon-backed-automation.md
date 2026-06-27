# Use Case: Daemon-Backed Automation

Use the daemon for editor integrations and automation that repeatedly asks for status, diffs, records, and lane reports.

## Start the Daemon

```sh
crabdb daemon
```

Defaults:

- Host: `127.0.0.1`
- Port: `8765`
- Auth: enabled unless `--no-auth` is passed

The daemon registers its endpoint in `.crabdb/daemon.json`. When it creates a token, it writes `.crabdb/daemon.token` with private permissions on Unix.

## Route CLI Commands Through the Daemon

```sh
crabdb --daemon-url http://127.0.0.1:8765 --daemon-token "$TOKEN" status
```

Or set:

```sh
export CRABDB_DAEMON_URL=http://127.0.0.1:8765
export CRABDB_DAEMON_TOKEN=$TOKEN
```

The CLI can also auto-discover `.crabdb/daemon.json` for supported hot commands.

## Supported Hot CLI Commands

Daemon routing supports:

- `status` without `--branch`
- `record`
- `diff`
- selected `lane` commands
- `merge-lane`
- `merge-queue`

If auto-discovery finds a stale daemon endpoint, the CLI falls back to local execution for unavailable daemon errors.

## Code Facts Used

- Daemon args and auth: `crates/crabdb/src/cli/command/maintenance_args.rs`, `crates/crabdb/src/cli/command/handler/maintenance.rs`
- Daemon CLI routing: `crates/crabdb/src/cli/command/handler/daemon_rpc.rs`
- Tests: `cli_daemon_url_routes_hot_lane_commands`, `cli_auto_discovers_daemon_for_hot_commands_and_falls_back_on_stale_endpoint`

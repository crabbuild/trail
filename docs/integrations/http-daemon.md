# HTTP Daemon

The daemon serves the local JSON HTTP API.

## Start

```sh
crabdb daemon
```

Options:

- `--host <HOST>` defaults to `127.0.0.1`.
- `--port <PORT>` defaults to `8765`.
- `--once` serves one request.
- `--max-requests <N>` exits after N requests.
- `--auth-token <TOKEN>` uses an explicit bearer token.
- `--auth-token-file <PATH>` reads or writes a token file.
- `--no-auth` disables auth and cannot be combined with token flags.

## Endpoint Discovery

The daemon writes:

```text
.crabdb/daemon.json
```

That file contains version, URL, pid, and whether auth is enabled.

When no token is supplied, the daemon creates or reads:

```text
.crabdb/daemon.token
```

On Unix, newly created token files are restricted to mode `0600`.

## Auth

`GET /v1/health` is unauthenticated. Other routes require auth unless `--no-auth` is used.
Requests larger than 16 MiB are rejected before routing. Requests with an
`Origin` header are accepted only when the origin host is loopback, such as
`localhost`, `127.0.0.1`, or `::1`.
The daemon applies a per-peer listener rate limit of 600 requests per 60
seconds and returns `429 Too Many Requests` when the window is exhausted.
Mutating requests can include `Idempotency-Key` to make retries replay the first
stored response instead of dispatching the mutation again.

Send either:

```text
Authorization: Bearer <token>
```

Or:

```text
x-crabdb-token: <token>
```

## CLI Routing

Use:

```sh
crabdb --daemon-url http://127.0.0.1:8765 --daemon-token "$TOKEN" status
```

Or set `CRABDB_DAEMON_URL` and `CRABDB_DAEMON_TOKEN`. Supported hot commands can also auto-discover `.crabdb/daemon.json`.

## Code Facts Used

- Daemon args/auth: `crates/crabdb/src/cli/command/maintenance_args.rs`, `crates/crabdb/src/cli/command/handler/maintenance.rs`
- HTTP transport/auth: `crates/crabdb/src/server/transport.rs`, `crates/crabdb/src/server/route/utils.rs`
- CLI daemon routing: `crates/crabdb/src/cli/command/handler/daemon_rpc.rs`

# HTTP Daemon

The daemon serves the local JSON HTTP API.

## Start

```sh
trail daemon
```

Options:

- `--host <HOST>` defaults to `127.0.0.1`.
- `--port <PORT>` defaults to `8765`.
- `--once` serves one request.
- `--max-requests <N>` exits after N requests.
- `--auth-token <TOKEN>` uses an explicit bearer token.
- `--auth-token-file <PATH>` reads or writes a token file.
- `--no-auth` disables auth, cannot be combined with token flags, is allowed
  only with a loopback listener, and prints a stderr `WARNING` even with
  `--quiet` because any local process can mutate the workspace while it is
  enabled.

## Endpoint Discovery

The daemon writes:

```text
.trail/daemon.json
```

That file contains version, URL, pid, and whether auth is enabled.

When no token is supplied, the daemon creates or reads:

```text
.trail/daemon.token
```

Token files must be regular files; symlink token files are rejected. On Unix,
newly created or reused token files are restricted to mode `0600` before the
daemon accepts them.

## Auth

`GET /v1/health` is unauthenticated. Other routes require auth unless `--no-auth` is used.
Requests larger than 16 MiB are rejected before routing. Requests with an
`Origin` header are accepted only when the origin is well-formed, uses a valid
port if one is present, and has a loopback host such as `localhost`,
`127.0.0.1`, or `::1`.
Every routed request must include a `Host` header whose host is loopback, such
as `localhost`, `127.0.0.1`, or `[::1]`; missing Host headers, DNS names, paths,
credentials, whitespace, and invalid ports are rejected.
The daemon accepts origin-form `HTTP/1.0` or `HTTP/1.1` request lines and
fixed-length requests only. It rejects malformed request lines, malformed
headers, duplicate `Content-Length`, bodies without `Content-Length`, body
length mismatches, and `Transfer-Encoding`.
Slow or incomplete requests receive `408 Request Timeout` without stopping the
listener.
The daemon applies a per-peer listener rate limit of 600 requests per 60
seconds and returns `429 Too Many Requests` with `Retry-After` when the window
is exhausted.
Use `--rate-limit-requests`, `--rate-limit-window-secs`, and
`--connection-timeout-secs` to tighten or relax these listener controls for a
specific integration. Each value must be greater than zero.
Mutating requests can include `Idempotency-Key` to make retries replay the first
stored response instead of dispatching the mutation again. Keys must be 1-200
non-control characters.

Send either:

```text
Authorization: Bearer <token>
```

Or:

```text
x-trail-token: <token>
```

## CLI Routing

Use:

```sh
trail --daemon-url http://127.0.0.1:8765 --daemon-token "$TOKEN" status
```

Or set `TRAIL_DAEMON_URL` and `TRAIL_DAEMON_TOKEN`. Supported hot commands can also auto-discover `.trail/daemon.json`.

## Code Facts Used

- Daemon args/auth: `crates/trail/src/cli/command/maintenance_args.rs`, `crates/trail/src/cli/command/handler/maintenance.rs`
- HTTP transport/auth: `crates/trail/src/server/transport.rs`, `crates/trail/src/server/route/utils.rs`
- CLI daemon routing: `crates/trail/src/cli/command/handler/daemon_rpc.rs`

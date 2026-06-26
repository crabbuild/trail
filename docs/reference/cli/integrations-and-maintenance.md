# CLI Reference: Integrations and Maintenance

## `git`

```text
crabdb git export <RANGE> [-m <MESSAGE>] [--output <PATH>]
crabdb git import-update [-m <MESSAGE>]
crabdb git mappings [--limit <N>]
```

Mapping limit defaults to 30.

## `api`

```text
crabdb api openapi [--output <PATH>]
```

## `daemon`

```text
crabdb daemon [--host <HOST>] [--port <PORT>] [--once] [--max-requests <N>] [--auth-token <TOKEN>] [--auth-token-file <PATH>] [--no-auth]
```

Defaults: host `127.0.0.1`, port `8765`, auth enabled.

## `mcp`

```text
crabdb mcp
```

Starts the MCP stdio server.

## `doctor`

```text
crabdb doctor
```

Runs workspace and integration diagnostics.

## `backup`

```text
crabdb backup create <OUTPUT> [--overwrite]
crabdb backup verify <PATH>
crabdb backup restore <PATH> [--force]
```

## `fsck`

```text
crabdb fsck
```

Verifies repository integrity.

## `index`

```text
crabdb index rebuild [--rich-text]
crabdb index watch [--once] [--iterations <N>] [--interval-ms <MS>]
```

`index watch` default interval is 1000 ms.

## `gc`

```text
crabdb gc [--dry-run]
```

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/maintenance_args.rs`
- Handlers: `crates/crabdb/src/cli/command/handler/maintenance.rs`
- Reports: `crates/crabdb/src/model/reports/maintenance.rs`


# Install and Build

CrabDB is a Rust workspace with two crates:

- `crabdb`: the CLI, library API, HTTP daemon, and MCP server.
- `prolly`: the prolly-tree storage library used by CrabDB.

The workspace declares Rust 1.81 in `Cargo.toml`.

## Build from Source

From the repository root:

```sh
cargo build -p crabdb
```

Run the CLI through Cargo:

```sh
cargo run -p crabdb -- --help
```

After a debug build, the binary is also available at:

```sh
target/debug/crabdb --help
```

## Validate the Local Build

Run the CrabDB test suite:

```sh
cargo test -p crabdb
```

Run the prolly crate tests separately when changing storage internals:

```sh
cargo test -p prolly
```

## What Gets Installed

CrabDB does not require a background service to use the basic CLI. The daemon is started explicitly with `crabdb daemon`, and the MCP server is started explicitly with `crabdb mcp`.

## Code Facts Used

- Workspace members and Rust version: `Cargo.toml`
- Binary entrypoint: `crates/crabdb/src/main.rs`
- CLI command surface: `crates/crabdb/src/cli/command.rs`


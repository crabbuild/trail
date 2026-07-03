# Install and Build

CrabDB is a Rust workspace with two crates:

- `crabdb`: the CLI, library API, HTTP daemon, and MCP server.
- `prolly`: the prolly-tree storage library used by CrabDB.

The workspace declares Rust 1.81 in `Cargo.toml`.

## Install the CLI

From the repository root:

```sh
make install
```

By default this builds an optimized release binary and installs it to:

```text
$HOME/.cargo/bin/crabdb
```

Verify the installed command and the lane command group:

```sh
crabdb --help
crabdb lane --help
```

If `$HOME/.cargo/bin` is not on your `PATH`, either add it or call the binary
directly from that directory. For a project-local install, override `PREFIX`:

```sh
make install PREFIX="$PWD/.local"
./.local/bin/crabdb --help
```

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

## Initialize a Project

Run this once from the project you want CrabDB to track:

```sh
cd /path/to/project
crabdb init --working-tree
crabdb status
```

`--working-tree` imports the visible current files as the initial CrabDB root.
Use `--from-git` instead when the initial root should include only Git-tracked
paths.

## Start Daily Lane Work

Create a lane for one task:

```sh
crabdb lane spawn docs-lane --from main --materialize=true
crabdb lane status docs-lane
```

Use the lane workdir for normal editing tools or an external coding agent:

```sh
LANE_DIR="$(crabdb lane workdir docs-lane)"
cd "$LANE_DIR"
# Edit files here, or point Claude Code, Codex, Cursor, or another tool here.
```

Record the lane work back into CrabDB from the original project workspace:

```sh
cd /path/to/project
crabdb lane record docs-lane -m "record docs update"
crabdb lane diff docs-lane --patch
crabdb lane readiness docs-lane
```

Merge only after review and readiness checks:

```sh
crabdb merge-lane docs-lane --into main --dry-run
crabdb merge-queue add docs-lane --into main
crabdb merge-queue run
```

## Validate the Local Build

Run the CrabDB test suite:

```sh
cargo test -p crabdb
```

Run the prolly crate tests separately when changing storage internals:

```sh
cargo test -p prolly-map
```

## What Gets Installed

CrabDB does not require a background service to use the basic CLI. The daemon is started explicitly with `crabdb daemon`, and the MCP server is started explicitly with `crabdb mcp`.

## Code Facts Used

- Workspace members and Rust version: `Cargo.toml`
- Binary entrypoint: `crates/crabdb/src/main.rs`
- CLI command surface: `crates/crabdb/src/cli/command.rs`

# Install and Build

Trail is a Rust workspace with two crates:

- `trail`: the CLI, library API, HTTP daemon, and MCP server.
- `prolly`: the prolly-tree storage library used by Trail.

The workspace declares Rust 1.81 in `Cargo.toml`.

## Install a Release

### macOS

Install the Intel or Apple Silicon binary with Homebrew:

```sh
brew install crabbuild/tap/trail
trail --version
```

### Linux

The installer selects the matching x86-64 or ARM64 GitHub Release archive and
installs `trail` under Cargo's normal binary directory:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/crabbuild/trail/releases/latest/download/trail-installer.sh | sh
trail --version
```

Homebrew users on Linux can alternatively run:

```sh
brew install crabbuild/tap/trail
```

### Windows

Install the Dokany 2 runtime, then run the Trail release installer from
PowerShell:

```powershell
winget install --id dokan-dev.Dokany --version 2.0.6.1000 --exact
irm https://github.com/crabbuild/trail/releases/latest/download/trail-installer.ps1 | iex
trail --version
```

Archives and their SHA-256 checksum files are also attached to every
[GitHub Release](https://github.com/crabbuild/trail/releases).

The Windows binary currently links the Dokany 2.0.6 runtime, while the Dokan
driver is actively used only for mounted-workspace lanes. Linux FUSE and
optional macFUSE are needed only for their corresponding mounted-workspace
implementations.

## Install from Source

From the repository root:

```sh
make install
```

By default this builds an optimized release binary and installs it to:

```text
$HOME/.cargo/bin/trail
```

Verify the source-built command and the lane command group:

```sh
trail --help
trail lane --help
```

If `$HOME/.cargo/bin` is not on your `PATH`, either add it or call the binary
directly from that directory. For a project-local install, override `PREFIX`:

```sh
make install PREFIX="$PWD/.local"
./.local/bin/trail --help
```

## Build Without Installing

From the repository root:

```sh
cargo build -p trail
```

Run the CLI through Cargo:

```sh
cargo run -p trail -- --help
```

After a debug build, the binary is also available at:

```sh
target/debug/trail --help
```

## Initialize a Project

Run this once from the project you want Trail to track:

```sh
cd /path/to/project
trail init --working-tree
trail status
```

`--working-tree` imports the visible current files as the initial Trail root.
Use `--from-git` instead when the initial root should include only Git-tracked
paths.

## Start Daily Lane Work

Create a lane for one task:

```sh
trail lane spawn docs-lane --from main --materialize=true
trail lane status docs-lane
```

Use the lane workdir for normal editing tools or an external coding agent:

```sh
LANE_DIR="$(trail lane workdir docs-lane)"
cd "$LANE_DIR"
# Edit files here, or point Claude Code, Codex, Cursor, or another tool here.
```

Record the lane work back into Trail from the original project workspace:

```sh
cd /path/to/project
trail lane record docs-lane -m "record docs update"
trail lane diff docs-lane --patch
trail lane readiness docs-lane
```

Merge only after review and readiness checks:

```sh
trail lane merge docs-lane --into main --dry-run
trail merge-queue add docs-lane --into main
trail merge-queue run
```

## Validate the Local Build

Run the Trail test suite:

```sh
cargo test -p trail
```

Run the prolly crate tests separately when changing storage internals:

```sh
cargo test -p prolly-map
```

## What Gets Installed

Trail does not require a background service to use the basic CLI. The daemon is started explicitly with `trail daemon`, and the MCP server is started explicitly with `trail mcp`.

## Code Facts Used

- Workspace members and Rust version: `Cargo.toml`
- Binary entrypoint: `trail/src/main.rs`
- CLI command surface: `trail/src/cli/command.rs`

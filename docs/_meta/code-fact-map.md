# Code Fact Map

This map records the source files used to write the docs. The rewritten docs intentionally do not use the old Markdown prose as source material.

## Global Sources

- CLI parser and commands: `crates/crabdb/src/cli/command.rs`
- CLI args: `crates/crabdb/src/cli/command/*_args.rs`
- CLI handlers: `crates/crabdb/src/cli/command/handler`
- CLI renderers: `crates/crabdb/src/cli/command/render`
- Error categories and exit codes: `crates/crabdb/src/error.rs`
- Public library exports: `crates/crabdb/src/lib.rs`
- Public models: `crates/crabdb/src/model`
- Public reports: `crates/crabdb/src/model/reports`
- e2e tests: `crates/crabdb/tests/e2e.rs`

## Getting Started

- Install/build: `Cargo.toml`, `crates/crabdb/Cargo.toml`, `crates/crabdb/src/main.rs`
- Init: `crates/crabdb/src/db/core/init.rs`, `crates/crabdb/src/db/util/config/policy.rs`
- First record/query: `crates/crabdb/src/db/record`, `crates/crabdb/src/db/storage/query.rs`
- First agent workflow: `crates/crabdb/src/db/agent`, `crates/crabdb/src/db/merge`

## Concepts

- Operation database: `crates/crabdb/src/model/domain/operations.rs`
- Workspaces/refs/branches: `crates/crabdb/src/db/storage/refs.rs`, `crates/crabdb/src/db/record/branches.rs`
- Objects/text/line identity: `crates/crabdb/src/model/domain/objects.rs`, `crates/crabdb/src/ids.rs`
- Selectors: `crates/crabdb/src/db/storage/refs.rs`, `crates/crabdb/src/cli/command/handler/inspect.rs`
- Ignore/guardrails: `crates/crabdb/src/db/core/workspace`, `crates/crabdb/src/db/util/guardrails`
- Agents/sessions/traces: `crates/crabdb/src/db/agent/control`, `crates/crabdb/src/model/agent`
- Readiness/gates: `crates/crabdb/src/db/agent/readiness.rs`, `crates/crabdb/src/db/agent/gates`
- Storage/indexes/backups: `crates/crabdb/src/db/storage`, `crates/crabdb/src/db/core/backup`

## Guides and Use Cases

- Recording: `crates/crabdb/src/db/record/recording`
- Inspection/provenance: `crates/crabdb/src/db/record/inspection`, `crates/crabdb/src/db/storage/query.rs`
- Branch/checkout/merge: `crates/crabdb/src/db/record/branches.rs`, `crates/crabdb/src/db/record/checkout.rs`, `crates/crabdb/src/db/merge`
- Config: `crates/crabdb/src/db/util/config`
- Git interop: `crates/crabdb/src/db/storage/git.rs`, `crates/crabdb/src/db/record/recording/git.rs`, `crates/crabdb/src/db/merge/git_export.rs`
- Maintenance: `crates/crabdb/src/db/core/doctor*.rs`, `crates/crabdb/src/db/storage/lifecycle`

## Agent Docs

- Lifecycle/workdirs: `crates/crabdb/src/db/agent/lifecycle.rs`, `crates/crabdb/src/db/agent/workdir`
- Structured patches: `crates/crabdb/src/model/inspect/patch.rs`, `crates/crabdb/src/db/agent/patching.rs`
- Sessions/turns/runs: `crates/crabdb/src/db/agent/control`
- Events/traces: `crates/crabdb/src/db/agent/control/traces`
- Gates/readiness: `crates/crabdb/src/db/agent/gates`, `crates/crabdb/src/db/agent/readiness.rs`
- Handoff/review/merge: `crates/crabdb/src/db/agent/readiness.rs`, `crates/crabdb/src/db/merge`

## Integrations and Reference

- HTTP daemon: `crates/crabdb/src/server`, `crates/crabdb/src/cli/command/handler/daemon_rpc.rs`
- OpenAPI: `crates/crabdb/src/server/openapi`
- MCP: `crates/crabdb/src/mcp`
- Config reference: `crates/crabdb/src/db/util/config`, `crates/crabdb/src/db/util/config_parse.rs`
- Patch reference: `crates/crabdb/src/model/inspect/patch.rs`, `crates/crabdb/src/server/request_types/patches.rs`
- Data types: `crates/crabdb/src/model`, `crates/crabdb/src/ids.rs`

## Verification Commands

Useful commands after docs changes:

```sh
cargo run -p crabdb -- --help
cargo run -p crabdb -- agent --help
cargo run -p crabdb -- merge-queue --help
cargo run -p crabdb -- daemon --help
rg -n "\\]\\(" docs
cargo test -p crabdb
```


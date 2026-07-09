# Code Fact Map

This map records the source files used to write the docs. The rewritten docs intentionally do not use the old Markdown prose as source material.

## Global Sources

- CLI parser and commands: `crates/trail/src/cli/command.rs`
- CLI args: `crates/trail/src/cli/command/*_args.rs`
- CLI handlers: `crates/trail/src/cli/command/handler`
- CLI renderers: `crates/trail/src/cli/command/render`
- Error categories and exit codes: `crates/trail/src/error.rs`
- Public library exports: `crates/trail/src/lib.rs`
- Public models: `crates/trail/src/model`
- Public reports: `crates/trail/src/model/reports`
- e2e tests: `crates/trail/tests/e2e.rs`

## Getting Started

- Install/build: `Cargo.toml`, `crates/trail/Cargo.toml`, `crates/trail/src/main.rs`
- Init: `crates/trail/src/db/core/init.rs`, `crates/trail/src/db/util/config/policy.rs`
- First record/query: `crates/trail/src/db/record`, `crates/trail/src/db/storage/query.rs`
- First lane workflow: `crates/trail/src/db/lane`, `crates/trail/src/db/merge`

## Concepts

- Operation database: `crates/trail/src/model/domain/operations.rs`
- Workspaces/refs/branches: `crates/trail/src/db/storage/refs.rs`, `crates/trail/src/db/record/branches.rs`
- Objects/text/line identity: `crates/trail/src/model/domain/objects.rs`, `crates/trail/src/ids.rs`
- Selectors: `crates/trail/src/db/storage/refs.rs`, `crates/trail/src/cli/command/handler/inspect.rs`
- Ignore/guardrails: `crates/trail/src/db/core/workspace`, `crates/trail/src/db/util/guardrails`
- Lanes/sessions/traces: `crates/trail/src/db/lane/control`, `crates/trail/src/model/lane`
- Readiness/gates: `crates/trail/src/db/lane/readiness.rs`, `crates/trail/src/db/lane/gates`
- Storage/indexes/backups: `crates/trail/src/db/storage`, `crates/trail/src/db/core/backup`

## Guides and Use Cases

- Recording: `crates/trail/src/db/record/recording`
- Inspection/provenance: `crates/trail/src/db/record/inspection`, `crates/trail/src/db/storage/query.rs`
- Branch/checkout/merge: `crates/trail/src/db/record/branches.rs`, `crates/trail/src/db/record/checkout.rs`, `crates/trail/src/db/merge`
- Config: `crates/trail/src/db/util/config`
- Git interop: `crates/trail/src/db/storage/git.rs`, `crates/trail/src/db/record/recording/git.rs`, `crates/trail/src/db/merge/git_export.rs`
- Maintenance: `crates/trail/src/db/core/doctor*.rs`, `crates/trail/src/db/storage/lifecycle`

## Lane Docs

- Lifecycle/workdirs: `crates/trail/src/db/lane/lifecycle.rs`, `crates/trail/src/db/lane/workdir`
- Structured patches: `crates/trail/src/model/inspect/patch.rs`, `crates/trail/src/db/lane/patching.rs`
- Sessions/turns/runs: `crates/trail/src/db/lane/control`
- Events/traces: `crates/trail/src/db/lane/control/traces`
- Gates/readiness: `crates/trail/src/db/lane/gates`, `crates/trail/src/db/lane/readiness.rs`
- Handoff/review/merge: `crates/trail/src/db/lane/readiness.rs`, `crates/trail/src/db/merge`

## Integrations and Reference

- HTTP daemon: `crates/trail/src/server`, `crates/trail/src/cli/command/handler/daemon_rpc.rs`
- OpenAPI: `crates/trail/src/server/openapi`
- MCP: `crates/trail/src/mcp`
- ACP relay design: `docs/design/acp-relay.md`
- Config reference: `crates/trail/src/db/util/config`, `crates/trail/src/db/util/config_parse.rs`
- Patch reference: `crates/trail/src/model/inspect/patch.rs`, `crates/trail/src/server/request_types/patches.rs`
- Data types: `crates/trail/src/model`, `crates/trail/src/ids.rs`

## Verification Commands

Useful commands after docs changes:

```sh
cargo run -p trail -- --help
cargo run -p trail -- lane --help
cargo run -p trail -- merge-queue --help
cargo run -p trail -- daemon --help
rg -n "\\]\\(" docs
cargo test -p trail
```

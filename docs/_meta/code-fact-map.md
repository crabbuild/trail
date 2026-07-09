# Code Fact Map

This map records the source files used to write the docs. The rewritten docs intentionally do not use the old Markdown prose as source material.

## Global Sources

- CLI parser and commands: `trail/src/cli/command.rs`
- CLI args: `trail/src/cli/command/*_args.rs`
- CLI handlers: `trail/src/cli/command/handler`
- CLI renderers: `trail/src/cli/command/render`
- Error categories and exit codes: `trail/src/error.rs`
- Public library exports: `trail/src/lib.rs`
- Public models: `trail/src/model`
- Public reports: `trail/src/model/reports`
- e2e tests: `trail/tests/e2e.rs`

## Getting Started

- Install/build: `Cargo.toml`, `trail/Cargo.toml`, `trail/src/main.rs`
- Init: `trail/src/db/core/init.rs`, `trail/src/db/util/config/policy.rs`
- First record/query: `trail/src/db/record`, `trail/src/db/storage/query.rs`
- First lane workflow: `trail/src/db/lane`, `trail/src/db/merge`

## Concepts

- Operation database: `trail/src/model/domain/operations.rs`
- Workspaces/refs/branches: `trail/src/db/storage/refs.rs`, `trail/src/db/record/branches.rs`
- Objects/text/line identity: `trail/src/model/domain/objects.rs`, `trail/src/ids.rs`
- Selectors: `trail/src/db/storage/refs.rs`, `trail/src/cli/command/handler/inspect.rs`
- Ignore/guardrails: `trail/src/db/core/workspace`, `trail/src/db/util/guardrails`
- Lanes/sessions/traces: `trail/src/db/lane/control`, `trail/src/model/lane`
- Readiness/gates: `trail/src/db/lane/readiness.rs`, `trail/src/db/lane/gates`
- Storage/indexes/backups: `trail/src/db/storage`, `trail/src/db/core/backup`

## Guides and Use Cases

- Recording: `trail/src/db/record/recording`
- Inspection/provenance: `trail/src/db/record/inspection`, `trail/src/db/storage/query.rs`
- Branch/checkout/merge: `trail/src/db/record/branches.rs`, `trail/src/db/record/checkout.rs`, `trail/src/db/merge`
- Config: `trail/src/db/util/config`
- Git interop: `trail/src/db/storage/git.rs`, `trail/src/db/record/recording/git.rs`, `trail/src/db/merge/git_export.rs`
- Maintenance: `trail/src/db/core/doctor*.rs`, `trail/src/db/storage/lifecycle`

## Lane Docs

- Lifecycle/workdirs: `trail/src/db/lane/lifecycle.rs`, `trail/src/db/lane/workdir`
- Structured patches: `trail/src/model/inspect/patch.rs`, `trail/src/db/lane/patching.rs`
- Sessions/turns/runs: `trail/src/db/lane/control`
- Events/traces: `trail/src/db/lane/control/traces`
- Gates/readiness: `trail/src/db/lane/gates`, `trail/src/db/lane/readiness.rs`
- Handoff/review/merge: `trail/src/db/lane/readiness.rs`, `trail/src/db/merge`

## Integrations and Reference

- HTTP daemon: `trail/src/server`, `trail/src/cli/command/handler/daemon_rpc.rs`
- OpenAPI: `trail/src/server/openapi`
- MCP: `trail/src/mcp`
- ACP relay design: `docs/design/acp-relay.md`
- Config reference: `trail/src/db/util/config`, `trail/src/db/util/config_parse.rs`
- Patch reference: `trail/src/model/inspect/patch.rs`, `trail/src/server/request_types/patches.rs`
- Data types: `trail/src/model`, `trail/src/ids.rs`

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

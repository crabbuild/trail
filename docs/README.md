# CrabDB Documentation

CrabDB is a local-first operation database for code and text worktrees. It records meaningful operations, preserves stable file and line identity, and gives humans and coding agents a branch, provenance, and review layer above a normal workspace.

These docs are written from the current Rust code, CLI definitions, exported model types, OpenAPI paths, MCP tools, and e2e tests. The old large guide files are kept only as compatibility entry points.

## Start Here

- [Install and build](getting-started/install-and-build.md)
- [Initialize a workspace](getting-started/initialize-a-workspace.md)
- [First record and provenance query](getting-started/first-record-and-query.md)
- [First agent workflow](getting-started/first-agent-workflow.md)

## Core Concepts

- [Operation database](concepts/operation-database.md)
- [Workspaces, refs, and branches](concepts/workspaces-refs-and-branches.md)
- [Objects, roots, text, and line identity](concepts/objects-roots-text-and-line-identity.md)
- [Selectors and ref-like inputs](concepts/selectors-and-refish.md)
- [Recording, ignore rules, and guardrails](concepts/recording-ignore-and-guardrails.md)
- [Agents, sessions, turns, and traces](concepts/agents-sessions-turns-and-traces.md)
- [Readiness gates and merge safety](concepts/readiness-gates-and-merge-safety.md)
- [Storage, indexes, and backups](concepts/storage-indexes-and-backups.md)

## Guides

- [Record worktree changes](guides/record-worktree-changes.md)
- [Inspect history and provenance](guides/inspect-history-and-provenance.md)
- [Branch, checkout, and merge](guides/branch-checkout-and-merge.md)
- [Configure CrabDB](guides/configure-crabdb.md)
- [Ignore files and preflight actions](guides/ignore-files-and-preflight-actions.md)
- [Git interop](guides/git-interop.md)
- [Maintenance and recovery](guides/maintenance-and-recovery.md)
- [Performance and scale benchmarks](guides/performance-and-scale-benchmarks.md)

## Agent Workflows

- [Agent overview](agents/overview.md)
- [Spawn and materialize workdirs](agents/spawn-and-materialize-workdirs.md)
- [Structured patches](agents/structured-patches.md)
- [Sessions, turns, messages, and runs](agents/sessions-turns-messages-and-runs.md)
- [Events, traces, and spans](agents/events-traces-and-spans.md)
- [Tests, evals, gates, and readiness](agents/tests-evals-gates-and-readiness.md)
- [Handoff, review, and merge](agents/handoff-review-and-merge.md)

## Integrations

- [Integration overview](integrations/overview.md)
- [HTTP daemon](integrations/http-daemon.md)
- [OpenAPI](integrations/openapi.md)
- [MCP](integrations/mcp.md)
- [Rust library](integrations/rust-library.md)
- [Git](integrations/git.md)

## Reference

- [CLI global options and environment](reference/cli/global-options-and-env.md)
- [CLI command reference](reference/cli/workspace-and-config.md)
- [Configuration keys](reference/config.md)
- [Patch format](reference/patch-format.md)
- [HTTP API](reference/http-api.md)
- [MCP tools](reference/mcp-tools.md)
- [Data types](reference/data-types.md)
- [Design notes](design/architecture.md)
- [Code fact map](./_meta/code-fact-map.md)

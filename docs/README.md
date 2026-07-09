# Trail Documentation

Trail is a local-first operation database for code and text worktrees. It records meaningful operations, preserves stable file and line identity, and gives humans and coding agents a branch, provenance, and review layer above a normal workspace.

These docs are written from the current Rust code, CLI definitions, exported model types, OpenAPI paths, MCP tools, and e2e tests.

## Start Here

- [Roadmap](../ROADMAP.md)
- [Install and build](getting-started/install-and-build.md)
- [Initialize a workspace](getting-started/initialize-a-workspace.md)
- [First record and provenance query](getting-started/first-record-and-query.md)
- [First lane workflow](getting-started/first-lane-workflow.md)

## Core Concepts

- [Operation database](concepts/operation-database.md)
- [Workspaces, refs, and branches](concepts/workspaces-refs-and-branches.md)
- [Objects, roots, text, and line identity](concepts/objects-roots-text-and-line-identity.md)
- [Selectors and ref-like inputs](concepts/selectors-and-refish.md)
- [Recording, ignore rules, and guardrails](concepts/recording-ignore-and-guardrails.md)
- [Lanes, sessions, turns, and traces](concepts/lanes-sessions-turns-and-traces.md)
- [Readiness gates and merge safety](concepts/readiness-gates-and-merge-safety.md)
- [Storage, indexes, and backups](concepts/storage-indexes-and-backups.md)
- [Prolly tree deep dive](concepts/prolly-tree-deep-dive.md)

## Guides

- [Record worktree changes](guides/record-worktree-changes.md)
- [Inspect history and provenance](guides/inspect-history-and-provenance.md)
- [Branch, checkout, and merge](guides/branch-checkout-and-merge.md)
- [Configure Trail](guides/configure-trail.md)
- [Harden agent workflows](guides/hardening-agent-workflows.md)
- [Ignore files and preflight actions](guides/ignore-files-and-preflight-actions.md)
- [Git interop](guides/git-interop.md)
- [Maintenance and recovery](guides/maintenance-and-recovery.md)
- [Performance and scale benchmarks](guides/performance-and-scale-benchmarks.md)

## Lane Workflows

- [Lane overview](lanes/overview.md)
- [Lane work model](lanes/work-model.md)
- [Spawn and materialize workdirs](lanes/spawn-and-materialize-workdirs.md)
- [Structured patches](lanes/structured-patches.md)
- [Sessions, turns, messages, and runs](lanes/sessions-turns-messages-and-runs.md)
- [Events, traces, and spans](lanes/events-traces-and-spans.md)
- [Tests, evals, gates, and readiness](lanes/tests-evals-gates-and-readiness.md)
- [Handoff, review, and merge](lanes/handoff-review-and-merge.md)

## Integrations

- [Integration overview](integrations/overview.md)
- [HTTP daemon](integrations/http-daemon.md)
- [OpenAPI](integrations/openapi.md)
- [MCP](integrations/mcp.md)
- [ACP relay design](design/acp-relay.md)
- [VS Code ACP chat view design](design/vscode-acp-chat-view.md)
- [VS Code extension implementation](../extensions/vscode/README.md)
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
- [Distributed Prolly VCS design](design/distributed-prolly-vcs.md)
- [ACP relay design](design/acp-relay.md)
- [VS Code ACP chat view design](design/vscode-acp-chat-view.md)
- [Code fact map](./_meta/code-fact-map.md)

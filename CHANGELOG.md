# Changelog

All notable changes to Trail are documented in this file. Trail follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Breaking:** Trail CLI human output now uses the unified outcome-first
  terminal renderer. The old human layouts and `--no-color` option are removed;
  use `--color never` instead.
- **Breaking:** `trail merge-lane` is removed. Use
  `trail lane merge <lane> --into <branch>` for lane-specific merges; the
  `trail merge` command remains for generic branch/ref merges.
- **Breaking:** `POST /v1/branches/{branch}/merge-lane` is removed. Use
  `POST /v1/lanes/{lane}/merge` with the target branch in the required `into`
  JSON field.
- Added `--format human|plain|json|ndjson`, `--color auto|always|never`, and
  `--pager auto|always|never`. `plain` is deterministic text; JSON and NDJSON
  are the supported contracts for automation.
- Status, diff, history, lane, agent, maintenance, and diagnostic output now
  use responsive tables, ordered checklists, explicit notices, and safe next
  actions. Human output is intentionally not stable for parsing.

## [0.1.0] - 2026-07-10

### Added

- Local-first operation history, branches, line provenance, and worktree recording.
- Isolated agent lanes with sessions, turns, patches, approvals, gates, and handoffs.
- Conflict-aware lane merges, merge queues, readiness reports, and recovery checkpoints.
- CLI, HTTP daemon, MCP stdio server, ACP relay, and Rust API integration surfaces.
- Backup, restore, filesystem checks, index rebuilding, and maintenance commands.

[Unreleased]: https://github.com/crabbuild/trail/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/crabbuild/trail/releases/tag/v0.1.0

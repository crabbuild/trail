# Changelog

All notable changes to Trail are documented in this file. Trail follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/crabbuild/trail/compare/v0.1.1...v0.2.0) (2026-08-08)


### Features

* start Trail at schema v1 and harden agent lanes ([#11](https://github.com/crabbuild/trail/issues/11)) ([733fbaa](https://github.com/crabbuild/trail/commit/733fbaaa6416f33a175f07a1196f75af308d67f6))


### Bug Fixes

* complete schema-v1 agent verification ([#13](https://github.com/crabbuild/trail/issues/13)) ([ea75f97](https://github.com/crabbuild/trail/commit/ea75f976d75edde62b8d842293a3696519967801))

## [Unreleased]

### Changed

- **Breaking:** Trail's SQLite database is now schema v1. The former v18–v21
  migration chain and compatibility fixtures are removed; existing non-v1
  workspaces must be backed up and reinitialized with `trail init --force`.

### Fixed

- Terminal-agent `--workdir-mode auto` now selects a supported transparent COW
  backend for environment-backed tasks, while retaining native/portable
  fallback on hosts without one.
- Agent apply releases its temporary layered-workdir mount before checking
  merge readiness, so an automatic COW lane no longer reports its own mount as
  an active writer.
- Backup restore re-secures private `.trail` directories and permits the
  restored changed-path scope to rebind to the current host on its next daemon
  startup.
- Lane archive and unarchive daemon requests no longer send an unexpected JSON
  body, and interrupted observer retirement with a failed owner can be reopened
  and resumed instead of being reported as a corrupt schema.

## [0.1.1] - 2026-07-29

### Added

- Added `trail upgrade` for installation-aware stable upgrades through
  Homebrew or cargo-dist release installer receipts.
- Added `trail upgrade --check` and non-blocking, once-daily interactive
  update notices. Set `TRAIL_NO_UPDATE_CHECK=1` to disable automatic checks.

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
- **Breaking:** the generic merge queue is now lane-only. Use
  `trail lane merge-queue`, `/v1/lanes/merges/queue`, and
  `trail.lane_merge_queue_*`; the previous CLI, HTTP, MCP, resource, and
  `merge_queue` storage contracts are removed without aliases. Generic
  branches and refs continue through `trail merge`.
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

[Unreleased]: https://github.com/crabbuild/trail/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/crabbuild/trail/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/crabbuild/trail/releases/tag/v0.1.0

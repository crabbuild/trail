# Agent Overview

CrabDB agent support is built around isolated agent branches, optional materialized workdirs, structured patches, durable sessions and turns, test/eval gates, readiness checks, and merge queues.

## Core Commands

```sh
crabdb agent spawn doc-bot --from main
crabdb agent status doc-bot
crabdb agent diff doc-bot --patch
crabdb agent review doc-bot
crabdb agent readiness doc-bot
crabdb agent handoff doc-bot
```

## Agent Branches

Each agent branch has:

- A base change and root.
- A head change and root.
- An optional current session.
- An optional materialized workdir.
- Provider and model metadata when supplied.

## Two Ways to Change an Agent Branch

Structured patches:

```sh
crabdb agent apply-patch doc-bot --patch patch.json
```

Materialized workdir recording:

```sh
crabdb agent workdir doc-bot
crabdb agent record doc-bot -m "record workdir edits"
```

## Review and Merge

Before merge, inspect contribution, readiness, gates, approvals, and diff.

```sh
crabdb agent contribution doc-bot
crabdb merge-agent doc-bot --into main --dry-run
```

## Code Facts Used

- Agent CLI surface: `crates/crabdb/src/cli/command/agent_args.rs`
- Agent models: `crates/crabdb/src/model/agent`
- Tests: `agent_management_commands_have_backing_apis`, `agent_patch_can_merge_into_main`

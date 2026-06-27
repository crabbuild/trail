# Ignore Files and Preflight Actions

Use ignore rules for files that should not be recorded, and guardrails for proposed lane actions that need review.

## Manage Ignore Rules

```sh
crabdb ignore list
crabdb ignore add "*.local"
crabdb ignore check scratch.local
crabdb ignore remove "*.local"
```

CrabDB reads `.crabignore` and `.gitignore`, plus hardcoded protections for internal and private paths.

## What Happens to Ignored Paths

Status and normal recording skip ignored paths. Selected records and lane patches reject ignored paths unless you explicitly opt in:

```sh
crabdb record --paths fixture.local --allow-ignored -m "capture fixture"
crabdb lane apply-patch doc-bot --patch patch.json --allow-ignored
```

Structured patch JSON can also set `"allow_ignored": true`.

## Preflight an Action

```sh
crabdb guardrails check \
  --lane doc-bot \
  --action shell.exec \
  --summary "run cargo test" \
  --path README.md
```

Guardrail reports include:

- `decision`
- `reasons`
- `path_checks`
- `pending_approvals`
- `satisfied_approvals`
- optional `approval_request`

## Human Approval Flow

```sh
crabdb approvals request doc-bot \
  --action shell.exec \
  --summary "run release smoke tests"

crabdb approvals list --lane doc-bot --status pending
crabdb approvals decide <approval-id> --decision approved --reviewer alice
```

After approval, matching approval-required reasons can become allowed for that action.

## Code Facts Used

- Ignore/guardrail args: `crates/crabdb/src/cli/command/workspace_args.rs`
- Approval args: `crates/crabdb/src/cli/command/collaboration_args/approvals.rs`
- Guardrail implementation: `crates/crabdb/src/db/core/workspace/guardrails.rs`
- Tests: `local_api_and_mcp_manage_human_approval_gates`, `hardcoded_private_key_denylist_is_not_recorded`

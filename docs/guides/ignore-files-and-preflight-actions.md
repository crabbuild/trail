# Ignore Files and Preflight Actions

Use ignore rules for files that should not be recorded, and guardrails for proposed lane actions that need review.

## Manage Ignore Rules

```sh
trail ignore list
trail ignore add "*.local"
trail ignore check scratch.local
trail ignore remove "*.local"
```

Trail reads `.trailignore` and `.gitignore`, plus hardcoded protections for internal and private paths.

## What Happens to Ignored Paths

Status and normal recording skip ignored paths. Selected records and lane patches reject ignored paths unless you explicitly opt in:

```sh
trail record --paths fixture.local --allow-ignored -m "capture fixture"
trail lane apply-patch doc-bot --patch patch.json --allow-ignored
```

Structured patch JSON can also set `"allow_ignored": true`.

## Preflight an Action

```sh
trail guardrails check \
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
trail approvals request doc-bot \
  --action shell.exec \
  --summary "run release smoke tests"

trail approvals list --lane doc-bot --status pending
trail approvals decide <approval-id> --decision approved --reviewer alice
```

After approval, matching approval-required reasons can become allowed for that action.

## Code Facts Used

- Ignore/guardrail args: `trail/src/cli/command/workspace_args.rs`
- Approval args: `trail/src/cli/command/collaboration_args/approvals.rs`
- Guardrail implementation: `trail/src/db/core/workspace/guardrails.rs`
- Tests: `local_api_and_mcp_manage_human_approval_gates`, `hardcoded_private_key_denylist_is_not_recorded`

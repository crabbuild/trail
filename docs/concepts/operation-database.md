# Operation Database

CrabDB stores a history of operations rather than only snapshots. Each operation records who changed the workspace, which root it changed from and to, which paths changed, and which parent operations it follows.

## Operation Shape

The public `Operation` model contains:

- `change_id`: stable operation identifier.
- `kind`: operation type such as `Init`, `ManualRecord`, `WatchRecord`, `AgentPatch`, `AgentRecord`, `AgentMerge`, `Merge`, or `GitExport`.
- `parents`: parent operation IDs.
- `before_root` and `after_root`: root objects before and after the operation.
- `branch`: branch name.
- `actor`: human, agent, or system actor.
- `session_id`: optional durable agent session link.
- `message`: optional human-readable message.
- `changes`: file and line change summaries.
- `created_at`: timestamp.

## Roots and Refs

Branches and agent branches point to refs. Refs point to operations and roots. A root describes the full worktree state at that point.

This lets CrabDB answer questions such as:

- What changed in this operation?
- Which operation introduced this line?
- Which branch root should be materialized?
- Is an agent branch merge-ready?

## Relationship to Git

CrabDB can import from and export to Git, but it is not just a Git wrapper. It tracks stable file IDs, line IDs, agent sessions, messages, approvals, test/eval gates, trace events, and merge queue state.

## Code Facts Used

- Operation model: `crates/crabdb/src/model/domain/operations.rs`
- Ref storage: `crates/crabdb/src/db/storage/refs.rs`
- Reports: `crates/crabdb/src/model/reports/worktree.rs`, `crates/crabdb/src/model/reports/merge.rs`


# Data Types Reference

This page summarizes public types used across CLI JSON, HTTP API, MCP structured content, and the Rust API.

## IDs

- `WorkspaceId`
- `ChangeId`
- `ObjectId`
- `FileId`
- `LineId`
- `MessageId`
- `AnchorId`

`FileId` and `LineId` preserve origin information through operation IDs and local sequence values.

## Core Domain

- `Operation`: operation metadata, parent changes, roots, branch, actor, session, message, changes, timestamp.
- `Actor`: human, agent, or system.
- `FileChange`: path, old path, file ID, kind, hashes, line changes.
- `LineChange`: line ID, kind, old/new line numbers, hashes.
- `RefRecord`: ref name, change/root/operation IDs, generation, updated timestamp.
- `Message`: durable conversation or operation message.

## Objects

- `WorktreeRoot`: root map IDs, file count, total text bytes, creating operation.
- `FileEntry`: stable file metadata and content reference.
- `TextContent`: line count, byte count, representation, map roots, optional full blob.
- `LineEntry`: line ID, bytes, newline kind, hash, origin/change metadata, flags.
- `Blob`: raw bytes with content hash.

## Worktree Reports

- `InitReport`
- `RecordReport`
- `GitImportReport`
- `GitExportReport`
- `BranchReport`
- `CheckoutReport`

## Agent Reports

- `AgentDetails`
- `AgentStatusReport`
- `AgentContributionReport`
- `AgentReadinessReport`
- `AgentHandoffReport`
- `AgentPatchReport`
- `AgentRecordReport`
- `AgentTestReport`
- `AgentGateHistoryReport`

## Coordination Types

- `AgentSession`
- `AgentTurn`
- `AgentEventRecord`
- `AgentTraceSpan`
- `AgentRunState`
- `AgentApproval`
- `LeaseRecord`
- `Anchor`

## Merge and Conflict Types

- `MergeReport`
- `MergeQueueEntry`
- `MergeQueueRunReport`
- `ConflictSetSummary`
- `ConflictManualResolution`
- `ConflictResolveReport`

## Maintenance Types

- `DoctorReport`
- `FsckReport`
- `IndexRebuildReport`
- `WorktreeIndexReport`
- `GcReport`
- `BackupCreateReport`
- `BackupVerifyReport`
- `BackupRestoreReport`

## Code Facts Used

- IDs: `crates/crabdb/src/ids.rs`
- Models: `crates/crabdb/src/model`
- Reports: `crates/crabdb/src/model/reports`
- Library exports: `crates/crabdb/src/lib.rs`


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

## Lane Reports

- `LaneDetails`
- `LaneStatusReport`
- `LaneBaseStatus`
- `LaneReviewPacketReport`
- `LaneReviewEvidenceSummary`
- `LaneContributionReport`
- `LaneReadinessReport`
- `LaneHandoffReport`
- `LanePatchReport`
- `LaneRecordReport`
- `LaneRecordPreviewReport`
- `LaneRefreshPreviewReport`
- `LaneRewindReport`
- `LaneTestReport`
- `LaneGateHistoryReport`

## Coordination Types

- `LaneSession`
- `LaneTurn`
- `LaneEventRecord`
- `LaneTraceSpan`
- `LaneRunState`
- `LaneApproval`
- `LeaseRecord`
- `Anchor`

## Agent and ACP Types

- `AgentTaskReport`
- `AgentTaskStatus`
- `AgentTaskListReport`
- `AgentInboxReport`
- `AgentInboxGroup`
- `AgentInboxItem`
- `AgentInboxReviewTarget`
- `AgentTaskViewReport`
- `AgentReviewReport`
- `AgentReviewPriority`
- `AgentFocusReport`
- `AgentReviewBundleReport`
- `AgentReceiptReport`
- `AgentPrDraftReport`
- `AgentSummaryReport`
- `AgentValidationReport`
- `AgentCheckpointReport`
- `AgentCheckpointEntry`
- `AgentChangesReport`
- `AgentChangeSetReport`
- `AgentChangeCard`
- `AgentChangeGroup`
- `AgentDeltaReport`
- `AgentReviewMarker`
- `AgentNewReport`
- `AgentMarkReviewedReport`
- `AgentTimelineReport`
- `AgentTimelineItem`
- `AgentFilesReport`
- `AgentFileReport`
- `AgentFileEntry`
- `AgentFileTouch`
- `AgentStoryReport`
- `AgentStoryTurn`
- `AgentRiskReport`
- `AgentRiskLevel`
- `AgentRiskReason`
- `AgentReadyReport`
- `AgentDiagnosisReport`
- `AgentCompareReport`
- `AgentComparePath`
- `AgentWorkdirReport`
- `AgentWhyReport`
- `AgentTurnReport`
- `AgentDiffReport`
- `AgentStatusReport`
- `AgentAskReport`
- `AgentApplyReport`
- `AgentGitApplyPlan`
- `AgentSetupReport`
- `AgentRunReport`
- `AcpProviderProfile`
- `AcpInstallReport`
- `AcpDoctorReport`
- `AcpDoctorCheck`
- `AcpSessionListReport`
- `TranscriptReport`
- `TranscriptTurn`
- `TranscriptMessage`
- `StatusSuggestion`

## Merge and Conflict Types

- `MergeReport`
- `MergeQueueEntry`
- `MergeQueueRunReport`
- `ConflictSetSummary`
- `ConflictExplanation`
- `ConflictPathExplanation`
- `ConflictLineExplanation`
- `ConflictResolutionCandidate`
- `ConflictKnownResolution`
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
- `ExternalMutationAuditRecord`

## Code Facts Used

- IDs: `crates/crabdb/src/ids.rs`
- Models: `crates/crabdb/src/model`
- Reports: `crates/crabdb/src/model/reports`
- Library exports: `crates/crabdb/src/lib.rs`

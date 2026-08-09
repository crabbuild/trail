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
- `AgentBoardReport`
- `AgentBoardColumn`
- `AgentBoardItem`
- `AgentStackReport`
- `AgentStackItem`
- `AgentStackSharedPath`
- `AgentArchiveReport`
- `AgentGuideReport`
- `AgentGuideStep`
- `AgentGuideConcept`
- `AgentTaskViewReport`
- `AgentDashboardReport`
- `AgentReviewReport`
- `AgentReviewPriority`
- `AgentFocusReport`
- `AgentReviewBundleReport`
- `AgentReceiptReport`
- `AgentHandoffReport`
- `AgentPrDraftReport`
- `AgentSummaryReport`
- `AgentValidationReport`
- `AgentTestPlanReport`
- `AgentTestPlanStep`
- `AgentCheckpointReport`
- `AgentCheckpointEntry`
- `AgentChangesReport`
- `AgentChangeSetReport`
- `AgentChangeCard`
- `AgentChangeGroup`
- `AgentDeltaReport`
- `AgentReviewMarker`
- `AgentNewReport`
- `AgentReviewFlowReport`
- `AgentReviewFlowStep`
- `AgentMarkReviewedReport`
- `AgentTimelineReport`
- `AgentTimelineItem`
- `AgentFilesReport`
- `AgentFileReport`
- `AgentFileEntry`
- `AgentFileTouch`
- `AgentStoryReport`
- `AgentStoryTurn`
- `AgentToolsReport`
- `AgentToolEntry`
- `AgentToolTurnRef`
- `AgentImpactReport`
- `AgentImpactArea`
- `AgentReviewMapReport`
- `AgentReviewMapArea`
- `AgentReviewMapFile`
- `AgentFileReviewMarker`
- `AgentMarkFileReviewedReport`
- `AgentRiskReport`
- `AgentRiskLevel`
- `AgentRiskReason`
- `AgentConfidenceReport`
- `AgentConfidenceFactor`
- `AgentReadyReport`
- `AgentReviewAction`
- `AgentReviewDataReport`
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
- `AgentFinishReport`
- `AgentGitApplyPlan`
- `AgentRunReport`
- `AgentContinueReport`
- `AcpProviderProfile`
- `AcpSetupReport`
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

## Artifact Resolution Types

- `ArtifactResolutionPlanV1`: pinned executable, argv, source inputs, authority and
  policy bounds, candidate contract, limits, and validations.
- `ArtifactResolutionSnapshotV1`: immutable verified snapshot identity and provenance.
- `ArtifactResolutionAttemptReportV1`: durable fenced attempt, bounded evidence, and
  success/failure/recovery state.
- `ArtifactResolutionRequestV1` and `ArtifactResolutionCandidateV1`: ephemeral Rust-only
  executor handoff; these are intentionally not serializable.
- `ArtifactResolutionDecisionV1` and `ArtifactResolutionComponentReportV1`: one typed
  `resolved`, `reused`, or deliberately `refreshed` decision.
- `ArtifactResolutionBatchReportV1`: deterministically ordered same-source-root result.

## Code Facts Used

- IDs: `trail/src/ids.rs`
- Models: `trail/src/model`
- Reports: `trail/src/model/reports`
- Library exports: `trail/src/lib.rs`

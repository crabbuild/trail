import type { CoordinationSummary } from "../shared/coordinationSummary";

export type ReviewTone = "new" | "ready" | "warning" | "blocked" | "applied";
export type ReviewGateTone = "ok" | "warning" | "blocked" | "muted";
export type ReviewActionTone = "primary" | "default" | "danger";

export interface ReviewAction {
  action: string;
  label: string;
  description: string;
  tone: ReviewActionTone;
  disabled?: boolean | undefined;
  disabledReason?: string | undefined;
}

export interface ReviewActionGroup {
  id: "next" | "inspect" | "validate" | "recover";
  label: string;
  detail: string;
  actions: ReviewAction[];
}

export interface ReviewGate {
  id: string;
  label: string;
  value: string;
  detail: string;
  tone: ReviewGateTone;
}

export interface ReviewMetric {
  label: string;
  value: string;
  tone: ReviewGateTone;
}

export interface ReviewReadinessInput {
  taskStatus?: string | undefined;
  changedPaths: number;
  turnCount: number;
  eventCount: number;
  blockers: number;
  warnings: number;
  conflictCount: number;
  overlapCount: number;
  testRunCount: number;
  evalRunCount: number;
  coordination: CoordinationSummary;
}

export interface ReviewReadiness {
  tone: ReviewTone;
  statusLabel: string;
  headline: string;
  description: string;
  primaryAction: ReviewAction;
  actionGroups: ReviewActionGroup[];
  gates: ReviewGate[];
  metrics: ReviewMetric[];
}

export function buildReviewReadiness(input: ReviewReadinessInput): ReviewReadiness {
  const status = normalizeStatus(input.taskStatus);
  const changedPaths = count(input.changedPaths);
  const turns = count(input.turnCount);
  const events = count(input.eventCount);
  const blockers = count(input.blockers) + count(input.coordination.blockers);
  const warnings = count(input.warnings) + count(input.coordination.warnings);
  const conflictCount = Math.max(count(input.conflictCount), count(input.coordination.conflicts));
  const overlapCount = count(input.overlapCount);
  const pendingApprovals = count(input.coordination.pendingApprovals);
  const testStatus = input.coordination.latestTestStatus;
  const evalStatus = input.coordination.latestEvalStatus;
  const hasHardBlock =
    blockers > 0 ||
    conflictCount > 0 ||
    pendingApprovals > 0 ||
    input.coordination.workdirDirty ||
    input.coordination.severity === "blocked" ||
    ["blocked", "conflicted", "failed"].includes(status);
  const hasWarning =
    warnings > 0 ||
    overlapCount > 0 ||
    input.coordination.severity === "warning" ||
    (changedPaths > 0 && !testStatus);

  const tone = reviewTone(status, hasHardBlock, hasWarning, changedPaths, turns);
  const nextAction = primaryAction(input, tone, conflictCount, pendingApprovals);
  return {
    tone,
    statusLabel: statusLabel(status, tone),
    headline: headline(tone, changedPaths),
    description: description(tone, input),
    primaryAction: nextAction,
    actionGroups: reviewActionGroups(input, tone, nextAction, changedPaths, conflictCount, overlapCount, pendingApprovals),
    gates: [
      changesGate(changedPaths),
      coordinationGate(input.coordination),
      approvalsGate(pendingApprovals),
      conflictsGate(conflictCount),
      testGate(testStatus, count(input.testRunCount), changedPaths),
      evalGate(evalStatus, count(input.evalRunCount), changedPaths),
      parallelGate(overlapCount)
    ],
    metrics: [
      { label: "Changed paths", value: formatCount(changedPaths), tone: changedPaths > 0 ? "ok" : "muted" },
      { label: "Turns", value: formatCount(turns), tone: turns > 0 ? "ok" : "muted" },
      { label: "Events", value: formatCount(events), tone: events > 0 ? "ok" : "muted" },
      { label: "Open gates", value: formatCount(blockers + warnings + conflictCount + pendingApprovals), tone: openGateTone(blockers + conflictCount + pendingApprovals, warnings) }
    ]
  };
}

function reviewActionGroups(
  input: ReviewReadinessInput,
  tone: ReviewTone,
  nextAction: ReviewAction,
  changedPaths: number,
  conflictCount: number,
  overlapCount: number,
  pendingApprovals: number
): ReviewActionGroup[] {
  const blocked = tone === "blocked";
  const hasChanges = changedPaths > 0;
  const hasParallelRisk = conflictCount > 0 || overlapCount > 0;
  return [
    {
      id: "next",
      label: "Next step",
      detail: "The safest action for the current review state.",
      actions: [
        nextAction,
        ...(nextAction.action === "refresh"
          ? []
          : [{ action: "refresh", label: "Refresh", description: "Fetch the latest CrabDB review state.", tone: "default" as const }])
      ]
    },
    {
      id: "inspect",
      label: "Inspect",
      detail: "Open the evidence behind this review.",
      actions: [
        {
          action: "openDiff",
          label: "Open diff",
          description: hasChanges ? "Inspect changed files." : "No changed paths are recorded yet.",
          tone: hasChanges ? "default" : "default",
          disabled: !hasChanges,
          disabledReason: "No changed paths are recorded yet."
        },
        {
          action: "compareTasks",
          label: "Compare tasks",
          description: hasParallelRisk ? "Inspect conflicts or overlapping task changes." : "Compare this lane with nearby task state.",
          tone: hasParallelRisk ? "primary" : "default"
        },
        { action: "openWorkdir", label: "Open workdir", description: "Inspect this lane in VS Code.", tone: "default" }
      ]
    },
    {
      id: "validate",
      label: "Validate",
      detail: "Run gates and prepare the change for apply or queue.",
      actions: [
        { action: "runTests", label: "Run tests", description: "Capture a test gate result for this task.", tone: nextAction.action === "runTests" ? "primary" : "default" },
        { action: "runEvals", label: "Run evals", description: "Capture eval evidence when agent behavior changed.", tone: "default" },
        {
          action: "dryRunApply",
          label: "Dry-run apply",
          description: blocked ? "Resolve blockers before previewing workspace changes." : hasChanges ? "Preview workspace changes safely." : "No changed paths are recorded yet.",
          tone: nextAction.action === "dryRunApply" ? "primary" : "default",
          disabled: blocked || !hasChanges,
          disabledReason: blocked ? "Resolve review blockers first." : "No changed paths are recorded yet."
        },
        {
          action: "queueMerge",
          label: "Queue merge",
          description: blocked || pendingApprovals > 0 ? "Clear blockers before queueing merge." : "Queue this lane for merge coordination.",
          tone: "default",
          disabled: blocked || pendingApprovals > 0 || !hasChanges,
          disabledReason: blocked || pendingApprovals > 0 ? "Clear review blockers first." : "No changed paths are recorded yet."
        }
      ]
    },
    {
      id: "recover",
      label: "Recover",
      detail: "Explicit recovery actions for this lane.",
      actions: [
        { action: "rewind", label: "Rewind", description: "Return the lane to a previous CrabDB checkpoint.", tone: "default" },
        { action: "preserveFailedAttempt", label: "Preserve and rewind", description: "Keep failed work as evidence before rewinding.", tone: "default" },
        { action: "removeTask", label: "Remove task", description: "Delete this task record from the CrabDB view.", tone: "danger" }
      ]
    }
  ];
}

function reviewTone(status: string, hasHardBlock: boolean, hasWarning: boolean, changedPaths: number, turns: number): ReviewTone {
  if (status === "applied") {
    return "applied";
  }
  if (hasHardBlock) {
    return "blocked";
  }
  if (hasWarning) {
    return "warning";
  }
  if (status === "ready" || changedPaths > 0 || turns > 0) {
    return "ready";
  }
  return "new";
}

function headline(tone: ReviewTone, changedPaths: number): string {
  switch (tone) {
    case "applied":
      return "Applied to the workspace";
    case "blocked":
      return "Blocked before apply";
    case "warning":
      return "Review recommended";
    case "ready":
      return changedPaths > 0 ? "Ready for dry-run" : "Transcript ready";
    default:
      return "Waiting for evidence";
  }
}

function description(tone: ReviewTone, input: ReviewReadinessInput): string {
  switch (tone) {
    case "applied":
      return "The review record is retained so the change can still be inspected or compared.";
    case "blocked":
      return "Resolve blockers, approvals, conflicts, or dirty worktree state before changing the main workspace.";
    case "warning":
      return "The lane can be inspected, but CrabDB still has warnings worth clearing before queueing a merge.";
    case "ready":
      return input.changedPaths > 0
        ? "No blocking gates are reported. Run a dry-run apply before making workspace changes."
        : "No file changes are recorded. Review the transcript before starting the next turn.";
    default:
      return "Start a prompt or attach context so CrabDB can build a reviewable task record.";
  }
}

function primaryAction(
  input: ReviewReadinessInput,
  tone: ReviewTone,
  conflictCount: number,
  pendingApprovals: number
): ReviewAction {
  if (conflictCount > 0 || input.overlapCount > 0) {
    return {
      action: "compareTasks",
      label: "Compare tasks",
      description: "Inspect overlap before applying",
      tone: "primary"
    };
  }
  if (pendingApprovals > 0) {
    return {
      action: "focusTranscript",
      label: "Find approval",
      description: "Resolve the pending tool request",
      tone: "primary"
    };
  }
  if (input.coordination.workdirDirty) {
    return {
      action: "openWorkdir",
      label: "Open workdir",
      description: "Inspect unrecorded lane changes",
      tone: "primary"
    };
  }
  if (tone === "applied") {
    return {
      action: "openDiff",
      label: "Open diff",
      description: "Inspect the applied change",
      tone: "primary"
    };
  }
  if (input.changedPaths > 0 && !input.coordination.latestTestStatus) {
    return {
      action: "runTests",
      label: "Run tests",
      description: "Capture a gate result",
      tone: "primary"
    };
  }
  if (input.changedPaths > 0) {
    return {
      action: "dryRunApply",
      label: "Dry-run apply",
      description: "Preview workspace changes safely",
      tone: "primary"
    };
  }
  if (input.turnCount > 0) {
    return {
      action: "focusTranscript",
      label: "Review transcript",
      description: "Inspect the agent conversation",
      tone: "primary"
    };
  }
  return {
    action: "refresh",
    label: "Refresh status",
    description: "Ask CrabDB for the latest task state",
    tone: "primary"
  };
}

function changesGate(changedPaths: number): ReviewGate {
  return {
    id: "changes",
    label: "Changes",
    value: changedPaths > 0 ? `${formatCount(changedPaths)} path${changedPaths === 1 ? "" : "s"}` : "None",
    detail: changedPaths > 0 ? "Changed paths are available for diff review." : "No changed paths are recorded for this task.",
    tone: changedPaths > 0 ? "ok" : "muted"
  };
}

function coordinationGate(summary: CoordinationSummary): ReviewGate {
  if (summary.severity === "blocked") {
    return {
      id: "coordination",
      label: "Coordination",
      value: "Blocked",
      detail: "CrabDB reports a blocker in the lane coordination state.",
      tone: "blocked"
    };
  }
  if (summary.severity === "warning") {
    return {
      id: "coordination",
      label: "Coordination",
      value: "Warning",
      detail: "CrabDB reports coordination warnings for this lane.",
      tone: "warning"
    };
  }
  return {
    id: "coordination",
    label: "Coordination",
    value: "Clear",
    detail: "No CrabDB coordination blockers are reported.",
    tone: "ok"
  };
}

function approvalsGate(pendingApprovals: number): ReviewGate {
  return {
    id: "approvals",
    label: "Approvals",
    value: pendingApprovals > 0 ? formatCount(pendingApprovals) : "None",
    detail: pendingApprovals > 0 ? "Resolve pending tool approval requests before applying." : "No approval requests are pending.",
    tone: pendingApprovals > 0 ? "blocked" : "ok"
  };
}

function conflictsGate(conflictCount: number): ReviewGate {
  return {
    id: "conflicts",
    label: "Conflicts",
    value: conflictCount > 0 ? formatCount(conflictCount) : "None",
    detail: conflictCount > 0 ? "Open conflict sets need inspection before merge." : "No conflict sets are reported.",
    tone: conflictCount > 0 ? "blocked" : "ok"
  };
}

function testGate(status: string | undefined, runCount: number, changedPaths: number): ReviewGate {
  if (!status) {
    return {
      id: "tests",
      label: "Tests",
      value: runCount > 0 ? `${formatCount(runCount)} recorded` : "Not run",
      detail: changedPaths > 0 ? "Run tests before applying code changes." : "No code changes are recorded yet.",
      tone: changedPaths > 0 ? "warning" : "muted"
    };
  }
  return {
    id: "tests",
    label: "Tests",
    value: status,
    detail: "Latest test gate status reported by CrabDB.",
    tone: gateStatusTone(status)
  };
}

function evalGate(status: string | undefined, runCount: number, changedPaths: number): ReviewGate {
  if (!status) {
    return {
      id: "evals",
      label: "Evals",
      value: runCount > 0 ? `${formatCount(runCount)} recorded` : "Not run",
      detail: changedPaths > 0 ? "Run evals when this task changes agent behavior." : "No eval gate is recorded yet.",
      tone: "muted"
    };
  }
  return {
    id: "evals",
    label: "Evals",
    value: status,
    detail: "Latest eval gate status reported by CrabDB.",
    tone: gateStatusTone(status)
  };
}

function parallelGate(overlapCount: number): ReviewGate {
  return {
    id: "parallel",
    label: "Parallel work",
    value: overlapCount > 0 ? formatCount(overlapCount) : "None",
    detail: overlapCount > 0 ? "Other tasks touch the same paths." : "No overlapping task changes are reported.",
    tone: overlapCount > 0 ? "warning" : "ok"
  };
}

function gateStatusTone(status: string): ReviewGateTone {
  const normalized = normalizeStatus(status);
  if (["passed", "pass", "success", "succeeded", "ok", "green", "completed"].includes(normalized)) {
    return "ok";
  }
  if (["failed", "fail", "error", "blocked", "red", "cancelled"].includes(normalized)) {
    return "blocked";
  }
  return "warning";
}

function statusLabel(status: string, tone: ReviewTone): string {
  if (status && status !== "new") {
    return status;
  }
  return tone === "new" ? "new" : tone;
}

function openGateTone(hard: number, warnings: number): ReviewGateTone {
  if (hard > 0) {
    return "blocked";
  }
  if (warnings > 0) {
    return "warning";
  }
  return "ok";
}

function normalizeStatus(value: string | undefined): string {
  return String(value || "new")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function count(value: number): number {
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

function formatCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(count(value));
}

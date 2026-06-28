import assert from "node:assert/strict";
import test from "node:test";
import { buildReviewReadiness } from "../webview/reviewModel";
import type { CoordinationSummary } from "../shared/coordinationSummary";

const clearCoordination: CoordinationSummary = {
  severity: "ok",
  labels: [],
  issues: [],
  blockers: 0,
  warnings: 0,
  conflicts: 0,
  pendingApprovals: 0,
  queuedMerges: 0,
  changedPaths: 0,
  workdirDirty: false
};

function reviewInput(overrides: Partial<Parameters<typeof buildReviewReadiness>[0]> = {}) {
  return {
    taskStatus: "ready",
    changedPaths: 2,
    turnCount: 3,
    eventCount: 18,
    blockers: 0,
    warnings: 0,
    conflictCount: 0,
    overlapCount: 0,
    testRunCount: 1,
    evalRunCount: 0,
    coordination: clearCoordination,
    ...overrides
  };
}

test("marks ready changes as dry-runable when gates are clear", () => {
  const readiness = buildReviewReadiness(
    reviewInput({
      coordination: {
        ...clearCoordination,
        latestTestStatus: "passed"
      }
    })
  );

  assert.equal(readiness.tone, "ready");
  assert.equal(readiness.primaryAction.action, "dryRunApply");
  assert.equal(readiness.gates.find((gate) => gate.id === "tests")?.tone, "ok");
  assert.equal(readiness.actionGroups.find((group) => group.id === "next")?.actions[0]?.action, "dryRunApply");
  assert.equal(readiness.actionGroups.find((group) => group.id === "validate")?.actions.find((action) => action.action === "dryRunApply")?.disabled, false);
  assert.equal(readiness.actionGroups.find((group) => group.id === "validate")?.actions.find((action) => action.action === "queueMerge")?.disabled, false);
});

test("promotes missing tests to the primary action without hard-blocking", () => {
  const readiness = buildReviewReadiness(reviewInput({ testRunCount: 0 }));

  assert.equal(readiness.tone, "warning");
  assert.equal(readiness.primaryAction.action, "runTests");
  assert.equal(readiness.gates.find((gate) => gate.id === "tests")?.tone, "warning");
});

test("prioritizes conflicts and overlaps before apply actions", () => {
  const readiness = buildReviewReadiness(
    reviewInput({
      overlapCount: 1,
      coordination: {
        ...clearCoordination,
        severity: "blocked",
        conflicts: 2,
        latestTestStatus: "passed"
      }
    })
  );

  assert.equal(readiness.tone, "blocked");
  assert.equal(readiness.primaryAction.action, "compareTasks");
  assert.equal(readiness.gates.find((gate) => gate.id === "conflicts")?.value, "2");
  assert.equal(readiness.actionGroups.find((group) => group.id === "inspect")?.actions.find((action) => action.action === "compareTasks")?.tone, "primary");
  assert.equal(readiness.actionGroups.find((group) => group.id === "validate")?.actions.find((action) => action.action === "dryRunApply")?.disabled, true);
  assert.match(readiness.actionGroups.find((group) => group.id === "validate")?.actions.find((action) => action.action === "queueMerge")?.disabledReason || "", /blockers/);
});

test("routes pending approvals back to the transcript", () => {
  const readiness = buildReviewReadiness(
    reviewInput({
      coordination: {
        ...clearCoordination,
        severity: "blocked",
        pendingApprovals: 1,
        latestTestStatus: "passed"
      }
    })
  );

  assert.equal(readiness.tone, "blocked");
  assert.equal(readiness.primaryAction.action, "focusTranscript");
  assert.equal(readiness.gates.find((gate) => gate.id === "approvals")?.tone, "blocked");
  assert.equal(readiness.actionGroups.find((group) => group.id === "validate")?.actions.find((action) => action.action === "queueMerge")?.disabled, true);
});

test("handles empty tasks as waiting for evidence", () => {
  const readiness = buildReviewReadiness(
    reviewInput({
      taskStatus: "new",
      changedPaths: 0,
      turnCount: 0,
      eventCount: 0,
      testRunCount: 0
    })
  );

  assert.equal(readiness.tone, "new");
  assert.equal(readiness.primaryAction.action, "refresh");
  assert.equal(readiness.metrics.find((metric) => metric.label === "Changed paths")?.tone, "muted");
  assert.deepEqual(readiness.actionGroups.find((group) => group.id === "next")?.actions.map((action) => action.action), ["refresh"]);
  assert.equal(readiness.actionGroups.find((group) => group.id === "inspect")?.actions.find((action) => action.action === "openDiff")?.disabled, true);
  assert.equal(readiness.actionGroups.find((group) => group.id === "validate")?.actions.find((action) => action.action === "dryRunApply")?.disabledReason, "No changed paths are recorded yet.");
});

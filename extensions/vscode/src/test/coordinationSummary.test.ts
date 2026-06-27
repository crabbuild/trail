import assert from "node:assert/strict";
import test from "node:test";
import { coordinationSummaryFromSources } from "../shared/coordinationSummary";

test("coordinationSummaryFromSources surfaces blockers, conflicts, approvals, and stale bases", () => {
  const summary = coordinationSummaryFromSources({
    readiness: {
      blockers: [
        {
          code: "open_conflicts",
          message: "2 merge conflict sets are still open",
          details: { conflict_set_ids: ["conflict-a", "conflict-b"] }
        },
        {
          code: "pending_approvals",
          message: "1 approval request is still pending",
          details: { approval_ids: ["approval-a"] }
        }
      ],
      warnings: [
        {
          code: "stale_lane_base",
          message: "lane started 14 operations behind main",
          details: { operations_behind: 14 }
        },
        {
          code: "missing_latest_test",
          message: "no test gate has been recorded for this lane"
        }
      ],
      changed_paths: [{ path: "src/app.ts" }],
      queued_merges: 1,
      latest_eval: { status: "eval_passed" }
    }
  });

  assert.equal(summary.severity, "blocked");
  assert.equal(summary.conflicts, 2);
  assert.equal(summary.pendingApprovals, 1);
  assert.equal(summary.queuedMerges, 1);
  assert.equal(summary.changedPaths, 1);
  assert.equal(summary.staleBaseOperations, 14);
  assert.equal(summary.latestEvalStatus, "eval_passed");
  assert.ok(summary.labels.includes("2 blocked"));
  assert.ok(summary.labels.includes("2 conflicts"));
  assert.ok(summary.labels.includes("1 approval"));
  assert.ok(summary.labels.includes("stale +14"));
});

test("coordinationSummaryFromSources reports warning-only states without blockers", () => {
  const summary = coordinationSummaryFromSources({
    warnings: [
      {
        code: "missing_latest_eval",
        message: "no eval gate has been recorded for this lane"
      }
    ],
    workdir_state: "Clean"
  });

  assert.equal(summary.severity, "warning");
  assert.equal(summary.blockers, 0);
  assert.equal(summary.warnings, 1);
  assert.deepEqual(summary.labels, ["missing eval", "1 warning"]);
});

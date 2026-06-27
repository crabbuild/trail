import assert from "node:assert/strict";
import test from "node:test";
import {
  labelForStopReason,
  promptCompletionNode,
  statusForStopReason
} from "../shared/promptCompletion";
import type { RenderReduceContext } from "../shared/renderModel";

const context: RenderReduceContext = {
  taskId: "task-1",
  lane: "lane-1",
  acpSessionId: "sess-1",
  currentTurnId: "turn-1",
  provider: "provider",
  now: () => "2026-06-27T00:00:00.000Z"
};

test("maps successful prompt completion to pending checkpoint state", () => {
  const node = promptCompletionNode({ stopReason: "end_turn" }, context);

  assert.equal(node.kind, "completion");
  assert.equal(node.id, "completion:turn-1");
  assert.equal(node.status, "pending");
  assert.equal(node.stopReason, "end_turn");
  assert.equal(node.checkpointPending, true);
});

test("maps cancellation to cancelled completion state", () => {
  const node = promptCompletionNode({ stopReason: "cancelled" }, context);

  assert.equal(node.status, "cancelled");
  assert.equal(node.checkpointPending, false);
});

test("maps non-success stop reasons to failed completion state", () => {
  for (const reason of ["max_tokens", "max_turn_requests", "refusal", "unknown"]) {
    assert.equal(statusForStopReason(reason), "failed");
    assert.equal(labelForStopReason(reason).length > 0, true);
  }
});

test("handles malformed prompt responses as unknown failures", () => {
  const node = promptCompletionNode({}, context);

  assert.equal(node.status, "failed");
  assert.equal(node.stopReason, "unknown");
});

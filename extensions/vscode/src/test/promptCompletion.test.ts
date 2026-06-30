import assert from "node:assert/strict";
import test from "node:test";
import {
  labelForStopReason,
  promptCompletionNode,
  statusForStopReason
} from "../shared/promptCompletion";
import { finalizeAcpLiveTurnNodes, finalizeAcpLiveTurnPatches } from "../shared/renderFinalization";
import type { RenderNode, RenderReduceContext } from "../shared/renderModel";

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

test("finalizes active live turn presentation nodes after successful completion", () => {
  const nodes = turnFinalizationFixture();

  const next = finalizeAcpLiveTurnNodes(nodes, "turn-1", "pending", "2026-06-27T00:01:00.000Z");

  const message = findNode(next, "message:assistant:one", "message");
  assert.equal(message.status, "completed");
  assert.equal(message.streaming, false);
  assert.equal(message.updatedAt, "2026-06-27T00:01:00.000Z");

  const plan = findNode(next, "plan:turn-1", "plan");
  assert.equal(plan.status, "completed");
  assert.deepEqual(
    plan.entries.map((entry) => entry.status),
    ["completed", "failed", "completed"]
  );

  const tool = findNode(next, "tool:run", "tool");
  assert.equal(tool.status, "completed");
  assert.equal(tool.toolStatus, "completed");
  assert.equal((tool.content[0] as Record<string, unknown>).status, "completed");
  assert.equal((tool.content[1] as Record<string, unknown>).status, "completed");
  assert.equal((tool.content[2] as Record<string, unknown>).state, "completed");

  const terminal = findNode(next, "terminal:run:term-1", "terminal");
  assert.equal(terminal.status, "completed");
  assert.equal(terminal.terminalStatus, "completed");

  const providerEvent = findNode(next, "unknown:provider-progress", "unknown");
  assert.equal(providerEvent.status, "completed");

  const failedTool = findNode(next, "tool:failed", "tool");
  assert.equal(failedTool.status, "failed");
  assert.equal(failedTool.toolStatus, "failed");

  const approval = findNode(next, "approval:perm-1", "approval");
  assert.equal(approval.status, "pending");
});

test("finalization emits patches for active live turn nodes and cancels pending approvals on cancellation", () => {
  const nodes = turnFinalizationFixture();

  const patches = finalizeAcpLiveTurnPatches(nodes, "turn-1", "cancelled", "2026-06-27T00:02:00.000Z");
  const changedIds = patches.map((patch) => patch.node?.id).filter(Boolean);

  assert.deepEqual(
    changedIds.sort(),
    [
      "approval:perm-1",
      "message:assistant:one",
      "plan:turn-1",
      "terminal:run:term-1",
      "tool:run",
      "unknown:provider-progress"
    ].sort()
  );
  const approvalPatch = patches.find((patch) => patch.node?.id === "approval:perm-1");
  assert.equal(approvalPatch?.node?.status, "cancelled");
});

function turnFinalizationFixture(): RenderNode[] {
  const base = {
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    provider: "provider",
    source: "acp-live" as const,
    status: "in_progress" as const
  };
  const tool: Extract<RenderNode, { kind: "tool" }> = {
    ...base,
    id: "tool:run",
    kind: "tool",
    acpToolCallId: "run",
    toolCallId: "run",
    title: "Run tests",
    toolKind: "execute",
    toolStatus: "in_progress",
    locations: [],
    content: [
      {
        type: "terminal",
        terminalId: "term-1",
        status: "in_progress",
        stdout: "running"
      },
      {
        type: "terminal",
        terminalId: "term-2",
        stdout: "no explicit status"
      },
      {
        type: "terminal",
        terminalId: "term-3",
        state: "running",
        stdout: "state-only status"
      }
    ]
  };
  return [
    {
      ...base,
      id: "message:assistant:one",
      kind: "message",
      role: "assistant",
      content: [{ type: "text", text: "Working" }],
      text: "Working",
      streaming: true
    },
    {
      ...base,
      id: "plan:turn-1",
      kind: "plan",
      entries: [
        { title: "Run tests", status: "in_progress" },
        { title: "Keep failure evidence", status: "failed" },
        { title: "Collect output", status: "running" }
      ]
    },
    tool,
    {
      ...base,
      id: "terminal:run:term-1",
      kind: "terminal",
      acpToolCallId: "run",
      terminalId: "term-1",
      terminalStatus: "running",
      stdout: "running"
    },
    {
      ...base,
      id: "approval:perm-1",
      kind: "approval",
      requestId: "perm-1",
      title: "Run command",
      tool,
      options: [{ optionId: "allow", label: "Allow" }],
      status: "pending"
    },
    {
      ...base,
      id: "unknown:provider-progress",
      kind: "unknown",
      label: "Unsupported ACP update: provider_progress",
      payload: {
        sessionUpdate: "provider_progress",
        detail: "still running"
      }
    },
    {
      ...base,
      id: "tool:failed",
      kind: "tool",
      acpToolCallId: "failed",
      toolCallId: "failed",
      title: "Failed command",
      toolKind: "execute",
      status: "failed",
      toolStatus: "failed",
      locations: [],
      content: []
    },
    {
      ...base,
      id: "message:assistant:other-turn",
      kind: "message",
      turnId: "turn-2",
      role: "assistant",
      content: [{ type: "text", text: "Other" }],
      text: "Other",
      streaming: true
    }
  ];
}

function findNode<TKind extends RenderNode["kind"]>(
  nodes: RenderNode[],
  id: string,
  kind: TKind
): Extract<RenderNode, { kind: TKind }> {
  const node = nodes.find((candidate) => candidate.id === id);
  assert.equal(node?.kind, kind);
  return node as Extract<RenderNode, { kind: TKind }>;
}

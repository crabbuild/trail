import assert from "node:assert/strict";
import test from "node:test";
import type { RenderNode, RenderPatch } from "../shared/renderModel";
import {
  applyRenderPatchesLocally,
  changedRenderNodes,
  changedRenderNodesFromPatches,
  hasTimelineStructuralChange,
  isHydratableNodePatchPayload,
  isLiveNodePatchPayload,
  isStreamingTextPatchPayload,
  parseBaseRenderRevision,
  parseRenderRevision,
  renderPatchBatchDecision,
  shouldAcceptRenderRevision
} from "../webview/renderPatchModel";

const base = {
  taskId: "task-1",
  lane: "lane-1",
  source: "acp-live" as const,
  status: "in_progress" as const
};

function messageNode(id: string, text: string): Extract<RenderNode, { kind: "message" }> {
  return {
    ...base,
    id,
    kind: "message",
    role: "assistant",
    acpMessageId: id.replace(/^message:assistant:/, ""),
    content: [{ type: "text", text }],
    text,
    streaming: true
  };
}

function thoughtNode(id: string, text: string): Extract<RenderNode, { kind: "thought" }> {
  return {
    ...base,
    id,
    kind: "thought",
    acpMessageId: id.replace(/^thought:/, ""),
    content: [{ type: "text", text }],
    ephemeral: true
  };
}

function terminalNode(status: RenderNode["status"] = "in_progress"): Extract<RenderNode, { kind: "terminal" }> {
  return {
    ...base,
    id: "terminal:read",
    kind: "terminal",
    status,
    terminalId: "read",
    stdout: "output"
  };
}

function toolNode(id = "tool:read", turnId = "turn-1"): Extract<RenderNode, { kind: "tool" }> {
  return {
    ...base,
    id,
    turnId,
    kind: "tool",
    toolCallId: "read",
    acpToolCallId: "read",
    title: `Read ${turnId}`,
    toolKind: "read",
    toolStatus: "completed",
    status: "completed",
    locations: [],
    content: []
  };
}

test("accepts only newer render revisions", () => {
  assert.equal(parseRenderRevision(1), 1);
  assert.equal(parseRenderRevision(0), undefined);
  assert.equal(parseRenderRevision(1.5), undefined);
  assert.equal(parseRenderRevision("2"), undefined);
  assert.equal(parseBaseRenderRevision(0), 0);
  assert.equal(parseBaseRenderRevision(2), 2);
  assert.equal(parseBaseRenderRevision(-1), undefined);
  assert.equal(parseBaseRenderRevision("2"), undefined);
  assert.equal(shouldAcceptRenderRevision(3, 2), true);
  assert.equal(shouldAcceptRenderRevision(2, 2), false);
  assert.equal(shouldAcceptRenderRevision(1, 2), false);
  assert.equal(shouldAcceptRenderRevision(undefined, 0), true);
  assert.equal(shouldAcceptRenderRevision(undefined, 2), false);
});

test("requires contiguous render patch revisions", () => {
  assert.equal(renderPatchBatchDecision(2, 3, 2), "apply");
  assert.equal(renderPatchBatchDecision(1, 3, 2), "resync");
  assert.equal(renderPatchBatchDecision(4, 5, 2), "resync");
  assert.equal(renderPatchBatchDecision(2, 2, 2), "drop");
  assert.equal(renderPatchBatchDecision(2, 1, 2), "drop");
  assert.equal(renderPatchBatchDecision(undefined, 3, 2), "drop");
  assert.equal(renderPatchBatchDecision(2, undefined, 2), "drop");
});

test("applies normalized render patches without reducer-side merging", () => {
  const first = messageNode("message:assistant:one", "Hello");
  const second = messageNode("message:assistant:two", "Second");
  const updated = messageNode("message:assistant:one", "Hello world");
  const patches: RenderPatch[] = [
    { type: "upsert", node: first },
    { type: "upsert", node: second },
    { type: "upsert", node: updated },
    { type: "remove", id: second.id }
  ];

  const nodes = applyRenderPatchesLocally([], patches);

  assert.deepEqual(nodes.map((node) => node.id), [first.id]);
  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message node");
  }
  assert.equal(node.text, "Hello world");
});

test("merges delta streamed message patches locally without losing prior text", () => {
  const first = messageNode("message:assistant:one", "Hello ");
  const second = messageNode("message:assistant:one", "world");

  const nodes = applyRenderPatchesLocally([], [
    { type: "upsert", node: first },
    { type: "upsert", node: second }
  ]);

  assert.deepEqual(nodes.map((node) => node.id), [first.id]);
  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message node");
  }
  assert.equal(node.text, "Hello world");
  assert.deepEqual(node.content, [{ type: "text", text: "Hello world" }]);
});

test("merges delta streamed thought patches locally without losing prior text", () => {
  const first = thoughtNode("thought:one", "Inspect");
  const second = thoughtNode("thought:one", " files");

  const nodes = applyRenderPatchesLocally([], [
    { type: "upsert", node: first },
    { type: "upsert", node: second }
  ]);

  assert.deepEqual(nodes.map((node) => node.id), [first.id]);
  const node = nodes[0];
  assert.equal(node?.kind, "thought");
  if (node?.kind !== "thought") {
    throw new Error("expected thought node");
  }
  assert.deepEqual(node.content, [{ type: "text", text: "Inspect files" }]);
});

test("preserves existing local timeline nodes when a patch reuses an id in another scope", () => {
  const existing = toolNode("tool:read", "turn-1");
  const incoming = toolNode("tool:read", "turn-2");

  const nodes = applyRenderPatchesLocally([existing], [{ type: "upsert", node: incoming }]);
  const changes = changedRenderNodesFromPatches([existing], [{ type: "upsert", node: incoming }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["tool:read", "tool:read:turn-2:acp-live"]
  );
  assert.equal(nodes[0], existing);
  assert.equal(nodes[1]?.kind, "tool");
  assert.equal(nodes[1]?.turnId, "turn-2");
  assert.deepEqual([...changes.addedNodeIds], ["tool:read:turn-2:acp-live"]);
  assert.deepEqual([...changes.changedNodeIds], ["tool:read:turn-2:acp-live"]);
});

test("normalizes appended duplicate ids into distinct local timeline nodes", () => {
  const existing = toolNode("tool:read", "turn-1");
  const incoming = toolNode("tool:read", "turn-2");

  const nodes = applyRenderPatchesLocally([existing], [{ type: "append", node: incoming }]);
  const changes = changedRenderNodesFromPatches([existing], [{ type: "append", node: incoming }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["tool:read", "tool:read:turn-2:acp-live"]
  );
  assert.deepEqual([...changes.addedNodeIds], ["tool:read:turn-2:acp-live"]);
  assert.deepEqual([...changes.changedNodeIds], ["tool:read:turn-2:acp-live"]);
});

test("cancels normalized duplicate additions without removing existing local nodes", () => {
  const existing = toolNode("tool:read", "turn-1");
  const incoming = toolNode("tool:read", "turn-2");

  const patches: RenderPatch[] = [
    { type: "append", node: incoming },
    { type: "remove", id: incoming.id }
  ];
  const nodes = applyRenderPatchesLocally([existing], patches);
  const changes = changedRenderNodesFromPatches([existing], patches);

  assert.deepEqual(nodes.map((node) => node.id), ["tool:read"]);
  assert.equal(nodes[0], existing);
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.changedNodeIds], []);
  assert.deepEqual([...changes.removedNodeIds], []);
});

test("keeps reused completed tool ids distinct after later local timeline nodes", () => {
  const existing = {
    ...toolNode("tool:read", "turn-1"),
    raw: { sessionUpdate: "tool_call" }
  };
  const message = {
    ...messageNode("message:assistant:after-tool", "After tool"),
    turnId: "turn-1"
  };
  const incoming = {
    ...toolNode("tool:read", "turn-1"),
    title: "Second read",
    raw: { sessionUpdate: "tool_call" }
  };

  const nodes = applyRenderPatchesLocally([existing, message], [{ type: "upsert", node: incoming }]);
  const changes = changedRenderNodesFromPatches([existing, message], [{ type: "upsert", node: incoming }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["tool:read", "message:assistant:after-tool", "tool:read:turn-1:acp-live"]
  );
  assert.deepEqual([...changes.addedNodeIds], ["tool:read:turn-1:acp-live"]);
  assert.deepEqual([...changes.changedNodeIds], ["tool:read:turn-1:acp-live"]);
});

test("keeps late local tool updates attached after later timeline nodes", () => {
  const existing = {
    ...toolNode("tool:read", "turn-1"),
    raw: { sessionUpdate: "tool_call" }
  };
  const message = {
    ...messageNode("message:assistant:after-tool", "After tool"),
    turnId: "turn-1"
  };
  const incoming = {
    ...toolNode("tool:read", "turn-1"),
    status: "failed" as const,
    toolStatus: "failed" as const,
    raw: { sessionUpdate: "tool_call_update" }
  };

  const nodes = applyRenderPatchesLocally([existing, message], [{ type: "upsert", node: incoming }]);
  const changes = changedRenderNodesFromPatches([existing, message], [{ type: "upsert", node: incoming }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["tool:read", "message:assistant:after-tool"]
  );
  assert.equal(nodes[0]?.kind === "tool" ? nodes[0].toolStatus : undefined, "failed");
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.changedNodeIds], ["tool:read"]);
});

test("keeps repeated assistant message ids after tool boundaries as separate local nodes", () => {
  const first = {
    ...messageNode("message:assistant:msg-1", "Before tool"),
    turnId: "turn-1",
    acpMessageId: "msg-1"
  };
  const tool = toolNode("tool:read", "turn-1");
  const continuation = {
    ...messageNode("message:assistant:msg-1", "After tool"),
    turnId: "turn-1",
    acpMessageId: "msg-1"
  };

  const nodes = applyRenderPatchesLocally([first, tool], [{ type: "upsert", node: continuation }]);
  const changes = changedRenderNodesFromPatches([first, tool], [{ type: "upsert", node: continuation }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:msg-1", "tool:read", "message:assistant:msg-1:2"]
  );
  assert.equal(nodes[0], first);
  assert.equal(nodes[2]?.kind, "message");
  assert.equal(nodes[2]?.kind === "message" ? nodes[2].text : undefined, "After tool");
  assert.deepEqual([...changes.addedNodeIds], ["message:assistant:msg-1:2"]);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:msg-1:2"]);
});

test("trims cumulative repeated assistant snapshots after local tool boundaries", () => {
  const first = {
    ...messageNode("message:assistant:msg-1", "Before tool. "),
    turnId: "turn-1",
    acpMessageId: "msg-1"
  };
  const tool = toolNode("tool:read", "turn-1");
  const continuation = {
    ...messageNode("message:assistant:msg-1", "Before tool. After "),
    turnId: "turn-1",
    acpMessageId: "msg-1"
  };
  const completedContinuation = {
    ...messageNode("message:assistant:msg-1", "Before tool. After tool."),
    turnId: "turn-1",
    acpMessageId: "msg-1"
  };

  const nodes = applyRenderPatchesLocally(
    [first, tool],
    [
      { type: "upsert", node: continuation },
      { type: "upsert", node: completedContinuation }
    ]
  );
  const changes = changedRenderNodesFromPatches(
    [first, tool],
    [
      { type: "upsert", node: continuation },
      { type: "upsert", node: completedContinuation }
    ]
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:msg-1", "tool:read", "message:assistant:msg-1:2"]
  );
  assert.equal(nodes[0]?.kind === "message" ? nodes[0].text : undefined, "Before tool. ");
  assert.equal(nodes[2]?.kind === "message" ? nodes[2].text : undefined, "After tool.");
  assert.deepEqual([...changes.addedNodeIds], ["message:assistant:msg-1:2"]);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:msg-1:2"]);
});

test("keeps local turn completion after late repeated assistant chunks", () => {
  const first = {
    ...messageNode("message:assistant:msg-late-completion", "Before completion. "),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    acpMessageId: "msg-late-completion",
    timelineOrder: 1
  };
  const completion: RenderNode = {
    ...base,
    id: "completion:turn-1",
    kind: "completion",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    status: "pending",
    stopReason: "end_turn",
    label: "Turn complete; checkpoint pending",
    checkpointPending: true,
    timelineOrder: 3
  };
  const tool = {
    ...toolNode("tool:tool-before-late-completion", "turn-1"),
    acpSessionId: "sess-1",
    toolCallId: "tool-before-late-completion",
    acpToolCallId: "tool-before-late-completion",
    timelineOrder: 2
  };
  const late = {
    ...messageNode("message:assistant:msg-late-completion", "Before completion. Final answer."),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    acpMessageId: "msg-late-completion",
    timelineOrder: 4
  };

  const nodes = applyRenderPatchesLocally([first, tool, completion], [{ type: "upsert", node: late }]);
  const changes = changedRenderNodesFromPatches([first, tool, completion], [{ type: "upsert", node: late }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    [
      "message:assistant:msg-late-completion",
      "tool:tool-before-late-completion",
      "message:assistant:msg-late-completion:2",
      "completion:turn-1"
    ]
  );
  assert.equal(nodes[2]?.kind === "message" ? nodes[2].text : undefined, "Final answer.");
  assert.equal((nodes[3]?.timelineOrder ?? 0) > (nodes[2]?.timelineOrder ?? 0), true);
  assert.deepEqual([...changes.addedNodeIds], ["message:assistant:msg-late-completion:2"]);
  assert.deepEqual([...changes.changedNodeIds].sort(), ["completion:turn-1", "message:assistant:msg-late-completion:2"].sort());
});

test("keeps local hydrated checkpoints after late live assistant chunks", () => {
  const first = {
    ...messageNode("message:assistant:msg-late-hydrated", "Before checkpoint. "),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    acpMessageId: "msg-late-hydrated",
    timelineOrder: 1
  };
  const tool = {
    ...toolNode("tool:tool-before-hydrated-checkpoint", "turn-1"),
    acpSessionId: "sess-1",
    toolCallId: "tool-before-hydrated-checkpoint",
    acpToolCallId: "tool-before-hydrated-checkpoint",
    timelineOrder: 2
  };
  const checkpoint: RenderNode = {
    taskId: "task-1",
    lane: "lane-1",
    id: "crabdb-checkpoint:turn-1",
    kind: "checkpoint",
    turnId: "turn-1",
    source: "crabdb",
    status: "completed",
    checkpointId: "ch_late",
    label: "Checkpoint ch_late",
    timelineOrder: 3
  };
  const late = {
    ...messageNode("message:assistant:msg-late-hydrated", "Before checkpoint. Final persisted answer."),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    acpMessageId: "msg-late-hydrated",
    timelineOrder: 4
  };

  const nodes = applyRenderPatchesLocally([first, tool, checkpoint], [{ type: "upsert", node: late }]);
  const changes = changedRenderNodesFromPatches([first, tool, checkpoint], [{ type: "upsert", node: late }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    [
      "message:assistant:msg-late-hydrated",
      "tool:tool-before-hydrated-checkpoint",
      "message:assistant:msg-late-hydrated:2",
      "crabdb-checkpoint:turn-1"
    ]
  );
  assert.equal(nodes[2]?.kind === "message" ? nodes[2].text : undefined, "Final persisted answer.");
  assert.equal((nodes[3]?.timelineOrder ?? 0) > (nodes[2]?.timelineOrder ?? 0), true);
  assert.deepEqual([...changes.addedNodeIds], ["message:assistant:msg-late-hydrated:2"]);
  assert.deepEqual([...changes.changedNodeIds].sort(), ["crabdb-checkpoint:turn-1", "message:assistant:msg-late-hydrated:2"].sort());
});

test("keeps anonymous assistant chunks after tool boundaries as separate local nodes", () => {
  const first = {
    ...messageNode("message:assistant:anonymous", "Before tool"),
    turnId: "turn-1",
    acpMessageId: undefined
  };
  const tool = toolNode("tool:read", "turn-1");
  const continuation = {
    ...messageNode("message:assistant:anonymous", "After tool"),
    turnId: "turn-1",
    acpMessageId: undefined
  };

  const nodes = applyRenderPatchesLocally([first, tool], [{ type: "upsert", node: continuation }]);
  const changes = changedRenderNodesFromPatches([first, tool], [{ type: "upsert", node: continuation }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:anonymous", "tool:read", "message:assistant:anonymous:2"]
  );
  assert.equal(nodes[0], first);
  assert.equal(nodes[2]?.kind, "message");
  assert.equal(nodes[2]?.kind === "message" ? nodes[2].text : undefined, "After tool");
  assert.deepEqual([...changes.addedNodeIds], ["message:assistant:anonymous:2"]);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:anonymous:2"]);
});

test("reports changed, added, and removed render nodes", () => {
  const first = messageNode("message:assistant:one", "Hello");
  const removed = messageNode("message:assistant:removed", "Gone");
  const updated = messageNode("message:assistant:one", "Hello world");
  const added = messageNode("message:assistant:added", "New");

  const changes = changedRenderNodes(
    new Map([first, removed].map((node) => [node.id, node])),
    [updated, added]
  );

  assert.deepEqual([...changes.changedNodeIds].sort(), [added.id, first.id].sort());
  assert.deepEqual([...changes.addedNodeIds], [added.id]);
  assert.deepEqual([...changes.removedNodeIds], [removed.id]);
});

test("reports patch changes without scanning the rendered node array", () => {
  const existing = messageNode("message:assistant:one", "Hello");
  const removed = messageNode("message:assistant:removed", "Gone");
  const updated = messageNode("message:assistant:one", "Hello world");
  const added = messageNode("message:assistant:added", "New");

  const changes = changedRenderNodesFromPatches([existing, removed], [
    { type: "upsert", node: updated },
    { type: "upsert", node: added },
    { type: "remove", id: removed.id }
  ]);

  assert.deepEqual([...changes.changedNodeIds].sort(), [added.id, existing.id].sort());
  assert.deepEqual([...changes.addedNodeIds], [added.id]);
  assert.deepEqual([...changes.removedNodeIds], [removed.id]);
});

test("cancels transient add/remove pairs in patch-derived changes", () => {
  const transient = messageNode("message:assistant:temp", "Temp");

  const changes = changedRenderNodesFromPatches([], [
    { type: "upsert", node: transient },
    { type: "remove", id: transient.id }
  ]);

  assert.deepEqual([...changes.changedNodeIds], []);
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.removedNodeIds], []);
});

test("distinguishes card-only changes from timeline structural changes", () => {
  const existing = {
    ...messageNode("message:assistant:one", "Hello"),
    turnId: "turn-1",
    timelineOrder: 1
  };
  const contentOnly = {
    ...existing,
    text: "Hello world",
    content: [{ type: "text" as const, text: "Hello world" }]
  };
  const contentChanges = changedRenderNodesFromPatches([existing], [{ type: "replace", node: contentOnly }]);

  assert.equal(hasTimelineStructuralChange([existing], [contentOnly], contentChanges), false);

  const moved = { ...contentOnly, timelineOrder: 3 };
  const movedChanges = changedRenderNodesFromPatches([existing], [{ type: "replace", node: moved }]);
  assert.equal(hasTimelineStructuralChange([existing], [moved], movedChanges), true);

  const regrouped = { ...contentOnly, turnId: "turn-2" };
  const regroupedChanges = changedRenderNodesFromPatches([existing], [{ type: "replace", node: regrouped }]);
  assert.equal(hasTimelineStructuralChange([existing], [regrouped], regroupedChanges), true);

  const completed = { ...contentOnly, status: "completed" as const, streaming: false };
  const completedChanges = changedRenderNodesFromPatches([existing], [{ type: "replace", node: completed }]);
  assert.equal(hasTimelineStructuralChange([existing], [completed], completedChanges), true);
});

test("detects structural updated-time changes when created time is malformed", () => {
  const existing = {
    ...messageNode("message:assistant:bad-created", "Hello"),
    createdAt: "not-a-date",
    updatedAt: "2026-06-27T00:00:03.000Z"
  };
  const movedEarlier = {
    ...existing,
    updatedAt: "2026-06-27T00:00:01.000Z"
  };
  const changes = changedRenderNodesFromPatches([existing], [{ type: "replace", node: movedEarlier }]);

  assert.equal(hasTimelineStructuralChange([existing], [movedEarlier], changes), true);
});

test("identifies streaming text patch payloads narrowly", () => {
  assert.equal(isStreamingTextPatchPayload({ type: "upsert", node: messageNode("message:assistant:one", "Hi") }), true);
  assert.equal(
    isStreamingTextPatchPayload({
      type: "upsert",
      node: {
        ...messageNode("message:assistant:done", "Done"),
        status: "completed",
        streaming: false
      }
    }),
    false
  );
  assert.equal(isStreamingTextPatchPayload({ type: "remove", id: "message:assistant:one" }), false);
});

test("identifies live node patch payloads for local hydration", () => {
  assert.equal(isLiveNodePatchPayload({ type: "upsert", node: messageNode("message:assistant:one", "Hi") }), true);
  assert.equal(isLiveNodePatchPayload({ type: "upsert", node: terminalNode() }), true);
  assert.equal(isLiveNodePatchPayload({ type: "upsert", node: terminalNode("completed") }), false);
  assert.equal(
    isLiveNodePatchPayload({
      type: "upsert",
      node: {
        ...messageNode("message:assistant:image", "image"),
        content: [{ type: "image", data: "abc", mimeType: "image/png" }]
      }
    }),
    false
  );
});

test("allows existing presentation node completion patches to hydrate locally", () => {
  assert.equal(
    isHydratableNodePatchPayload({
      type: "upsert",
      node: {
        ...messageNode("message:assistant:done", "Done"),
        status: "completed",
        streaming: false
      }
    }),
    true
  );
  assert.equal(
    isHydratableNodePatchPayload({
      type: "upsert",
      node: {
        ...base,
        id: "thought:done",
        kind: "thought",
        status: "completed",
        content: [{ type: "text", text: "Done thinking" }],
        ephemeral: true
      }
    }),
    true
  );
  assert.equal(isHydratableNodePatchPayload({ type: "upsert", node: terminalNode("completed") }), true);
  assert.equal(
    isHydratableNodePatchPayload({
      type: "upsert",
      node: {
        ...base,
        id: "tool:done",
        kind: "tool",
        status: "completed",
        toolCallId: "done",
        title: "Read README",
        toolKind: "read",
        toolStatus: "completed",
        locations: [],
        content: []
      }
    }),
    true
  );
});

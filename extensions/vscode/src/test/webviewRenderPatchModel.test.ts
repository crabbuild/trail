import assert from "node:assert/strict";
import test from "node:test";
import type { RenderNode, RenderPatch } from "../shared/renderModel";
import {
  applyRenderPatchesLocally,
  changedRenderNodes,
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
    content: [{ type: "text", text }],
    text,
    streaming: true
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

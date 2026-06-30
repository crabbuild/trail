import assert from "node:assert/strict";
import test from "node:test";
import type { ContentBlock } from "../shared/acpTypes";
import { applyRenderPatchesAndCollect } from "../shared/acpRenderReducers";
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

function mediaMessageNode(id: string, content: ContentBlock): Extract<RenderNode, { kind: "message" }> {
  return {
    ...base,
    id,
    kind: "message",
    role: "assistant",
    acpMessageId: id.replace(/^message:assistant:/, ""),
    content: [content],
    text: `[${content.type || "content"}]`,
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

function approvalNode(toolCallId: string, title: string): Extract<RenderNode, { kind: "approval" }> {
  const tool = {
    ...toolNode(`tool:${toolCallId}`, "turn-1"),
    toolCallId,
    acpToolCallId: toolCallId,
    title
  };
  return {
    ...base,
    id: "approval:shared-request",
    kind: "approval",
    turnId: "turn-1",
    acpToolCallId: toolCallId,
    requestId: "shared-request",
    title,
    tool,
    options: [{ optionId: "allow", label: "Allow" }]
  };
}

function unknownNode(
  id: string,
  payload: unknown,
  label = "Unsupported ACP update: provider_extra"
): Extract<RenderNode, { kind: "unknown" }> {
  return {
    ...base,
    id,
    kind: "unknown",
    turnId: "turn-1",
    label,
    payload,
    status: "completed"
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

test("replaces same-id live nodes with hydrated nodes without duplicating", () => {
  const live = {
    ...messageNode("message:assistant:msg-hydrated", "Hydrated answer"),
    turnId: "turn-1",
    source: "acp-live" as const,
    status: "completed" as const,
    streaming: false
  };
  const hydrated = {
    ...live,
    source: "crabdb" as const
  };

  const nodes = applyRenderPatchesLocally([live], [{ type: "replace", node: hydrated }]);
  const changes = changedRenderNodesFromPatches([live], [{ type: "replace", node: hydrated }]);

  assert.deepEqual(nodes.map((node) => `${node.id}:${node.source}`), ["message:assistant:msg-hydrated:crabdb"]);
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.removedNodeIds], []);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:msg-hydrated"]);
  assert.equal(hasTimelineStructuralChange([live], nodes, changes), true);
});

test("reconciles cross-source hydrated message replacements by message identity locally", () => {
  const live = {
    ...messageNode("message:assistant:msg-hydrated-cross-source", "Hydrated answer"),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    status: "completed" as const,
    streaming: false,
    timelineOrder: 1
  };
  const hydrated = {
    ...live,
    id: "crabdb-message:turn-1:msg-hydrated-cross-source",
    source: "crabdb" as const,
    timelineOrder: 4,
    updatedAt: "2026-06-27T00:01:00.000Z"
  };

  const nodes = applyRenderPatchesLocally([live], [{ type: "replace", node: hydrated }]);
  const changes = changedRenderNodesFromPatches([live], [{ type: "replace", node: hydrated }]);

  assert.deepEqual(nodes.map((node) => `${node.id}:${node.source}`), [
    "message:assistant:msg-hydrated-cross-source:crabdb"
  ]);
  assert.equal(nodes[0]?.timelineOrder, 1);
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.removedNodeIds], []);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:msg-hydrated-cross-source"]);
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

test("promotes anonymous streamed message patches locally when ids arrive late", () => {
  const first = {
    ...messageNode("message:assistant:anonymous", "Hello "),
    acpMessageId: undefined
  };
  const second = {
    ...messageNode("message:assistant:msg-promoted", "world"),
    acpMessageId: "msg-promoted"
  };

  const nodes = applyRenderPatchesLocally([first], [{ type: "upsert", node: second }]);
  const changes = changedRenderNodesFromPatches([first], [{ type: "upsert", node: second }]);

  const message = nodes.find((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message");
  assert.deepEqual(nodes.map((node) => node.id), ["message:assistant:anonymous"]);
  assert.equal(message?.acpMessageId, "msg-promoted");
  assert.equal(message?.text, "Hello world");
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:anonymous"]);
});

test("promotes streamed message session scope locally when session ids arrive late", () => {
  const first = {
    ...messageNode("message:assistant:msg-session-promoted", "Hello "),
    turnId: "turn-1"
  };
  const second = {
    ...messageNode("message:assistant:msg-session-promoted", "world"),
    turnId: "turn-1",
    acpSessionId: "sess-1"
  };

  const nodes = applyRenderPatchesLocally([first], [{ type: "upsert", node: second }]);
  const changes = changedRenderNodesFromPatches([first], [{ type: "upsert", node: second }]);

  const message = nodes.find((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message");
  assert.deepEqual(nodes.map((node) => node.id), ["message:assistant:msg-session-promoted"]);
  assert.equal(message?.acpSessionId, "sess-1");
  assert.equal(message?.text, "Hello world");
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:msg-session-promoted"]);
});

test("keeps streamed message scope locally when later chunks omit session and provider", () => {
  const first = {
    ...messageNode("message:assistant:msg-scope-preserved", "Hello "),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider"
  };
  const second = {
    ...messageNode("message:assistant:msg-scope-preserved", "world"),
    turnId: "turn-1"
  };

  const nodes = applyRenderPatchesLocally([first], [{ type: "upsert", node: second }]);
  const message = nodes.find((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message");

  assert.deepEqual(nodes.map((node) => node.id), ["message:assistant:msg-scope-preserved"]);
  assert.equal(message?.acpSessionId, "sess-1");
  assert.equal(message?.provider, "test-provider");
  assert.equal(message?.text, "Hello world");
});

test("keeps streamed message id locally when later chunks omit it", () => {
  const first = {
    ...messageNode("message:assistant:msg-id-preserved", "Hello "),
    turnId: "turn-1",
    acpMessageId: "msg-id-preserved"
  };
  const second = {
    ...messageNode("message:assistant:msg-id-preserved", "world"),
    turnId: "turn-1",
    acpMessageId: undefined
  };

  const nodes = applyRenderPatchesLocally([first], [{ type: "upsert", node: second }]);
  const message = nodes.find((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message");

  assert.equal(message?.acpMessageId, "msg-id-preserved");
  assert.equal(message?.text, "Hello world");
});

test("merges empty and aliased local text blocks without placeholder corruption", () => {
  const first = {
    ...messageNode("message:assistant:empty-alias", ""),
    content: [{ type: "text", text: "" }]
  };
  const second = {
    ...messageNode("message:assistant:empty-alias", "Rendered after empty text."),
    content: [{ type: "text", value: "Rendered after empty text." } as ContentBlock]
  };

  const nodes = applyRenderPatchesLocally([], [
    { type: "upsert", node: first },
    { type: "upsert", node: second }
  ]);

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message node");
  }
  assert.equal(node.text, "Rendered after empty text.");
});

test("normalizes completed local message patches out of streaming state", () => {
  const first = messageNode("message:assistant:one", "Hello ");
  const completed = {
    ...messageNode("message:assistant:one", "Hello world"),
    status: "completed" as const,
    streaming: true
  };

  const nodes = applyRenderPatchesLocally([], [
    { type: "upsert", node: first },
    { type: "upsert", node: completed }
  ]);

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message node");
  }
  assert.equal(node.status, "completed");
  assert.equal(node.streaming, false);
  assert.equal(node.text, "Hello world");
});

test("keeps completed local message lifecycle after late active chunks", () => {
  const completed = {
    ...messageNode("message:assistant:one", "Final"),
    status: "completed" as const,
    streaming: false
  };
  const late = messageNode("message:assistant:one", "Final answer.");

  const nodes = applyRenderPatchesLocally([completed], [{ type: "upsert", node: late }]);

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message node");
  }
  assert.equal(node.status, "completed");
  assert.equal(node.streaming, false);
  assert.equal(node.text, "Final answer.");
});

test("keeps distinct non-text message patches with the same display placeholder locally", () => {
  const firstImage: ContentBlock = {
    type: "image",
    data: "first-image",
    mimeType: "image/png"
  };
  const secondImage: ContentBlock = {
    type: "image",
    data: "second-image",
    mimeType: "image/png"
  };
  const first = mediaMessageNode("message:assistant:media", firstImage);
  const second = mediaMessageNode("message:assistant:media", secondImage);

  const nodes = applyRenderPatchesLocally([], [
    { type: "upsert", node: first },
    { type: "upsert", node: second }
  ]);

  assert.deepEqual(nodes.map((node) => node.id), [first.id]);
  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected media message node");
  }
  assert.equal(node.text, "[image][image]");
  assert.deepEqual(node.content, [firstImage, secondImage]);
});

test("preserves rich content text summaries when merging local message patches", () => {
  const resourceLink: ContentBlock = {
    type: "resource_link",
    uri: "file:///workspace/README.md",
    name: "README.md",
    title: "Context file"
  };
  const first = {
    ...messageNode("message:assistant:rich", "Rendered with context.Context file (README.md)"),
    content: [
      { type: "text", text: "Rendered with context." },
      resourceLink
    ]
  };
  const second = messageNode("message:assistant:rich", " More");

  const nodes = applyRenderPatchesLocally([], [
    { type: "upsert", node: first },
    { type: "upsert", node: second }
  ]);

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message node");
  }
  assert.equal(node.text, "Rendered with context.Context file (README.md) More");
  assert.deepEqual(node.content, [
    { type: "text", text: "Rendered with context." },
    resourceLink,
    { type: "text", text: " More" }
  ]);
});

test("appends explicit terminal stdout deltas in local patches", () => {
  const first: RenderNode = {
    ...base,
    id: "terminal:stream",
    kind: "terminal",
    turnId: "turn-1",
    terminalId: "stream",
    command: "npm test",
    stdout: "line 1\n"
  };
  const delta = {
    ...first,
    command: undefined,
    stdout: undefined
  } as Extract<RenderNode, { kind: "terminal" }> & { stdoutDelta?: string };
  delta.stdoutDelta = "line 2\n";

  const nodes = applyRenderPatchesLocally([first], [{ type: "upsert", node: delta }]);
  const changes = changedRenderNodesFromPatches([first], [{ type: "upsert", node: delta }]);
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.command, "npm test");
  assert.equal(terminal?.stdout, "line 1\nline 2\n");
  assert.deepEqual([...changes.changedNodeIds], ["terminal:stream"]);
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

test("keeps streamed thought scope locally when later chunks omit session and provider", () => {
  const first = {
    ...thoughtNode("thought:scope-preserved", "Inspect"),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider"
  };
  const second = {
    ...thoughtNode("thought:scope-preserved", " files"),
    turnId: "turn-1"
  };

  const nodes = applyRenderPatchesLocally([first], [{ type: "upsert", node: second }]);
  const node = nodes[0];

  assert.equal(node?.kind, "thought");
  if (node?.kind !== "thought") {
    throw new Error("expected thought node");
  }
  assert.equal(node.acpSessionId, "sess-1");
  assert.equal(node.provider, "test-provider");
  assert.deepEqual(node.content, [{ type: "text", text: "Inspect files" }]);
});

test("keeps streamed thought id locally when later chunks omit it", () => {
  const first = {
    ...thoughtNode("thought:id-preserved", "Inspect"),
    turnId: "turn-1",
    acpMessageId: "id-preserved"
  };
  const second = {
    ...thoughtNode("thought:id-preserved", " files"),
    turnId: "turn-1",
    acpMessageId: undefined
  };

  const nodes = applyRenderPatchesLocally([first], [{ type: "upsert", node: second }]);
  const node = nodes[0];

  assert.equal(node?.kind, "thought");
  if (node?.kind !== "thought") {
    throw new Error("expected thought node");
  }
  assert.equal(node.acpMessageId, "id-preserved");
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

test("preserves existing local approval nodes when a provider reuses request ids across tools", () => {
  const existing = approvalNode("tool-a", "Run first command");
  const incoming = approvalNode("tool-b", "Run second command");

  const nodes = applyRenderPatchesLocally([existing], [{ type: "upsert", node: incoming }]);
  const changes = changedRenderNodesFromPatches([existing], [{ type: "upsert", node: incoming }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["approval:shared-request", "approval:shared-request:turn-1:acp-live"]
  );
  assert.deepEqual(
    nodes.map((node) => node.kind === "approval" ? node.tool.toolCallId : undefined),
    ["tool-a", "tool-b"]
  );
  assert.deepEqual([...changes.addedNodeIds], ["approval:shared-request:turn-1:acp-live"]);
  assert.deepEqual([...changes.changedNodeIds], ["approval:shared-request:turn-1:acp-live"]);
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

test("normalizes replacement patches that reuse message ids across local turns", () => {
  const existing = {
    ...messageNode("message:assistant:replace-dup", "First turn"),
    turnId: "turn-1",
    acpMessageId: "replace-dup",
    status: "completed" as const,
    streaming: false,
    timelineOrder: 1
  };
  const incoming = {
    ...existing,
    turnId: "turn-2",
    content: [{ type: "text", text: "Second turn" } as ContentBlock],
    text: "Second turn",
    timelineOrder: undefined
  };

  const nodes = applyRenderPatchesLocally([existing], [{ type: "replace", node: incoming }]);
  const changes = changedRenderNodesFromPatches([existing], [{ type: "replace", node: incoming }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:replace-dup", "message:assistant:replace-dup:2"]
  );
  assert.equal(nodes[0], existing);
  assert.equal(nodes[1]?.kind === "message" ? nodes[1].turnId : undefined, "turn-2");
  assert.equal(nodes[1]?.kind === "message" ? nodes[1].text : undefined, "Second turn");
  assert.deepEqual([...changes.addedNodeIds], ["message:assistant:replace-dup:2"]);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:replace-dup:2"]);
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

test("syncs local expanded terminal status from parent tool patches", () => {
  const tool: Extract<RenderNode, { kind: "tool" }> = {
    ...toolNode("tool:run-tests", "turn-1"),
    toolCallId: "run-tests",
    acpToolCallId: "run-tests",
    status: "in_progress",
    toolStatus: "in_progress",
    content: [
      {
        type: "terminal",
        terminalId: "term-1",
        status: "running",
        stdout: "running"
      }
    ]
  };
  const terminal: Extract<RenderNode, { kind: "terminal" }> = {
    ...base,
    id: "terminal:run-tests:term-1",
    kind: "terminal",
    turnId: "turn-1",
    acpToolCallId: "run-tests",
    terminalId: "term-1",
    status: "in_progress",
    terminalStatus: "running",
    stdout: "running"
  };
  const completed: Extract<RenderNode, { kind: "tool" }> = {
    ...tool,
    status: "completed",
    toolStatus: "completed",
    content: [
      {
        type: "terminal",
        terminalId: "term-1",
        status: "completed",
        stdout: "running"
      }
    ]
  };

  const nodes = applyRenderPatchesLocally([tool, terminal], [{ type: "upsert", node: completed }]);
  const changes = changedRenderNodesFromPatches([tool, terminal], [{ type: "upsert", node: completed }]);

  const syncedTerminal = nodes.find((node) => node.kind === "terminal");
  assert.equal(syncedTerminal?.kind, "terminal");
  if (syncedTerminal?.kind !== "terminal") {
    throw new Error("expected synced terminal node");
  }
  assert.equal(syncedTerminal.status, "completed");
  assert.equal(syncedTerminal.terminalStatus, "completed");
  assert.deepEqual([...changes.changedNodeIds].sort(), ["terminal:run-tests:term-1", "tool:run-tests"].sort());
});

test("syncs local expanded terminal status from parent tool ACP id aliases", () => {
  const tool: Extract<RenderNode, { kind: "tool" }> = {
    ...toolNode("tool:display-tool", "turn-1"),
    toolCallId: "display-tool",
    acpToolCallId: "provider-tool",
    status: "in_progress",
    toolStatus: "in_progress"
  };
  const terminal: Extract<RenderNode, { kind: "terminal" }> = {
    ...base,
    id: "terminal:provider-tool:term-alias",
    kind: "terminal",
    turnId: "turn-1",
    acpToolCallId: "provider-tool",
    terminalId: "term-alias",
    status: "in_progress",
    terminalStatus: "running",
    stdout: "running"
  };
  const completed: Extract<RenderNode, { kind: "tool" }> = {
    ...tool,
    status: "completed",
    toolStatus: "completed"
  };

  const nodes = applyRenderPatchesLocally([tool, terminal], [{ type: "upsert", node: completed }]);
  const changes = changedRenderNodesFromPatches([tool, terminal], [{ type: "upsert", node: completed }]);
  const syncedTerminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(syncedTerminal?.kind, "terminal");
  assert.equal(syncedTerminal?.status, "completed");
  assert.equal(syncedTerminal?.terminalStatus, "completed");
  assert.deepEqual([...changes.changedNodeIds].sort(), ["terminal:provider-tool:term-alias", "tool:display-tool"].sort());
});

test("syncs local expanded diff status from parent tool patches", () => {
  const tool: Extract<RenderNode, { kind: "tool" }> = {
    ...toolNode("tool:edit-readme", "turn-1"),
    toolCallId: "edit-readme",
    acpToolCallId: "edit-readme",
    status: "in_progress",
    toolStatus: "in_progress",
    content: [
      {
        type: "diff",
        path: "README.md",
        oldText: "before",
        newText: "after"
      }
    ]
  };
  const diff: Extract<RenderNode, { kind: "diff" }> = {
    ...base,
    id: "diff:edit-readme:README.md",
    kind: "diff",
    turnId: "turn-1",
    acpToolCallId: "edit-readme",
    status: "in_progress",
    path: "README.md",
    oldText: "before",
    newText: "after"
  };
  const completed: Extract<RenderNode, { kind: "tool" }> = {
    ...tool,
    status: "completed",
    toolStatus: "completed"
  };

  const nodes = applyRenderPatchesLocally([tool, diff], [{ type: "upsert", node: completed }]);
  const changes = changedRenderNodesFromPatches([tool, diff], [{ type: "upsert", node: completed }]);

  const syncedDiff = nodes.find((node) => node.kind === "diff");
  assert.equal(syncedDiff?.kind, "diff");
  if (syncedDiff?.kind !== "diff") {
    throw new Error("expected synced diff node");
  }
  assert.equal(syncedDiff.status, "completed");
  assert.deepEqual([...changes.changedNodeIds].sort(), ["diff:edit-readme:README.md", "tool:edit-readme"].sort());
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

test("keeps repeated assistant message ids after active tool boundaries as separate local nodes", () => {
  const first = {
    ...messageNode("message:assistant:msg-active-boundary", "Before active tool. "),
    turnId: "turn-1",
    acpMessageId: "msg-active-boundary"
  };
  const tool = {
    ...toolNode("tool:active-boundary", "turn-1"),
    status: "in_progress" as const,
    toolStatus: "in_progress" as const,
    toolCallId: "active-boundary",
    acpToolCallId: "active-boundary"
  };
  const continuation = {
    ...messageNode("message:assistant:msg-active-boundary", "Before active tool. After active tool."),
    turnId: "turn-1",
    acpMessageId: "msg-active-boundary"
  };

  const nodes = applyRenderPatchesLocally([first, tool], [{ type: "upsert", node: continuation }]);
  const changes = changedRenderNodesFromPatches([first, tool], [{ type: "upsert", node: continuation }]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:msg-active-boundary", "tool:active-boundary", "message:assistant:msg-active-boundary:2"]
  );
  assert.equal(nodes[0]?.kind === "message" ? nodes[0].text : undefined, "Before active tool. ");
  assert.equal(nodes[2]?.kind === "message" ? nodes[2].text : undefined, "After active tool.");
  assert.deepEqual([...changes.addedNodeIds], ["message:assistant:msg-active-boundary:2"]);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:msg-active-boundary:2"]);
});

test("keeps same-batch active tool refreshes from splitting local repeated message streams", () => {
  const first = {
    ...messageNode("message:assistant:msg-active-same-batch", "Before active same-batch tool. "),
    turnId: "turn-1",
    acpMessageId: "msg-active-same-batch"
  };
  const tool = {
    ...toolNode("tool:active-same-batch", "turn-1"),
    status: "in_progress" as const,
    toolStatus: "in_progress" as const,
    toolCallId: "active-same-batch",
    acpToolCallId: "active-same-batch"
  };
  const continuation = {
    ...messageNode(
      "message:assistant:msg-active-same-batch",
      "Before active same-batch tool. Still before active same-batch tool."
    ),
    turnId: "turn-1",
    acpMessageId: "msg-active-same-batch"
  };
  const refreshedTool = {
    ...tool,
    title: "Read refreshed same-batch tool"
  };

  const nodes = applyRenderPatchesLocally(
    [first, tool],
    [
      { type: "upsert", node: continuation },
      { type: "upsert", node: refreshedTool }
    ]
  );
  const changes = changedRenderNodesFromPatches(
    [first, tool],
    [
      { type: "upsert", node: continuation },
      { type: "upsert", node: refreshedTool }
    ]
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:msg-active-same-batch", "tool:active-same-batch"]
  );
  assert.equal(
    nodes[0]?.kind === "message" ? nodes[0].text : undefined,
    "Before active same-batch tool. Still before active same-batch tool."
  );
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.changedNodeIds].sort(), ["message:assistant:msg-active-same-batch", "tool:active-same-batch"].sort());
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

test("applies host-normalized late assistant continuation patches without losing final order", () => {
  const first = {
    ...messageNode("message:assistant:msg-host-late-completion", "Before completion. "),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    acpMessageId: "msg-host-late-completion",
    timelineOrder: 1
  };
  const tool = {
    ...toolNode("tool:tool-before-host-late-completion", "turn-1"),
    acpSessionId: "sess-1",
    toolCallId: "tool-before-host-late-completion",
    acpToolCallId: "tool-before-host-late-completion",
    timelineOrder: 2
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
  const late = {
    ...messageNode("message:assistant:msg-host-late-completion", "Before completion. Final answer."),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    acpMessageId: "msg-host-late-completion",
    timelineOrder: 4
  };
  const before = [first, tool, completion];

  const applied = applyRenderPatchesAndCollect(before, [{ type: "upsert", node: late }]);
  const webviewNodes = applyRenderPatchesLocally(before, applied.patches);

  assert.deepEqual(
    applied.patches.map((patch) => patch.node?.id || patch.id),
    ["message:assistant:msg-host-late-completion:2", "completion:turn-1"]
  );
  assert.deepEqual(webviewNodes, applied.nodes);
  assert.equal(webviewNodes[2]?.kind === "message" ? webviewNodes[2].text : undefined, "Final answer.");
  assert.equal((webviewNodes[3]?.timelineOrder ?? 0) > (webviewNodes[2]?.timelineOrder ?? 0), true);
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

test("keeps reused unknown provider event ids distinct after timeline boundaries", () => {
  const first = {
    ...unknownNode("unknown:provider-event", { sessionUpdate: "provider_extra", value: "first" }),
    timelineOrder: 1
  };
  const boundary = {
    ...messageNode("message:assistant:after-unknown-boundary", "After event."),
    turnId: "turn-1",
    timelineOrder: 2
  };
  const second = {
    ...unknownNode("unknown:provider-event", { sessionUpdate: "provider_extra", value: "second" }),
    timelineOrder: 3
  };

  const nodes = applyRenderPatchesLocally([first, boundary], [{ type: "upsert", node: second }]);
  const changes = changedRenderNodesFromPatches([first, boundary], [{ type: "upsert", node: second }]);

  const unknowns = nodes.filter((node): node is Extract<RenderNode, { kind: "unknown" }> => node.kind === "unknown");
  assert.equal(unknowns.length, 2);
  assert.deepEqual(unknowns.map((node) => node.payload), [
    { sessionUpdate: "provider_extra", value: "first" },
    { sessionUpdate: "provider_extra", value: "second" }
  ]);
  assert.notEqual(unknowns[0]?.id, unknowns[1]?.id);
  assert.deepEqual([...changes.addedNodeIds], [unknowns[1]!.id]);
  assert.deepEqual([...changes.changedNodeIds], [unknowns[1]!.id]);
});

test("reconciles refreshed unknown provider events by stable payload identity", () => {
  const existing = {
    ...unknownNode("unknown:provider-event-live", { detail: "same event", sessionUpdate: "provider_extra" }),
    timelineOrder: 1,
    createdAt: "2026-06-27T00:00:00.000Z"
  };
  const refreshed = {
    ...unknownNode("unknown:provider-event-hydrated", { sessionUpdate: "provider_extra", detail: "same event" }),
    timelineOrder: 5,
    createdAt: "2026-06-27T00:01:00.000Z"
  };

  const nodes = applyRenderPatchesLocally([existing], [{ type: "replace", node: refreshed }]);
  const changes = changedRenderNodesFromPatches([existing], [{ type: "replace", node: refreshed }]);

  assert.deepEqual(nodes.map((node) => node.id), ["unknown:provider-event-live"]);
  assert.equal(nodes[0]?.timelineOrder, 1);
  assert.equal(nodes[0]?.createdAt, "2026-06-27T00:00:00.000Z");
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.removedNodeIds], []);
  assert.deepEqual([...changes.changedNodeIds], ["unknown:provider-event-live"]);
});

test("does not move local turn completion behind late nodes from another ACP session", () => {
  const first = {
    ...messageNode("message:assistant:session-one", "First session done."),
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider",
    status: "completed" as const,
    streaming: false,
    timelineOrder: 1
  };
  const completion: RenderNode = {
    ...base,
    id: "completion:turn-1",
    kind: "completion",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider",
    status: "pending",
    stopReason: "end_turn",
    label: "Turn complete; checkpoint pending",
    checkpointPending: true,
    timelineOrder: 2
  };
  const lateOtherSession = {
    ...messageNode("message:assistant:session-two", "Second session late chunk."),
    turnId: "turn-1",
    acpSessionId: "sess-2",
    provider: "test-provider",
    timelineOrder: 3
  };

  const nodes = applyRenderPatchesLocally([first, completion], [{ type: "upsert", node: lateOtherSession }]);
  const changes = changedRenderNodesFromPatches([first, completion], [{ type: "upsert", node: lateOtherSession }]);

  assert.deepEqual(nodes.map((node) => node.id), [
    "message:assistant:session-one",
    "completion:turn-1",
    "message:assistant:session-two"
  ]);
  assert.equal((nodes[1]?.timelineOrder ?? 0) < (nodes[2]?.timelineOrder ?? 0), true);
  assert.deepEqual([...changes.changedNodeIds], ["message:assistant:session-two"]);
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
        id: "unknown:provider-event",
        kind: "unknown",
        status: "completed",
        label: "Provider progress",
        payload: { sessionUpdate: "custom_progress" }
      }
    }),
    true
  );
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

import assert from "node:assert/strict";
import test from "node:test";
import {
  applyRenderPatches,
  applyRenderPatchesAndCollect,
  reducePermissionRequest,
  reduceSessionUpdate,
  renderNodeSnapshotPatches,
  sessionControlsToPatches
} from "../shared/acpRenderReducers";
import type { ContentBlock } from "../shared/acpTypes";
import type { RenderNode, RenderReduceContext } from "../shared/renderModel";

const context: RenderReduceContext = {
  taskId: "task-1",
  lane: "lane-1",
  acpSessionId: "sess-1",
  currentTurnId: "turn-1",
  provider: "test-provider",
  now: () => "2026-06-27T00:00:00.000Z"
};

test("reduces streamed assistant text into a message node", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-1",
      content: {
        type: "text",
        text: "Done"
      }
    },
    context
  );

  const nodes = applyRenderPatches([], patches);
  assert.equal(nodes.length, 1);
  assert.equal(nodes[0]?.kind, "message");
  assert.equal(nodes[0]?.id, "message:assistant:msg-1");
});

test("aggregates streamed assistant chunks by message id", () => {
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-1",
      content: {
        type: "text",
        text: "Hello "
      }
    },
    context
  );
  const second = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-1",
      content: {
        type: "text",
        text: "world"
      }
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  assert.equal(nodes.length, 1);
  assert.equal(nodes[0]?.kind, "message");
  assert.equal(nodes[0]?.text, "Hello world");
  assert.equal(nodes[0]?.content.length, 1);
  assert.deepEqual(nodes[0]?.content[0], { type: "text", text: "Hello world" });
});

test("aggregates streamed assistant text chunks with aliased text fields", () => {
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-text-alias",
      content: {
        type: "text",
        content: "Hello "
      }
    },
    context
  );
  const second = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-text-alias",
      content: {
        type: "text",
        value: "world"
      }
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  assert.equal(nodes.length, 1);
  assert.equal(nodes[0]?.kind, "message");
  assert.equal(nodes[0]?.kind === "message" ? nodes[0].text : undefined, "Hello world");
  assert.equal(nodes[0]?.content.length, 1);
  assert.equal(nodes[0]?.content[0]?.type, "text");
  assert.equal((nodes[0]?.content[0] as { text?: string } | undefined)?.text, "Hello world");
});

test("accepts cumulative streamed assistant chunks without duplicating text", () => {
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-1",
      content: {
        type: "text",
        text: "Hello "
      }
    },
    context
  );
  const second = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-1",
      content: {
        type: "text",
        text: "Hello world"
      }
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  assert.equal(nodes.length, 1);
  assert.equal(nodes[0]?.kind, "message");
  assert.equal(nodes[0]?.kind === "message" ? nodes[0].text : undefined, "Hello world");
  assert.deepEqual(nodes[0]?.content[0], { type: "text", text: "Hello world" });
});

test("marks completed assistant message upserts as no longer streaming", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-complete-stream",
        content: {
          type: "text",
          text: "Final"
        }
      },
      context
    )
  );
  const previous = nodes[0];
  assert.equal(previous?.kind, "message");
  if (previous?.kind !== "message") {
    throw new Error("expected streaming assistant message");
  }
  const completed: typeof previous = {
    ...previous,
    status: "completed",
    streaming: false,
    content: [{ type: "text", text: "Final answer." }],
    text: "Final answer."
  };

  nodes = applyRenderPatches(nodes, [{ type: "upsert", node: completed }]);

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected completed assistant message");
  }
  assert.equal(node.status, "completed");
  assert.equal(node.streaming, false);
  assert.equal(node.text, "Final answer.");
});

test("keeps distinct non-text assistant chunks with the same display placeholder", () => {
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
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-media",
        content: firstImage
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-media",
        content: secondImage
      },
      context
    )
  );

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected media message node");
  }
  assert.equal(node.text, "[image][image]");
  assert.deepEqual(node.content, [firstImage, secondImage]);
});

test("keeps distinct streamed message ids separate", () => {
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-1",
      content: {
        type: "text",
        text: "One"
      }
    },
    context
  );
  const second = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-2",
      content: {
        type: "text",
        text: "Two"
      }
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  assert.equal(nodes.length, 2);
  assert.equal(nodes[0]?.id, "message:assistant:msg-1");
  assert.equal(nodes[1]?.id, "message:assistant:msg-2");
});

test("starts a continuation when a repeated message id resumes after a tool boundary", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-shared",
        content: {
          type: "text",
          text: "Before tool. "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-shared",
        content: {
          type: "text",
          text: "Still before tool."
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-between-shared-message",
        title: "Read README.md",
        kind: "read",
        status: "completed"
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-shared",
        content: {
          type: "text",
          text: "After tool. "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-shared",
        content: {
          type: "text",
          text: "Still after tool."
        }
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:msg-shared", "tool:tool-between-shared-message", "message:assistant:msg-shared:2"]
  );
  assert.deepEqual(
    nodes.map((node) => node.timelineOrder),
    [1, 2, 3]
  );
  const first = nodes[0];
  const second = nodes[2];
  assert.equal(first?.kind, "message");
  assert.equal(second?.kind, "message");
  if (first?.kind !== "message" || second?.kind !== "message") {
    throw new Error("expected split assistant message nodes");
  }
  assert.equal(first.acpMessageId, "msg-shared");
  assert.equal(second.acpMessageId, "msg-shared");
  assert.equal(first.text, "Before tool. Still before tool.");
  assert.equal(second.text, "After tool. Still after tool.");
});

test("trims cumulative repeated assistant snapshots after tool boundaries", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-cumulative",
        content: {
          type: "text",
          text: "Before tool. "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-between-cumulative-message",
        title: "Read README.md",
        kind: "read",
        status: "completed"
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-cumulative",
        content: {
          type: "text",
          text: "Before tool. After "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-cumulative",
        content: {
          type: "text",
          text: "Before tool. After tool."
        }
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:msg-cumulative", "tool:tool-between-cumulative-message", "message:assistant:msg-cumulative:2"]
  );
  const first = nodes[0];
  const second = nodes[2];
  assert.equal(first?.kind, "message");
  assert.equal(second?.kind, "message");
  if (first?.kind !== "message" || second?.kind !== "message") {
    throw new Error("expected cumulative assistant message split around a tool call");
  }
  assert.equal(first.text, "Before tool. ");
  assert.equal(second.text, "After tool.");
});

test("keeps turn completion after late repeated assistant chunks", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-late-completion",
        content: {
          type: "text",
          text: "Before completion. "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-before-late-completion",
        title: "Read README.md",
        kind: "read",
        status: "completed"
      },
      context
    )
  );
  const completion: RenderNode = {
    id: "completion:turn-1",
    kind: "completion",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "pending",
    updatedAt: "2026-06-27T00:00:01.000Z",
    stopReason: "end_turn",
    label: "Turn complete; checkpoint pending",
    checkpointPending: true
  };
  nodes = applyRenderPatches(nodes, [{ type: "upsert", node: completion }]);
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-late-completion",
        content: {
          type: "text",
          text: "Before completion. Final answer."
        }
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    [
      "message:assistant:msg-late-completion",
      "tool:tool-before-late-completion",
      "message:assistant:msg-late-completion:2",
      "completion:turn-1"
    ]
  );
  const lateMessage = nodes[2];
  const finalMarker = nodes[3];
  assert.equal(lateMessage?.kind, "message");
  assert.equal(finalMarker?.kind, "completion");
  if (lateMessage?.kind !== "message" || finalMarker?.kind !== "completion") {
    throw new Error("expected late message before completion marker");
  }
  assert.equal(lateMessage.text, "Final answer.");
  assert.equal((finalMarker.timelineOrder ?? 0) > (lateMessage.timelineOrder ?? 0), true);
});

test("keeps hydrated turn checkpoints after late live assistant chunks", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-late-hydrated",
        content: {
          type: "text",
          text: "Before checkpoint. "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-before-hydrated-checkpoint",
        title: "Read package.json",
        kind: "read",
        status: "completed"
      },
      context
    )
  );
  const checkpoint: RenderNode = {
    id: "crabdb-checkpoint:turn-1",
    kind: "checkpoint",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    provider: "test-provider",
    source: "crabdb",
    status: "completed",
    checkpointId: "ch_late",
    label: "Checkpoint ch_late",
    timelineOrder: 3
  };
  nodes = applyRenderPatches(nodes, [{ type: "upsert", node: checkpoint }]);
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-late-hydrated",
        content: {
          type: "text",
          text: "Before checkpoint. Final persisted answer."
        }
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    [
      "message:assistant:msg-late-hydrated",
      "tool:tool-before-hydrated-checkpoint",
      "message:assistant:msg-late-hydrated:2",
      "crabdb-checkpoint:turn-1"
    ]
  );
  const lateMessage = nodes[2];
  const finalMarker = nodes[3];
  assert.equal(lateMessage?.kind, "message");
  assert.equal(finalMarker?.kind, "checkpoint");
  if (lateMessage?.kind !== "message" || finalMarker?.kind !== "checkpoint") {
    throw new Error("expected late message before hydrated checkpoint");
  }
  assert.equal(lateMessage.text, "Final persisted answer.");
  assert.equal((finalMarker.timelineOrder ?? 0) > (lateMessage.timelineOrder ?? 0), true);
});

test("skips snapshot patches for semantically identical render nodes", () => {
  const node: RenderNode = {
    id: "message:assistant:stable",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    source: "crabdb",
    status: "completed",
    raw: { z: 2, a: 1 },
    role: "assistant",
    content: [{ type: "text", text: "Stable" }],
    text: "Stable",
    streaming: false,
    timelineOrder: 1
  };

  assert.deepEqual(renderNodeSnapshotPatches([node], [{ ...node, raw: { a: 1, z: 2 } }]), []);
});

test("normalizes appended duplicate render ids before applying patches", () => {
  const existing: RenderNode = {
    id: "message:assistant:dup",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    source: "acp-live",
    status: "in_progress",
    role: "assistant",
    acpMessageId: "dup",
    content: [{ type: "text", text: "First" }],
    text: "First",
    streaming: true,
    timelineOrder: 1
  };
  const incoming: RenderNode = {
    ...existing,
    content: [{ type: "text", text: "Second" }],
    text: "Second",
    timelineOrder: undefined
  };

  const applied = applyRenderPatchesAndCollect([existing], [{ type: "append", node: incoming }]);

  assert.deepEqual(
    applied.nodes.map((node) => node.id),
    ["message:assistant:dup", "message:assistant:dup:2"]
  );
  assert.deepEqual(
    applied.nodes.map((node) => node.timelineOrder),
    [1, 2]
  );
  assert.deepEqual(applied.patches.map((patch) => patch.node?.id || patch.id), ["message:assistant:dup:2"]);
});

test("removes normalized duplicate additions without deleting existing render nodes", () => {
  const existing: RenderNode = {
    id: "message:assistant:dup-remove",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    source: "acp-live",
    status: "completed",
    role: "assistant",
    acpMessageId: "dup-remove",
    content: [{ type: "text", text: "Original" }],
    text: "Original",
    streaming: false,
    timelineOrder: 1
  };
  const incoming: RenderNode = {
    ...existing,
    content: [{ type: "text", text: "Transient duplicate" }],
    text: "Transient duplicate",
    timelineOrder: undefined
  };

  const applied = applyRenderPatchesAndCollect(
    [existing],
    [
      { type: "append", node: incoming },
      { type: "remove", id: incoming.id }
    ]
  );

  assert.deepEqual(applied.nodes.map((node) => node.id), ["message:assistant:dup-remove"]);
  assert.equal(applied.nodes[0], existing);
  assert.deepEqual(
    applied.patches.map((patch) => patch.node?.id || patch.id),
    ["message:assistant:dup-remove:2", "message:assistant:dup-remove:2"]
  );
});

test("converts refreshed render snapshots into applicable patches", () => {
  const liveMessage: RenderNode = {
    id: "message:assistant:live",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    source: "acp-live",
    status: "completed",
    role: "assistant",
    content: [{ type: "text", text: "Done" }],
    text: "Done",
    streaming: false,
    timelineOrder: 1
  };
  const tool: RenderNode = {
    id: "tool:run-tests",
    kind: "tool",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    source: "acp-live",
    status: "in_progress",
    toolCallId: "run-tests",
    title: "Run tests",
    toolKind: "execute",
    toolStatus: "in_progress",
    locations: [],
    content: [],
    timelineOrder: 2
  };
  const completedTool: RenderNode = {
    ...tool,
    source: "crabdb",
    status: "completed",
    toolStatus: "completed"
  };
  const checkpoint: RenderNode = {
    id: "crabdb-checkpoint:turn-1",
    kind: "checkpoint",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    source: "crabdb",
    status: "completed",
    checkpointId: "ch_1",
    label: "Checkpoint ch_1",
    timelineOrder: 3
  };

  const before = [liveMessage, tool];
  const next = [completedTool, checkpoint];
  const patches = renderNodeSnapshotPatches(before, next);

  assert.deepEqual(
    patches.map((patch) => `${patch.type}:${patch.node?.id || patch.id}`),
    ["remove:message:assistant:live", "replace:tool:run-tests", "upsert:crabdb-checkpoint:turn-1"]
  );
  assert.deepEqual(applyRenderPatches(before, patches), next);
});

test("keeps anonymous streamed assistant messages in tool-interleaved timeline order", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        content: {
          type: "text",
          text: "Before tool. "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        content: {
          type: "text",
          text: "Still before tool."
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-interleave",
        title: "Read README.md",
        kind: "read",
        status: "completed"
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        content: {
          type: "text",
          text: "After tool. "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        content: {
          type: "text",
          text: "Still after tool."
        }
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.kind),
    ["message", "tool", "message"]
  );
  assert.deepEqual(
    nodes.map((node) => node.timelineOrder),
    [1, 2, 3]
  );
  const first = nodes[0];
  const tool = nodes[1];
  const second = nodes[2];
  assert.equal(first?.kind, "message");
  assert.equal(tool?.kind, "tool");
  assert.equal(second?.kind, "message");
  if (first?.kind !== "message" || tool?.kind !== "tool" || second?.kind !== "message") {
    throw new Error("expected anonymous assistant messages around a tool call");
  }
  assert.equal(tool.createdAt, "2026-06-27T00:00:00.000Z");
  assert.equal(first.text, "Before tool. Still before tool.");
  assert.equal(second.text, "After tool. Still after tool.");
  assert.equal(first.content.length, 1);
  assert.equal(second.content.length, 1);
  assert.deepEqual(first.content[0], { type: "text", text: "Before tool. Still before tool." });
  assert.deepEqual(second.content[0], { type: "text", text: "After tool. Still after tool." });
  assert.notEqual(nodes[0]?.id, nodes[2]?.id);
});

test("aggregates streamed thought chunks without showing them as transcript messages", () => {
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_thought_chunk",
      messageId: "thought-1",
      content: {
        type: "text",
        text: "Inspect"
      }
    },
    context
  );
  const second = reduceSessionUpdate(
    {
      sessionUpdate: "agent_thought_chunk",
      messageId: "thought-1",
      content: {
        type: "text",
        text: " files"
      }
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  assert.equal(nodes.length, 1);
  assert.equal(nodes[0]?.kind, "thought");
  assert.equal(nodes[0]?.content.length, 1);
  assert.deepEqual(nodes[0]?.content[0], { type: "text", text: "Inspect files" });
});

test("accepts cumulative streamed thought chunks without duplicating text", () => {
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_thought_chunk",
      messageId: "thought-1",
      content: {
        type: "text",
        text: "Inspect"
      }
    },
    context
  );
  const second = reduceSessionUpdate(
    {
      sessionUpdate: "agent_thought_chunk",
      messageId: "thought-1",
      content: {
        type: "text",
        text: "Inspect files"
      }
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  assert.equal(nodes.length, 1);
  assert.equal(nodes[0]?.kind, "thought");
  assert.deepEqual(nodes[0]?.content[0], { type: "text", text: "Inspect files" });
});

test("keeps anonymous streamed thoughts in tool-interleaved timeline order", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_thought_chunk",
        content: {
          type: "text",
          text: "Think first. "
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_thought_chunk",
        content: {
          type: "text",
          text: "Still thinking."
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-between-thoughts",
        title: "Read README.md",
        kind: "read",
        status: "completed"
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_thought_chunk",
        content: {
          type: "text",
          text: "Think after tool."
        }
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.kind),
    ["thought", "tool", "thought"]
  );
  const first = nodes[0];
  const second = nodes[2];
  assert.equal(first?.kind, "thought");
  assert.equal(second?.kind, "thought");
  if (first?.kind !== "thought" || second?.kind !== "thought") {
    throw new Error("expected anonymous thought nodes");
  }
  assert.deepEqual(first.content, [{ type: "text", text: "Think first. Still thinking." }]);
  assert.deepEqual(second.content, [{ type: "text", text: "Think after tool." }]);
  assert.equal(first.id, "thought:anonymous");
  assert.equal(second.id, "thought:anonymous:2");
});

test("expands tool diff content into tool and diff nodes", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-1",
      title: "Edit README",
      kind: "edit",
      status: "completed",
      content: [
        {
          type: "diff",
          path: "README.md",
          oldText: "old",
          newText: "new"
        }
      ]
    },
    context
  );

  const nodes = applyRenderPatches([], patches);
  assert.equal(nodes.some((node) => node.kind === "tool"), true);
  assert.equal(nodes.some((node) => node.kind === "diff" && node.id === "diff:tool-1:README.md"), true);
});

test("expands terminal tool content with command metadata", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-terminal",
      title: "Run tests",
      kind: "execute",
      status: "failed",
      content: [
        {
          type: "terminal",
          terminalId: "term-1",
          command: ["npm", "test"],
          cwd: "/workspace/project",
          status: "exited",
          exitCode: 1,
          elapsedMs: 1200,
          stdout: "ok",
          stderr: "failed"
        }
      ]
    },
    context
  );

  const nodes = applyRenderPatches([], patches);
  const terminal = nodes.find((node) => node.kind === "terminal");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.terminalId, "term-1");
  assert.equal(terminal?.command, "npm test");
  assert.equal(terminal?.cwd, "/workspace/project");
  assert.equal(terminal?.terminalStatus, "exited");
  assert.equal(terminal?.exitCode, 1);
  assert.equal(terminal?.elapsedMs, 1200);
  assert.equal(terminal?.stdout, "ok");
  assert.equal(terminal?.stderr, "failed");
});

test("recovers persisted formatted output into readable tool content", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-list-files",
        title: "List files",
        kind: "read",
        status: "completed",
        rawOutput: {
          output: {
            exit_code: 0,
            formatted_output: "src/handler.ts\nsrc/view.ts\nREADME.md\n"
          }
        }
      },
      context
    )
  );
  const tool = nodes.find((node) => node.kind === "tool");
  assert.equal(tool?.kind, "tool");
  assert.deepEqual(tool?.content, [
    {
      type: "content",
      content: {
        type: "text",
        text: "src/handler.ts\nsrc/view.ts\nREADME.md"
      }
    }
  ]);
});

test("recovers persisted command output into terminal content", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-recovered-terminal",
        title: "Bash",
        kind: "other",
        status: "completed",
        rawInput: { command: "ls" },
        rawOutput: {
          output: {
            exit_code: 0,
            formatted_output: "README.md\nsrc\n"
          }
        }
      },
      context
    )
  );
  const tool = nodes.find((node) => node.kind === "tool");
  const terminal = nodes.find((node) => node.kind === "terminal");
  assert.equal(tool?.kind, "tool");
  assert.equal((tool?.content[0] as Record<string, unknown> | undefined)?.type, "terminal");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.terminalId, "tool-recovered-terminal");
  assert.equal(terminal?.command, "ls");
  assert.equal(terminal?.exitCode, 0);
  assert.equal(terminal?.stdout, "README.md\nsrc");
});

test("syncs expanded terminal status from status-only tool updates", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-terminal-sync",
        title: "Run git remote",
        kind: "execute",
        status: "in_progress",
        content: [
          {
            type: "terminal",
            terminalId: "term-sync",
            command: ["git", "remote", "-v"]
          }
        ]
      },
      context
    )
  );

  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call_update",
        toolCallId: "tool-terminal-sync",
        status: "completed"
      },
      context
    )
  );

  const terminal = nodes.find((node) => node.kind === "terminal");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.status, "completed");
  assert.equal(terminal?.terminalStatus, "completed");
  const tool = nodes.find((node) => node.kind === "tool");
  assert.equal(tool?.kind, "tool");
  assert.equal((tool?.content[0] as Record<string, unknown> | undefined)?.status, "completed");
});

test("collects actual applied patches while reducing tool-linked terminal changes", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-terminal-collect",
        title: "Run git status",
        kind: "execute",
        status: "in_progress",
        content: [
          {
            type: "terminal",
            terminalId: "term-collect",
            command: ["git", "status", "--short"]
          }
        ]
      },
      context
    )
  );

  const update = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call_update",
      toolCallId: "tool-terminal-collect",
      status: "completed"
    },
    context
  );
  const applied = applyRenderPatchesAndCollect(nodes, update);
  nodes = applied.nodes;

  assert.deepEqual(
    applied.patches.map((patch) => patch.node?.id || patch.id).sort(),
    ["terminal:tool-terminal-collect:term-collect", "tool:tool-terminal-collect"].sort()
  );
  const terminal = nodes.find((node) => node.kind === "terminal");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.status, "completed");
});

test("keeps expanded terminal nodes distinct when providers reuse terminal ids across tools", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-first-terminal",
        title: "Run first command",
        kind: "execute",
        status: "completed",
        content: [
          {
            type: "terminal",
            terminalId: "shared-terminal",
            command: "npm test",
            stdout: "first output"
          }
        ]
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-second-terminal",
        title: "Run second command",
        kind: "execute",
        status: "completed",
        content: [
          {
            type: "terminal",
            terminalId: "shared-terminal",
            command: "npm run lint",
            stdout: "second output"
          }
        ]
      },
      context
    )
  );

  const terminals = nodes.filter((node): node is Extract<RenderNode, { kind: "terminal" }> => node.kind === "terminal");
  assert.deepEqual(
    terminals.map((node) => node.id),
    [
      "terminal:tool-first-terminal:shared-terminal",
      "terminal:tool-second-terminal:shared-terminal"
    ]
  );
  assert.deepEqual(
    terminals.map((node) => node.acpToolCallId),
    ["tool-first-terminal", "tool-second-terminal"]
  );
  assert.deepEqual(
    terminals.map((node) => node.stdout),
    ["first output", "second output"]
  );
  assert.deepEqual(
    nodes.map((node) => node.kind),
    ["tool", "terminal", "tool", "terminal"]
  );
  assert.deepEqual(
    nodes.map((node) => node.timelineOrder),
    [1, 2, 3, 4]
  );
});

test("keeps tool nodes distinct when providers reuse tool ids across turns", () => {
  const turnTwoContext: RenderReduceContext = {
    ...context,
    currentTurnId: "turn-2",
    now: () => "2026-06-27T00:01:00.000Z"
  };
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "shared-tool",
        title: "Run first shared tool",
        kind: "execute",
        status: "completed",
        content: [
          {
            type: "terminal",
            terminalId: "shared-terminal",
            stdout: "first output"
          }
        ]
      },
      context
    )
  );

  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "shared-tool",
        title: "Run second shared tool",
        kind: "execute",
        status: "in_progress",
        content: [
          {
            type: "terminal",
            terminalId: "shared-terminal",
            stdout: "second output"
          }
        ]
      },
      turnTwoContext
    )
  );

  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call_update",
        toolCallId: "shared-tool",
        status: "failed"
      },
      turnTwoContext
    )
  );

  assert.equal(new Set(nodes.map((node) => node.id)).size, nodes.length);
  assert.deepEqual(
    nodes.map((node) => node.id),
    [
      "tool:shared-tool",
      "terminal:shared-tool:shared-terminal",
      "tool:shared-tool:turn-2:sess-1:acp-live",
      "terminal:shared-tool:shared-terminal:turn-2:sess-1:acp-live"
    ]
  );
  assert.deepEqual(
    nodes.map((node) => node.timelineOrder),
    [1, 2, 3, 4]
  );
  const tools = nodes.filter((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool");
  assert.deepEqual(
    tools.map((node) => [node.turnId, node.title, node.toolStatus]),
    [
      ["turn-1", "Run first shared tool", "completed"],
      ["turn-2", "Run second shared tool", "failed"]
    ]
  );
  const terminals = nodes.filter((node): node is Extract<RenderNode, { kind: "terminal" }> => node.kind === "terminal");
  assert.deepEqual(
    terminals.map((node) => [node.turnId, node.stdout, node.status, node.terminalStatus]),
    [
      ["turn-1", "first output", "completed", "completed"],
      ["turn-2", "second output", "failed", "failed"]
    ]
  );
});

test("keeps reused completed tool ids distinct after later same-turn messages", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "same-turn-tool",
        title: "Run first command",
        kind: "execute",
        status: "completed",
        content: [
          {
            type: "terminal",
            terminalId: "shared-terminal",
            command: "npm test",
            stdout: "first output"
          }
        ]
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "after-first-tool",
        content: {
          type: "text",
          text: "Continuing after first tool."
        }
      },
      context
    )
  );
  const applied = applyRenderPatchesAndCollect(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "same-turn-tool",
        title: "Run second command",
        kind: "execute",
        status: "completed",
        content: [
          {
            type: "terminal",
            terminalId: "shared-terminal",
            command: "npm run lint",
            stdout: "second output"
          }
        ]
      },
      context
    )
  );
  nodes = applied.nodes;

  assert.equal(new Set(nodes.map((node) => node.id)).size, nodes.length);
  assert.deepEqual(
    nodes.map((node) => node.id),
    [
      "tool:same-turn-tool",
      "terminal:same-turn-tool:shared-terminal",
      "message:assistant:after-first-tool",
      "tool:same-turn-tool:turn-1:sess-1:acp-live",
      "terminal:same-turn-tool:shared-terminal:turn-1:sess-1:acp-live"
    ]
  );
  assert.deepEqual(
    nodes.map((node) => node.timelineOrder),
    [1, 2, 3, 4, 5]
  );
  assert.deepEqual(
    nodes.filter((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool").map((node) => node.title),
    ["Run first command", "Run second command"]
  );
  assert.deepEqual(
    nodes.filter((node): node is Extract<RenderNode, { kind: "terminal" }> => node.kind === "terminal").map((node) => node.stdout),
    ["first output", "second output"]
  );
  assert.deepEqual(
    applied.patches.map((patch) => patch.node?.id || patch.id),
    [
      "tool:same-turn-tool:turn-1:sess-1:acp-live",
      "terminal:same-turn-tool:shared-terminal:turn-1:sess-1:acp-live"
    ]
  );
});

test("keeps late tool call updates attached after later same-turn messages", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "late-update-tool",
        title: "Run command",
        kind: "execute",
        status: "completed"
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "after-late-tool",
        content: {
          type: "text",
          text: "Text after tool."
        }
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call_update",
        toolCallId: "late-update-tool",
        status: "failed"
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["tool:late-update-tool", "message:assistant:after-late-tool"]
  );
  const tool = nodes.find((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool");
  assert.equal(tool?.toolStatus, "failed");
  assert.equal((tool?.raw as { sessionUpdate?: unknown } | undefined)?.sessionUpdate, "tool_call_update");
});

test("merges repeated terminal updates without dropping prior command metadata", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-terminal-output",
        title: "Run git rev-parse",
        kind: "execute",
        status: "completed",
        content: [
          {
            type: "terminal",
            terminalId: "term-output",
            command: ["git", "rev-parse", "--show-toplevel"],
            cwd: "/workspace/project"
          }
        ]
      },
      context
    )
  );

  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call_update",
        toolCallId: "tool-terminal-output",
        content: [
          {
            type: "terminal",
            terminalId: "term-output",
            stdout: "/workspace/project\n"
          }
        ]
      },
      context
    )
  );

  const terminal = nodes.find((node) => node.kind === "terminal");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.status, "completed");
  assert.equal(terminal?.terminalStatus, "completed");
  assert.equal(terminal?.command, "git rev-parse --show-toplevel");
  assert.equal(terminal?.cwd, "/workspace/project");
  assert.equal(terminal?.stdout, "/workspace/project\n");
});

test("merges status-only tool updates without losing the original call details", () => {
  const start = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-merge",
      title: "Read README.md",
      kind: "read",
      status: "pending",
      locations: [{ path: "README.md", line: 1 }],
      rawInput: {
        path: "README.md"
      }
    },
    context
  );
  const update = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call_update",
      toolCallId: "tool-merge",
      status: "completed"
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], start), update);
  const tool = nodes.find((node) => node.kind === "tool");
  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.title, "Read README.md");
  assert.equal(tool?.toolKind, "read");
  assert.equal(tool?.toolStatus, "completed");
  assert.equal(tool?.locations[0]?.path, "README.md");
  assert.deepEqual(tool?.rawInput, { path: "README.md" });
});

test("keeps completed tool status when later updates omit status", () => {
  const start = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-status",
      title: "Read README.md",
      kind: "read",
      status: "completed"
    },
    context
  );
  const update = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call_update",
      toolCallId: "tool-status",
      content: [
        {
          type: "content",
          content: {
            type: "text",
            text: "README"
          }
        }
      ]
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], start), update);
  const tool = nodes.find((node) => node.kind === "tool");
  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.status, "completed");
  assert.equal(tool?.toolStatus, "completed");
  assert.equal(tool?.content.length, 1);
});

test("creates an approval node from ACP permission requests", () => {
  const patches = reducePermissionRequest(
    "100",
    {
      sessionId: "sess-1",
      toolCall: {
        sessionUpdate: "tool_call",
        toolCallId: "tool-2",
        title: "Run tests",
        kind: "execute",
        status: "pending"
      },
      options: [
        {
          optionId: "allow",
          name: "Allow once"
        }
      ]
    },
    context
  );

  const nodes = applyRenderPatches([], patches);
  assert.equal(nodes[0]?.kind, "approval");
  assert.equal(nodes[0]?.id, "approval:100");
});

test("unknown session updates degrade to unknown nodes", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "provider_extra",
      payload: true
    },
    context
  );

  const nodes = applyRenderPatches([], patches);
  assert.equal(nodes[0]?.kind, "unknown");
});

test("hydrates initial session mode and config controls", () => {
  const patches = sessionControlsToPatches(
    {
      sessionId: "sess-1",
      modes: {
        currentModeId: "ask",
        availableModes: [
          {
            id: "ask",
            name: "Ask"
          },
          {
            id: "code",
            name: "Code"
          }
        ]
      },
      configOptions: [
        {
          id: "model",
          name: "Model",
          type: "select",
          currentValue: "fast",
          options: [
            {
              value: "fast",
              name: "Fast"
            }
          ]
        }
      ]
    },
    context
  );

  const nodes = applyRenderPatches([], patches);
  const mode = nodes.find((node) => node.kind === "mode");
  const config = nodes.find((node) => node.kind === "config");
  assert.equal(mode?.kind, "mode");
  assert.equal(mode?.modeId, "ask");
  assert.equal(mode?.availableModes.length, 2);
  assert.equal(config?.kind, "config");
  assert.equal(config?.configOptions[0]?.id, "model");
});

test("accepts legacy modeId field in current mode updates", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "current_mode_update",
      modeId: "code"
    },
    context
  );

  const nodes = applyRenderPatches([], patches);
  assert.equal(nodes[0]?.kind, "mode");
  assert.equal(nodes[0]?.modeId, "code");
});

test("renders session info updates as session metadata", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "session_info_update",
      title: "Implement review drawer",
      updatedAt: "2026-06-27T01:00:00.000Z"
    },
    context
  );

  const nodes = applyRenderPatches([], patches);
  const session = nodes[0];
  assert.equal(session?.kind, "session");
  if (session?.kind !== "session") {
    throw new Error("Expected session node.");
  }
  assert.equal(session.title, "Implement review drawer");
  assert.equal(session.sessionUpdatedAt, "2026-06-27T01:00:00.000Z");
});

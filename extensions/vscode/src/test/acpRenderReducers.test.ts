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
import type { ContentBlock, SessionUpdate } from "../shared/acpTypes";
import type { RenderNode, RenderReduceContext } from "../shared/renderModel";

const context: RenderReduceContext = {
  taskId: "task-1",
  lane: "lane-1",
  acpSessionId: "sess-1",
  currentTurnId: "turn-1",
  provider: "test-provider",
  now: () => "2026-06-27T00:00:00.000Z"
};

function unknownNode(id: string, payload: unknown): Extract<RenderNode, { kind: "unknown" }> {
  return {
    id,
    kind: "unknown",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "completed",
    label: "Unsupported ACP update: provider_extra",
    payload
  };
}

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

test("reduces snake-case live session updates into message nodes", () => {
  const patches = reduceSessionUpdate(
    {
      session_update: "agent_message_chunk",
      message_id: "msg-snake-update",
      content: {
        type: "text",
        text: "Rendered from snake case."
      }
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  assert.equal(nodes.length, 1);
  assert.equal(nodes[0]?.kind, "message");
  assert.equal(nodes[0]?.id, "message:assistant:msg-snake-update");
  assert.equal(nodes[0]?.kind === "message" ? nodes[0].text : undefined, "Rendered from snake case.");
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

test("promotes anonymous assistant streams when a later chunk carries the message id", () => {
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
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
      messageId: "msg-promoted",
      content: {
        type: "text",
        text: "world"
      }
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  const message = nodes.find((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message");

  assert.deepEqual(nodes.map((node) => node.id), ["message:assistant:anonymous"]);
  assert.equal(message?.acpMessageId, "msg-promoted");
  assert.equal(message?.text, "Hello world");
  assert.deepEqual(message?.content, [{ type: "text", text: "Hello world" }]);
});

test("promotes assistant stream session scope when session ids arrive late", () => {
  const { acpSessionId: _acpSessionId, ...contextWithoutSession } = context;
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-session-promoted",
      content: {
        type: "text",
        text: "Hello "
      }
    },
    contextWithoutSession
  );
  const second = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-session-promoted",
      content: {
        type: "text",
        text: "world"
      }
    },
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  const message = nodes.find((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message");

  assert.deepEqual(nodes.map((node) => node.id), ["message:assistant:msg-session-promoted"]);
  assert.equal(message?.acpSessionId, "sess-1");
  assert.equal(message?.text, "Hello world");
  assert.deepEqual(message?.content, [{ type: "text", text: "Hello world" }]);
});

test("keeps assistant stream scope when later chunks omit session and provider", () => {
  const { acpSessionId: _acpSessionId, provider: _provider, ...contextWithoutScope } = context;
  const first = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-scope-preserved",
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
      messageId: "msg-scope-preserved",
      content: {
        type: "text",
        text: "world"
      }
    },
    contextWithoutScope
  );

  const nodes = applyRenderPatches(applyRenderPatches([], first), second);
  const message = nodes.find((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message");

  assert.deepEqual(nodes.map((node) => node.id), ["message:assistant:msg-scope-preserved"]);
  assert.equal(message?.acpSessionId, "sess-1");
  assert.equal(message?.provider, "test-provider");
  assert.equal(message?.text, "Hello world");
});

test("keeps assistant stream message id when later chunks omit it", () => {
  const [existing] = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-id-preserved",
        content: {
          type: "text",
          text: "Hello "
        }
      },
      context
    )
  );
  assert.equal(existing?.kind, "message");
  if (existing?.kind !== "message") {
    throw new Error("expected existing message node");
  }
  const incoming: Extract<RenderNode, { kind: "message" }> = {
    ...existing,
    acpMessageId: undefined,
    content: [{ type: "text", text: "world" }],
    text: "world"
  };

  const nodes = applyRenderPatches([existing], [{ type: "upsert", node: incoming }]);
  const message = nodes[0];

  assert.equal(message?.kind, "message");
  if (message?.kind !== "message") {
    throw new Error("expected merged message node");
  }
  assert.equal(message.acpMessageId, "msg-id-preserved");
  assert.equal(message.text, "Hello world");
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

test("renders live assistant chunks carried by delta aliases", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        message_id: "msg-delta-alias",
        delta: "Rendered from delta."
      } as unknown as SessionUpdate,
      context
    )
  );

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected aliased assistant message");
  }
  assert.equal(node.id, "message:assistant:msg-delta-alias");
  assert.equal(node.text, "Rendered from delta.");
  assert.deepEqual(node.content, [{ type: "text", text: "Rendered from delta." }]);
});

test("uses wrapped live message ids instead of anonymous stream collisions", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        message_id: { id: 301 },
        content: {
          type: "text",
          text: "First wrapped message."
        }
      } as unknown as SessionUpdate,
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        message_id: { 0: 302 },
        content: {
          type: "text",
          text: "Second wrapped message."
        }
      } as unknown as SessionUpdate,
      context
    )
  );

  assert.deepEqual(
    nodes
      .filter((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message")
      .map((node) => `${node.id}:${node.text}`),
    [
      "message:assistant:301:First wrapped message.",
      "message:assistant:302:Second wrapped message."
    ]
  );
});

test("falls back to live message aliases when canonical content is empty", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-empty-content-alias",
        content: [],
        contentDelta: "Rendered after empty content."
      } as unknown as SessionUpdate,
      context
    )
  );

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected fallback assistant message");
  }
  assert.equal(node.text, "Rendered after empty content.");
});

test("renders live user chunks carried by text aliases", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "user_message_chunk",
        messageId: "msg-user-text-alias",
        text: "Rendered from text."
      } as unknown as SessionUpdate,
      context
    )
  );

  const node = nodes[0];
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected aliased user message");
  }
  assert.equal(node.role, "user");
  assert.equal(node.text, "Rendered from text.");
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

test("keeps completed assistant message lifecycle after late active chunks", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-late-active",
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
    streaming: false
  };
  nodes = applyRenderPatches(nodes, [{ type: "replace", node: completed }]);
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-late-active",
        content: {
          type: "text",
          text: "Final answer."
        }
      },
      context
    )
  );

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

test("preserves streamed assistant content arrays as structured blocks", () => {
  const content: ContentBlock[] = [
    {
      type: "text",
      text: "Rendered with context."
    },
    {
      type: "resource_link",
      uri: "file:///workspace/README.md",
      name: "README.md",
      title: "Context file"
    }
  ];
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-content-array",
      content
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  const message = nodes.find((node) => node.kind === "message");

  assert.equal(message?.kind, "message");
  assert.equal(message?.text, "Rendered with context.Context file (README.md)");
  assert.deepEqual(message?.content, content);
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

test("starts a continuation when a repeated message id resumes after an active tool boundary", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-active-boundary",
        content: {
          type: "text",
          text: "Before active tool. "
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
        toolCallId: "tool-active-boundary",
        title: "Read while active",
        kind: "read",
        status: "in_progress"
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-active-boundary",
        content: {
          type: "text",
          text: "Before active tool. After active tool."
        }
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:msg-active-boundary", "tool:tool-active-boundary", "message:assistant:msg-active-boundary:2"]
  );
  assert.deepEqual(
    nodes.map((node) => node.timelineOrder),
    [1, 2, 3]
  );
  assert.equal(nodes[0]?.kind === "message" ? nodes[0].text : undefined, "Before active tool. ");
  assert.equal(nodes[2]?.kind === "message" ? nodes[2].text : undefined, "After active tool.");
});

test("keeps same-batch active tool refreshes from splitting repeated message streams", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_message_chunk",
        messageId: "msg-active-same-batch",
        content: {
          type: "text",
          text: "Before active same-batch tool. "
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
        toolCallId: "tool-active-same-batch",
        title: "Read while active",
        kind: "read",
        status: "in_progress"
      },
      context
    )
  );

  const messageUpdate = reduceSessionUpdate(
    {
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-active-same-batch",
      content: {
        type: "text",
        text: "Before active same-batch tool. Still before active same-batch tool."
      }
    },
    context
  );
  const toolRefresh = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call_update",
      toolCallId: "tool-active-same-batch",
      title: "Read while active",
      status: "in_progress"
    },
    context
  );

  nodes = applyRenderPatches(nodes, [...messageUpdate, ...toolRefresh]);

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["message:assistant:msg-active-same-batch", "tool:tool-active-same-batch"]
  );
  assert.deepEqual(
    nodes.map((node) => node.timelineOrder),
    [1, 2]
  );
  assert.equal(
    nodes[0]?.kind === "message" ? nodes[0].text : undefined,
    "Before active same-batch tool. Still before active same-batch tool."
  );
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
    id: "trail-checkpoint:turn-1",
    kind: "checkpoint",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    provider: "test-provider",
    source: "trail",
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
      "trail-checkpoint:turn-1"
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

test("does not move turn completion behind late nodes from another ACP session", () => {
  const first: RenderNode = {
    id: "message:assistant:session-one",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "completed",
    role: "assistant",
    content: [{ type: "text", text: "First session done." }],
    text: "First session done.",
    streaming: false,
    timelineOrder: 1
  };
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
    checkpointPending: true,
    timelineOrder: 2
  };
  const lateOtherSession: RenderNode = {
    id: "message:assistant:session-two",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-2",
    provider: "test-provider",
    source: "acp-live",
    status: "in_progress",
    role: "assistant",
    content: [{ type: "text", text: "Second session late chunk." }],
    text: "Second session late chunk.",
    streaming: true,
    timelineOrder: 3
  };

  const nodes = applyRenderPatches([first, completion], [{ type: "upsert", node: lateOtherSession }]);

  assert.deepEqual(nodes.map((node) => node.id), [
    "message:assistant:session-one",
    "completion:turn-1",
    "message:assistant:session-two"
  ]);
  assert.equal((nodes[1]?.timelineOrder ?? 0) < (nodes[2]?.timelineOrder ?? 0), true);
});

test("skips snapshot patches for semantically identical render nodes", () => {
  const node: RenderNode = {
    id: "message:assistant:stable",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    source: "trail",
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

test("normalizes replacement patches that reuse message ids across turns", () => {
  const existing: RenderNode = {
    id: "message:assistant:replace-dup",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    source: "acp-live",
    status: "completed",
    role: "assistant",
    acpMessageId: "replace-dup",
    content: [{ type: "text", text: "First turn" }],
    text: "First turn",
    streaming: false,
    timelineOrder: 1
  };
  const incoming: RenderNode = {
    ...existing,
    turnId: "turn-2",
    content: [{ type: "text", text: "Second turn" }],
    text: "Second turn",
    timelineOrder: undefined
  };

  const applied = applyRenderPatchesAndCollect([existing], [{ type: "replace", node: incoming }]);

  assert.deepEqual(
    applied.nodes.map((node) => node.id),
    ["message:assistant:replace-dup", "message:assistant:replace-dup:2"]
  );
  assert.equal(applied.nodes[0], existing);
  assert.equal(applied.nodes[1]?.kind === "message" ? applied.nodes[1].turnId : undefined, "turn-2");
  assert.equal(applied.nodes[1]?.kind === "message" ? applied.nodes[1].text : undefined, "Second turn");
  assert.deepEqual(applied.patches.map((patch) => patch.node?.id || patch.id), ["message:assistant:replace-dup:2"]);
});

test("keeps host-side reused unknown provider event ids distinct after later timeline nodes", () => {
  const existing: RenderNode = {
    ...unknownNode("unknown:provider-event", { sessionUpdate: "provider_extra", value: "first" }),
    timelineOrder: 1
  };
  const boundary: RenderNode = {
    id: "message:assistant:after-unknown-boundary",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    source: "acp-live",
    status: "completed",
    role: "assistant",
    content: [{ type: "text", text: "After event." }],
    text: "After event.",
    streaming: false,
    timelineOrder: 2
  };
  const incoming: RenderNode = {
    ...unknownNode("unknown:provider-event", { sessionUpdate: "provider_extra", value: "second" }),
    timelineOrder: 3
  };

  const applied = applyRenderPatchesAndCollect([existing, boundary], [{ type: "upsert", node: incoming }]);
  const unknowns = applied.nodes.filter((node): node is Extract<RenderNode, { kind: "unknown" }> => node.kind === "unknown");

  assert.equal(unknowns.length, 2);
  assert.deepEqual(unknowns.map((node) => node.payload), [
    { sessionUpdate: "provider_extra", value: "first" },
    { sessionUpdate: "provider_extra", value: "second" }
  ]);
  assert.notEqual(unknowns[0]?.id, unknowns[1]?.id);
  assert.deepEqual(applied.patches.map((patch) => patch.node?.id || patch.id), [unknowns[1]!.id]);
});

test("reconciles host-side refreshed unknown provider events by stable payload identity", () => {
  const existing: RenderNode = {
    ...unknownNode("unknown:provider-event-live", { detail: "same event", sessionUpdate: "provider_extra" }),
    timelineOrder: 1,
    createdAt: "2026-06-27T00:00:00.000Z"
  };
  const refreshed: RenderNode = {
    ...unknownNode("unknown:provider-event-hydrated", { sessionUpdate: "provider_extra", detail: "same event" }),
    timelineOrder: 5,
    createdAt: "2026-06-27T00:01:00.000Z"
  };

  const applied = applyRenderPatchesAndCollect([existing], [{ type: "replace", node: refreshed }]);

  assert.deepEqual(applied.nodes.map((node) => node.id), ["unknown:provider-event-live"]);
  assert.equal(applied.nodes[0]?.timelineOrder, 1);
  assert.equal(applied.nodes[0]?.createdAt, "2026-06-27T00:00:00.000Z");
  assert.deepEqual(applied.patches.map((patch) => patch.node?.id || patch.id), ["unknown:provider-event-live"]);
});

test("converts refreshed semantic snapshot ids into replacements instead of remove and add", () => {
  const existing: RenderNode = {
    ...unknownNode("unknown:provider-event-live", { detail: "same event", sessionUpdate: "provider_extra" }),
    timelineOrder: 1,
    createdAt: "2026-06-27T00:00:00.000Z",
    updatedAt: "2026-06-27T00:00:00.000Z"
  };
  const refreshed: RenderNode = {
    ...unknownNode("unknown:provider-event-hydrated", { sessionUpdate: "provider_extra", detail: "same event" }),
    timelineOrder: 7,
    createdAt: "2026-06-27T00:01:00.000Z",
    updatedAt: "2026-06-27T00:01:00.000Z"
  };

  const patches = renderNodeSnapshotPatches([existing], [refreshed]);
  const applied = applyRenderPatchesAndCollect([existing], patches);

  assert.deepEqual(patches.map((patch) => `${patch.type}:${patch.node?.id || patch.id}`), [
    "replace:unknown:provider-event-hydrated"
  ]);
  assert.deepEqual(applied.nodes.map((node) => node.id), ["unknown:provider-event-live"]);
  assert.equal(applied.nodes[0]?.timelineOrder, 1);
  assert.equal(applied.nodes[0]?.createdAt, "2026-06-27T00:00:00.000Z");
  assert.equal(applied.nodes[0]?.updatedAt, "2026-06-27T00:01:00.000Z");
  assert.deepEqual(applied.patches.map((patch) => `${patch.type}:${patch.node?.id || patch.id}`), [
    "upsert:unknown:provider-event-live"
  ]);
});

test("converts live-to-hydrated message snapshot ids into replacements without remove and add", () => {
  const live: RenderNode = {
    id: "message:assistant:msg-final",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-final-message",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "completed",
    role: "assistant",
    acpMessageId: "msg-final",
    content: [{ type: "text", text: "Final answer rendered." }],
    text: "Final answer rendered.",
    streaming: false,
    timelineOrder: 1
  };
  const hydrated: RenderNode = {
    ...live,
    id: "trail-message:turn-final-message:msg-final",
    source: "trail",
    updatedAt: "2026-06-27T00:01:00.000Z"
  };

  const patches = renderNodeSnapshotPatches([live], [hydrated]);
  const applied = applyRenderPatchesAndCollect([live], patches);

  assert.deepEqual(patches.map((patch) => `${patch.type}:${patch.node?.id || patch.id}`), [
    "replace:trail-message:turn-final-message:msg-final"
  ]);
  assert.deepEqual(applied.nodes.map((node) => `${node.id}:${node.source}`), [
    "message:assistant:msg-final:trail"
  ]);
  assert.equal(applied.nodes[0]?.timelineOrder, 1);
  assert.deepEqual(applied.patches.map((patch) => `${patch.type}:${patch.node?.id || patch.id}`), [
    "upsert:message:assistant:msg-final"
  ]);
});

test("converts live-to-hydrated prompt turn snapshots into replacements across Trail turn ids", () => {
  const liveUser: RenderNode = {
    id: "message:user:turn-live",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-live",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "completed",
    role: "user",
    content: [{ type: "text", text: "what I have in current repo and how many lines of code ?" }],
    text: "what I have in current repo and how many lines of code ?",
    streaming: false,
    timelineOrder: 1
  };
  const liveTool: RenderNode = {
    id: "tool:list-files:live",
    kind: "tool",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-live",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "completed",
    acpToolCallId: "list-files",
    toolCallId: "list-files",
    title: "Listed files",
    toolKind: "execute",
    toolStatus: "completed",
    locations: [],
    content: [],
    timelineOrder: 2
  };
  const liveAssistant: RenderNode = {
    id: "message:assistant:msg-summary",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-live",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "completed",
    role: "assistant",
    acpMessageId: "msg-summary",
    content: [{ type: "text", text: "Here's a summary of your repo." }],
    text: "Here's a summary of your repo.",
    streaming: false,
    timelineOrder: 3
  };
  const hydratedUser: RenderNode = {
    ...liveUser,
    id: "trail-message:turn-trail:msg-user",
    turnId: "turn-trail",
    source: "trail",
    updatedAt: "2026-06-27T00:01:00.000Z"
  };
  const hydratedTool: RenderNode = {
    ...liveTool,
    id: "tool:list-files",
    turnId: "turn-trail",
    source: "trail",
    updatedAt: "2026-06-27T00:01:01.000Z"
  };
  const hydratedAssistant: RenderNode = {
    ...liveAssistant,
    id: "trail-message:turn-trail:msg-summary",
    turnId: "turn-trail",
    source: "trail",
    updatedAt: "2026-06-27T00:01:02.000Z"
  };

  const before = [liveUser, liveTool, liveAssistant];
  const next = [hydratedUser, hydratedTool, hydratedAssistant];
  const patches = renderNodeSnapshotPatches(before, next);
  const applied = applyRenderPatchesAndCollect(before, patches);

  assert.deepEqual(patches.map((patch) => `${patch.type}:${patch.node?.id || patch.id}`), [
    "replace:trail-message:turn-trail:msg-user",
    "replace:tool:list-files",
    "replace:trail-message:turn-trail:msg-summary"
  ]);
  assert.deepEqual(
    applied.nodes.map((node) => `${node.id}:${node.source}:${node.turnId}`),
    [
      "message:user:turn-live:trail:turn-trail",
      "tool:list-files:live:trail:turn-trail",
      "message:assistant:msg-summary:trail:turn-trail"
    ]
  );
  assert.deepEqual(applied.nodes.map((node) => node.timelineOrder), [1, 2, 3]);
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
    source: "trail",
    status: "completed",
    toolStatus: "completed"
  };
  const checkpoint: RenderNode = {
    id: "trail-checkpoint:turn-1",
    kind: "checkpoint",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    source: "trail",
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
    ["remove:message:assistant:live", "replace:tool:run-tests", "upsert:trail-checkpoint:turn-1"]
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

test("keeps thought stream message id when later chunks omit it", () => {
  const [existing] = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_thought_chunk",
        messageId: "thought-id-preserved",
        content: {
          type: "text",
          text: "Inspect"
        }
      },
      context
    )
  );
  assert.equal(existing?.kind, "thought");
  if (existing?.kind !== "thought") {
    throw new Error("expected existing thought node");
  }
  const incoming: Extract<RenderNode, { kind: "thought" }> = {
    ...existing,
    acpMessageId: undefined,
    content: [{ type: "text", text: " files" }]
  };

  const nodes = applyRenderPatches([existing], [{ type: "upsert", node: incoming }]);
  const thought = nodes[0];

  assert.equal(thought?.kind, "thought");
  if (thought?.kind !== "thought") {
    throw new Error("expected merged thought node");
  }
  assert.equal(thought.acpMessageId, "thought-id-preserved");
  assert.deepEqual(thought.content, [{ type: "text", text: "Inspect files" }]);
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

test("renders live thought chunks carried by delta aliases", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "agent_thought_chunk",
        message_id: "thought-delta-alias",
        delta: "Thinking from delta."
      } as unknown as SessionUpdate,
      context
    )
  );

  const node = nodes[0];
  assert.equal(node?.kind, "thought");
  if (node?.kind !== "thought") {
    throw new Error("expected aliased thought node");
  }
  assert.equal(node.id, "thought:thought-delta-alias");
  assert.deepEqual(node.content, [{ type: "text", text: "Thinking from delta." }]);
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

test("expands snake-case tool diff content into stable diff nodes", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-snake-diff",
      title: "Edit package",
      kind: "edit",
      status: "completed",
      content: [
        {
          type: "diff",
          file: "package.json",
          old_text: "{\"scripts\":{}}",
          new_text: "{\"scripts\":{\"test\":\"node --test\"}}"
        }
      ]
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  const diff = nodes.find((node): node is Extract<RenderNode, { kind: "diff" }> => node.kind === "diff");
  assert.equal(diff?.id, "diff:tool-snake-diff:package.json");
  assert.equal(diff?.path, "package.json");
  assert.equal(diff?.oldText, "{\"scripts\":{}}");
  assert.equal(diff?.newText, "{\"scripts\":{\"test\":\"node --test\"}}");
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

test("expands snake-case terminal content into stable terminal nodes", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-snake-terminal",
      title: "Run snake terminal",
      kind: "execute",
      status: "completed",
      content: [
        {
          type: "terminal",
          terminal_id: "term-snake",
          command_line: "npm run build",
          working_directory: "/workspace/project",
          exit_code: 0,
          elapsed_ms: 3400,
          stdout_preview: "built ok"
        }
      ]
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  const terminal = nodes.find((node): node is Extract<RenderNode, { kind: "terminal" }> => node.kind === "terminal");
  assert.equal(terminal?.id, "terminal:tool-snake-terminal:term-snake");
  assert.equal(terminal?.terminalId, "term-snake");
  assert.equal(terminal?.command, "npm run build");
  assert.equal(terminal?.cwd, "/workspace/project");
  assert.equal(terminal?.exitCode, 0);
  assert.equal(terminal?.elapsedMs, 3400);
  assert.equal(terminal?.stdout, "built ok");
});

test("normalizes terminal content state aliases when expanding tools", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-terminal-state-alias",
      title: "Run aliased terminal",
      kind: "execute",
      status: "completed",
      content: [
        {
          type: "terminal",
          terminalId: "term-state-alias",
          command: "npm test",
          state: "succeeded",
          stdout: "ok"
        }
      ]
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.status, "completed");
  assert.equal(terminal?.terminalStatus, "completed");
});

test("expands singular tool content and location records", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-singular-content",
      title: "Run singular command",
      kind: "execute",
      status: "completed",
      location: {
        path: "package.json",
        line: 12
      },
      content: {
        type: "terminal",
        terminalId: "term-singular",
        command: "npm run singular",
        status: "exited",
        stdout: "singular ok"
      }
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  const tool = nodes.find((node) => node.kind === "tool");
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(tool?.kind, "tool");
  assert.deepEqual(tool?.locations, [{ path: "package.json", line: 12 }]);
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.terminalId, "term-singular");
  assert.equal(terminal?.stdout, "singular ok");
});

test("normalizes untyped tool content aliases into rendered content blocks", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-content-alias",
        title: "Render aliased content",
        kind: "other",
        status: "completed",
        content: [
          {
            content: "Rendered from an untyped content alias."
          },
          {
            content: {
              type: "text",
              value: "Rendered from a nested content alias."
            }
          }
        ]
      } as unknown as SessionUpdate,
      context
    )
  );

  const tool = nodes.find((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool");
  assert.equal(tool?.kind, "tool");
  assert.deepEqual(tool?.content, [
    {
      type: "content",
      content: {
        type: "text",
        text: "Rendered from an untyped content alias."
      }
    },
    {
      type: "content",
      content: {
        type: "text",
        value: "Rendered from a nested content alias.",
        text: "Rendered from a nested content alias."
      }
    }
  ]);
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

test("recovers snake-case raw command output into terminal content", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-snake-raw-terminal",
        title: "Bash",
        kind: "other",
        status: "completed",
        raw_input: { command: ["npm", "test"] },
        raw_output: {
          output: {
            exit_code: 0,
            formatted_output: "tests passed\n"
          }
        }
      } as unknown as SessionUpdate,
      context
    )
  );
  const tool = nodes.find((node) => node.kind === "tool");
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(tool?.kind, "tool");
  assert.deepEqual(tool?.rawInput, { command: ["npm", "test"] });
  assert.deepEqual(tool?.rawOutput, {
    output: {
      exit_code: 0,
      formatted_output: "tests passed\n"
    }
  });
  assert.equal((tool?.content[0] as Record<string, unknown> | undefined)?.type, "terminal");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.terminalId, "tool-snake-raw-terminal");
  assert.equal(terminal?.command, "npm test");
  assert.equal(terminal?.exitCode, 0);
  assert.equal(terminal?.stdout, "tests passed");
});

test("recovers snake-case raw command output from tool updates", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call_update",
        tool_call_id: "tool-snake-update-terminal",
        state: "succeeded",
        raw_input: { command: "pnpm test" },
        raw_output: {
          stdout: "updated tests passed\n"
        }
      } as unknown as SessionUpdate,
      context
    )
  );
  const tool = nodes.find((node) => node.kind === "tool");
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.status, "completed");
  assert.deepEqual(tool?.rawInput, { command: "pnpm test" });
  assert.deepEqual(tool?.rawOutput, { stdout: "updated tests passed\n" });
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.terminalId, "tool-snake-update-terminal");
  assert.equal(terminal?.command, "pnpm test");
  assert.equal(terminal?.stdout, "updated tests passed");
});

test("recovers raw output when tool content is present but empty", () => {
  const nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-empty-content-output",
        title: "Bash",
        kind: "execute",
        status: "completed",
        content: "",
        rawInput: { command: "npm test" },
        rawOutput: {
          stdout: "tests passed\n"
        }
      } as unknown as SessionUpdate,
      context
    )
  );
  const tool = nodes.find((node) => node.kind === "tool");
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(tool?.kind, "tool");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.command, "npm test");
  assert.equal(terminal?.stdout, "tests passed");
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

test("syncs expanded terminal status from parent tool ACP id aliases", () => {
  const tool: Extract<RenderNode, { kind: "tool" }> = {
    id: "tool:display-tool",
    kind: "tool",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "in_progress",
    acpToolCallId: "provider-tool",
    toolCallId: "display-tool",
    title: "Run aliased command",
    toolKind: "execute",
    toolStatus: "in_progress",
    locations: [],
    content: []
  };
  const terminal: Extract<RenderNode, { kind: "terminal" }> = {
    id: "terminal:provider-tool:term-alias",
    kind: "terminal",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "sess-1",
    provider: "test-provider",
    source: "acp-live",
    status: "in_progress",
    acpToolCallId: "provider-tool",
    terminalId: "term-alias",
    terminalStatus: "running",
    stdout: "running"
  };
  const completed: Extract<RenderNode, { kind: "tool" }> = {
    ...tool,
    status: "completed",
    toolStatus: "completed",
    raw: { status: "completed" }
  };

  const applied = applyRenderPatchesAndCollect([tool, terminal], [{ type: "upsert", node: completed }]);
  const syncedTerminal = applied.nodes.find((node) => node.kind === "terminal");

  assert.equal(syncedTerminal?.kind, "terminal");
  assert.equal(syncedTerminal?.status, "completed");
  assert.equal(syncedTerminal?.terminalStatus, "completed");
  assert.deepEqual(
    applied.patches.map((patch) => patch.node?.id || patch.id).sort(),
    ["terminal:provider-tool:term-alias", "tool:display-tool"].sort()
  );
});

test("syncs expanded diff status from status-only tool updates", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-diff-sync",
        title: "Edit README.md",
        kind: "edit",
        status: "in_progress",
        content: [
          {
            type: "diff",
            path: "README.md",
            oldText: "before",
            newText: "after"
          }
        ]
      },
      context
    )
  );

  const update = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call_update",
      toolCallId: "tool-diff-sync",
      status: "completed"
    },
    context
  );
  const applied = applyRenderPatchesAndCollect(nodes, update);
  nodes = applied.nodes;

  assert.deepEqual(
    applied.patches.map((patch) => patch.node?.id || patch.id).sort(),
    ["diff:tool-diff-sync:README.md", "tool:tool-diff-sync"].sort()
  );
  const diff = nodes.find((node) => node.kind === "diff");
  assert.equal(diff?.kind, "diff");
  assert.equal(diff?.status, "completed");
  const tool = nodes.find((node) => node.kind === "tool");
  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.toolStatus, "completed");
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

test("appends explicit terminal stdout deltas without dropping prior output", () => {
  let nodes = applyRenderPatches(
    [],
    reduceSessionUpdate(
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool-terminal-delta",
        title: "Run streaming command",
        kind: "execute",
        status: "in_progress",
        content: [
          {
            type: "terminal",
            terminalId: "term-delta",
            command: "npm test",
            stdout: "line 1\n"
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
        toolCallId: "tool-terminal-delta",
        content: [
          {
            type: "terminal",
            terminalId: "term-delta",
            stdoutDelta: "line 2\n"
          }
        ]
      } as unknown as SessionUpdate,
      context
    )
  );

  const tool = nodes.find((node) => node.kind === "tool");
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(tool?.kind, "tool");
  assert.equal((tool?.content[0] as Record<string, unknown> | undefined)?.stdout, "line 1\nline 2\n");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.command, "npm test");
  assert.equal(terminal?.stdout, "line 1\nline 2\n");
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

test("normalizes provider tool state aliases on updates", () => {
  const start = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      toolCallId: "tool-state-alias",
      title: "Run aliased status",
      kind: "execute",
      status: "pending"
    },
    context
  );
  const update = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call_update",
      toolCallId: "tool-state-alias",
      state: "succeeded"
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], start), update);
  const tool = nodes.find((node) => node.kind === "tool");

  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.status, "completed");
  assert.equal(tool?.toolStatus, "completed");
});

test("normalizes provider tool call id aliases on live updates", () => {
  const start = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      tool_call_id: "tool-id-alias",
      title: "Run aliased id",
      kind: "execute",
      status: "pending"
    } as unknown as SessionUpdate,
    context
  );
  const update = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call_update",
      tool_call_id: "tool-id-alias",
      state: "succeeded"
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], start), update);
  const tool = nodes.find((node) => node.kind === "tool");

  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.id, "tool:tool-id-alias");
  assert.equal(tool?.toolCallId, "tool-id-alias");
  assert.equal(tool?.acpToolCallId, "tool-id-alias");
  assert.equal(tool?.status, "completed");
  assert.equal(tool?.toolStatus, "completed");
});

test("uses wrapped live tool call ids instead of unknown tool collisions", () => {
  const start = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call",
      tool_call_id: { id: 42 },
      title: "Read config",
      kind: "read",
      locations: [{ path: "config.json" }]
    } as unknown as SessionUpdate,
    context
  );
  const update = reduceSessionUpdate(
    {
      sessionUpdate: "tool_call_update",
      tool_call_id: { 0: 42 },
      status: "completed",
      content: [
        {
          type: "content",
          content: {
            type: "text",
            text: "Config read."
          }
        }
      ]
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches(applyRenderPatches([], start), update);
  const tool = nodes.find((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool");
  assert.equal(tool?.id, "tool:42");
  assert.equal(tool?.toolCallId, "42");
  assert.equal(tool?.acpToolCallId, "42");
  assert.equal(tool?.status, "completed");
  assert.equal(tool?.title, "Read config");
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

test("keeps approval requests distinct when providers reuse request ids across tools", () => {
  let nodes = applyRenderPatches(
    [],
    reducePermissionRequest(
      "100",
      {
        sessionId: "sess-1",
        toolCall: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-a",
          title: "Run first command",
          kind: "execute",
          status: "pending"
        },
        options: [{ optionId: "allow", name: "Allow once" }]
      },
      context
    )
  );
  nodes = applyRenderPatches(
    nodes,
    reducePermissionRequest(
      "100",
      {
        sessionId: "sess-1",
        toolCall: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-b",
          title: "Run second command",
          kind: "execute",
          status: "pending"
        },
        options: [{ optionId: "allow", name: "Allow once" }]
      },
      context
    )
  );

  assert.deepEqual(
    nodes.map((node) => node.id),
    ["approval:100", "approval:100:turn-1:sess-1:acp-live"]
  );
  assert.deepEqual(
    nodes.map((node) => node.kind === "approval" ? node.tool.toolCallId : undefined),
    ["tool-a", "tool-b"]
  );
  assert.deepEqual(
    nodes.map((node) => node.acpToolCallId),
    ["tool-a", "tool-b"]
  );
});

test("uses permission request session id for embedded approval tools", () => {
  const contextWithoutSession: RenderReduceContext = {
    taskId: context.taskId,
    lane: context.lane,
    currentTurnId: context.currentTurnId,
    provider: context.provider,
    now: context.now
  };
  const patches = reducePermissionRequest(
    "session-scope",
    {
      sessionId: "permission-session",
      toolCall: {
        sessionUpdate: "tool_call",
        toolCallId: "tool-permission-session",
        title: "Run permission command",
        kind: "execute",
        status: "pending"
      },
      options: [
        {
          optionId: "allow",
          name: "Allow"
        }
      ]
    },
    contextWithoutSession
  );

  const nodes = applyRenderPatches([], patches);
  const approval = nodes[0];
  assert.equal(approval?.kind, "approval");
  if (approval?.kind !== "approval") {
    throw new Error("expected approval node");
  }
  assert.equal(approval.acpSessionId, "permission-session");
  assert.equal(approval.tool.acpSessionId, "permission-session");
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

test("renders live plan update aliases as plan nodes", () => {
  const patches = reduceSessionUpdate(
    {
      sessionUpdate: "plan_update",
      steps: [
        "Inspect files",
        {
          name: "Run checks",
          state: "in_progress"
        }
      ]
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  const plan = nodes[0];
  assert.equal(plan?.kind, "plan");
  if (plan?.kind !== "plan") {
    throw new Error("expected plan node");
  }
  assert.equal(plan.entries[0]?.title, "Inspect files");
  assert.equal(plan.entries[1]?.title, "Run checks");
  assert.equal(plan.entries[1]?.status, "in_progress");
});

test("renders live usage updates with metric aliases", () => {
  const patches = reduceSessionUpdate(
    {
      session_update: "usage_update",
      usage: {
        total_tokens: 123
      },
      context_window: 200,
      cost: {
        usd: 0.01
      }
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  const usage = nodes[0];
  assert.equal(usage?.kind, "usage");
  if (usage?.kind !== "usage") {
    throw new Error("expected usage node");
  }
  assert.equal(usage.used, 123);
  assert.equal(usage.size, 200);
  assert.deepEqual(usage.cost, { usd: 0.01 });
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
          current_value: "fast",
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

test("hydrates snake-case initial session mode and config controls", () => {
  const patches = sessionControlsToPatches(
    {
      session_id: "sess-1",
      modes: {
        current_mode_id: "code",
        available_modes: [
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
      config_options: [
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
  assert.equal(mode?.modeId, "code");
  assert.equal(mode?.availableModes.length, 2);
  assert.equal(config?.kind, "config");
  assert.equal(config?.configOptions[0]?.id, "model");
  assert.equal(config?.configOptions[0]?.currentValue, "fast");
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

test("renders snake-case live control updates", () => {
  const updates: SessionUpdate[] = [
    {
      session_update: "current_mode_update",
      current_mode_id: "code",
      available_modes: [
        {
          id: "code",
          name: "Code"
        }
      ]
    } as unknown as SessionUpdate,
    {
      session_update: "config_option_update",
      config_options: [
        {
          id: "model",
          name: "Model",
          type: "select",
          current_value: "large"
        }
      ]
    } as unknown as SessionUpdate,
    {
      session_update: "available_commands_update",
      available_commands: [
        {
          name: "/review",
          description: "Review changes"
        }
      ]
    } as unknown as SessionUpdate,
    {
      session_update: "session_info_update",
      name: "Review task",
      updated_at: "2026-06-27T02:00:00.000Z"
    } as unknown as SessionUpdate
  ];

  const nodes = updates.reduce(
    (current, update) => applyRenderPatches(current, reduceSessionUpdate(update, context)),
    [] as RenderNode[]
  );

  const mode = nodes.find((node) => node.kind === "mode");
  const config = nodes.find((node) => node.kind === "config");
  const commands = nodes.find((node) => node.kind === "commands");
  const session = nodes.find((node) => node.kind === "session");
  assert.equal(mode?.kind, "mode");
  assert.equal(mode?.modeId, "code");
  assert.equal(mode?.availableModes[0]?.id, "code");
  assert.equal(config?.kind, "config");
  assert.equal(config?.configOptions[0]?.id, "model");
  assert.equal(config?.configOptions[0]?.currentValue, "large");
  assert.equal(commands?.kind, "commands");
  assert.equal(commands?.availableCommands[0]?.name, "/review");
  assert.equal(session?.kind, "session");
  assert.equal(session?.title, "Review task");
  assert.equal(session?.sessionUpdatedAt, "2026-06-27T02:00:00.000Z");
});

test("renders command-name-only live command updates", () => {
  const patches = reduceSessionUpdate(
    {
      session_update: "available_commands_update",
      command_names: ["/compact"]
    } as unknown as SessionUpdate,
    context
  );

  const nodes = applyRenderPatches([], patches);
  const commands = nodes[0];
  assert.equal(commands?.kind, "commands");
  assert.equal(commands?.availableCommands[0]?.name, "/compact");
  assert.equal(commands?.availableCommands[0]?.description, "");
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

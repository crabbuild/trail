import assert from "node:assert/strict";
import test from "node:test";
import {
  applyRenderPatches,
  reducePermissionRequest,
  reduceSessionUpdate,
  sessionControlsToPatches
} from "../shared/acpRenderReducers";
import type { RenderReduceContext } from "../shared/renderModel";

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
  assert.equal(nodes[0]?.content.length, 2);
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
  assert.equal(nodes[0]?.content.length, 2);
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

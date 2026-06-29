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
  assert.equal(nodes[0]?.content.length, 1);
  assert.deepEqual(nodes[0]?.content[0], { type: "text", text: "Hello world" });
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

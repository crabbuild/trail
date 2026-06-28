import assert from "node:assert/strict";
import test from "node:test";
import type { TaskView } from "../crabdb/TaskRepository";
import { hydrateTaskView, mergeHydratedNodes } from "../state/crabDbHydration";
import type { RenderNode } from "../shared/renderModel";

const view: TaskView = {
  task: {
    id: "task-1",
    lane: "lane-1",
    title: "Hydrate task",
    status: "ready",
    provider: "provider",
    changedPaths: ["README.md"],
    raw: {}
  },
  turns: [
    {
      turn: {
        turn_id: "turn-1",
        status: "completed",
        after_change: "ch_123"
      },
      messages: [
        {
          role: "user",
          body: "Please edit README",
          created_at: 1
        },
        {
          role: "assistant",
          body: "Done",
          created_at: 2
        }
      ],
      tool_summaries: ["edited README.md"],
      checkpoint: "ch_123"
    }
  ],
  messages: [],
  events: [],
  changes: [],
  raw: {}
};

test("hydrates persisted CrabDB transcript turns into render nodes", () => {
  const nodes = hydrateTaskView(view);
  assert.equal(nodes.filter((node) => node.kind === "message").length, 2);
  assert.equal(nodes.some((node) => node.kind === "tool" && node.title === "edited README.md"), true);
  assert.equal(nodes.some((node) => node.kind === "checkpoint" && node.checkpointId === "ch_123"), true);
  assert.equal(nodes.every((node) => node.source === "crabdb"), true);
});

test("hydrates persisted tool events into rich tool and terminal nodes", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        ...(view.turns[0] as Record<string, unknown>),
        tool_summaries: ["fallback summary"],
        events: [
          {
            event_type: "tool_call",
            created_at: 3,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-1",
              title: "Run tests",
              kind: "execute",
              status: "pending",
              rawInput: {
                command: ["npm", "test"]
              }
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 4,
            payload: {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool-1",
              status: "completed",
              content: [
                {
                  type: "terminal",
                  terminalId: "term-1",
                  command: ["npm", "test"],
                  status: "exited",
                  stdout: "ok"
                }
              ]
            }
          }
        ]
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const tool = nodes.find((node) => node.kind === "tool");
  const terminal = nodes.find((node) => node.kind === "terminal");
  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.title, "Run tests");
  assert.equal(tool?.toolKind, "execute");
  assert.equal(tool?.toolStatus, "completed");
  assert.equal(tool?.source, "crabdb");
  assert.equal(nodes.some((node) => node.kind === "tool" && node.title === "fallback summary"), false);
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.command, "npm test");
  assert.equal(terminal?.stdout, "ok");
});

test("orders hydrated turn messages and tools by recorded timeline timestamps", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-ordered",
          status: "completed",
          after_change: "ch_ordered",
          ended_at: 6
        },
        messages: [
          {
            role: "user",
            body: "Please inspect README",
            created_at: 1
          },
          {
            role: "assistant",
            body: "Done after the read",
            created_at: 5
          }
        ],
        events: [
          {
            event_type: "tool_call",
            created_at: 2,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-ordered",
              title: "Read README.md",
              kind: "read",
              status: "completed"
            }
          }
        ],
        checkpoint: "ch_ordered"
      }
    ]
  };

  const nodes = hydrateTaskView(orderedView);
  assert.deepEqual(
    nodes.map((node) => node.kind),
    ["message", "tool", "message", "checkpoint"]
  );
  const user = nodes[0];
  const assistant = nodes[2];
  assert.equal(user?.kind, "message");
  assert.equal(assistant?.kind, "message");
  if (user?.kind !== "message" || assistant?.kind !== "message") {
    throw new Error("expected hydrated messages around the tool call");
  }
  assert.equal(user.role, "user");
  assert.equal(assistant.role, "assistant");
});

test("hydrates reopened ACP turns by message-added event order around tools", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-interleaved",
          status: "completed",
          after_change: "ch_interleaved",
          ended_at: 6
        },
        messages: [
          {
            message_id: "msg-user",
            role: "user",
            body: "Please inspect README",
            created_at: 1
          },
          {
            message_id: "msg-before",
            role: "assistant",
            body: "I will read it first.",
            created_at: 2
          },
          {
            message_id: "msg-after",
            role: "assistant",
            body: "The read is done.",
            created_at: 5
          }
        ],
        events: [
          {
            event_type: "message_added",
            message_id: "msg-user",
            created_at: 1
          },
          {
            event_type: "message_added",
            message_id: "msg-before",
            created_at: 2
          },
          {
            event_type: "tool_call",
            created_at: 3,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-read",
              title: "Read README.md",
              kind: "read",
              status: "pending"
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 4,
            payload: {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool-read",
              status: "completed"
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-after",
            created_at: 5
          }
        ],
        checkpoint: "ch_interleaved"
      }
    ]
  };

  const nodes = hydrateTaskView(orderedView);
  assert.deepEqual(
    nodes.map((node) => (node.kind === "message" ? `${node.kind}:${node.role}:${node.text}` : `${node.kind}:${node.kind === "tool" ? node.title : ""}`)),
    [
      "message:user:Please inspect README",
      "message:assistant:I will read it first.",
      "tool:Read README.md",
      "message:assistant:The read is done.",
      "checkpoint:"
    ]
  );
});

test("hydrates reopened ACP turns by message-added event order around plan updates", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-plan",
          status: "completed",
          after_change: "ch_plan",
          ended_at: 6
        },
        messages: [
          {
            message_id: "msg-user",
            role: "user",
            body: "Please make a plan",
            created_at: 1
          },
          {
            message_id: "msg-before-plan",
            role: "assistant",
            body: "I will break it down.",
            created_at: 2
          },
          {
            message_id: "msg-after-plan",
            role: "assistant",
            body: "Now I can start.",
            created_at: 4
          }
        ],
        events: [
          {
            event_type: "message_added",
            message_id: "msg-user",
            created_at: 1
          },
          {
            event_type: "message_added",
            message_id: "msg-before-plan",
            created_at: 2
          },
          {
            event_type: "plan_update",
            created_at: 3,
            payload: {
              sessionUpdate: "plan",
              entries: [{ title: "Inspect files", status: "completed" }]
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-after-plan",
            created_at: 4
          }
        ],
        checkpoint: "ch_plan"
      }
    ]
  };

  const nodes = hydrateTaskView(orderedView);
  assert.deepEqual(
    nodes.map((node) => (node.kind === "message" ? `${node.kind}:${node.role}:${node.text}` : node.kind)),
    [
      "message:user:Please make a plan",
      "message:assistant:I will break it down.",
      "plan",
      "message:assistant:Now I can start.",
      "checkpoint"
    ]
  );
  const plan = nodes[2];
  assert.equal(plan?.kind, "plan");
  if (plan?.kind !== "plan") {
    throw new Error("expected hydrated plan update");
  }
  assert.equal(plan.entries[0]?.title, "Inspect files");
});

test("keeps explicit hydrated tool update status after completed-turn fallback", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-failed-tool",
          status: "completed",
          after_change: "ch_failed_tool",
          ended_at: 5
        },
        messages: [
          {
            message_id: "msg-user",
            role: "user",
            body: "Run the command",
            created_at: 1
          }
        ],
        events: [
          {
            event_type: "message_added",
            message_id: "msg-user",
            created_at: 1
          },
          {
            event_type: "tool_call",
            created_at: 2,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-failed",
              title: "Run command",
              kind: "execute",
              status: "pending"
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 3,
            payload: {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool-failed",
              status: "failed"
            }
          }
        ],
        checkpoint: "ch_failed_tool"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const tool = nodes.find((node) => node.kind === "tool");
  assert.equal(tool?.kind, "tool");
  if (tool?.kind !== "tool") {
    throw new Error("expected hydrated tool");
  }
  assert.equal(tool.status, "failed");
  assert.equal(tool.toolStatus, "failed");
});

test("marks open tool events completed when hydrating a completed CrabDB turn", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        ...(view.turns[0] as Record<string, unknown>),
        status: "completed",
        events: [
          {
            event_type: "tool_call",
            created_at: 3,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-read",
              title: "Read README.md",
              kind: "read",
              status: "in_progress"
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 4,
            payload: {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool-read",
              content: [
                {
                  type: "content",
                  content: {
                    type: "text",
                    text: "# README"
                  }
                }
              ]
            }
          }
        ]
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const tool = nodes.find((node) => node.kind === "tool");
  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.status, "completed");
  assert.equal(tool?.toolStatus, "completed");
});

test("keeps in-progress live nodes when merging hydrated state", () => {
  const live: RenderNode = {
    id: "message:assistant:streaming",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    source: "acp-live",
    status: "in_progress",
    role: "assistant",
    content: [{ type: "text", text: "working" }],
    text: "working",
    streaming: true
  };

  const merged = mergeHydratedNodes(hydrateTaskView(view), [live]);
  assert.equal(merged.some((node) => node.id === live.id), true);
});

test("drops checkpoint-pending completion placeholders after CrabDB hydration", () => {
  const liveCompletion: RenderNode = {
    id: "completion:turn-live",
    kind: "completion",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-live",
    source: "acp-live",
    status: "pending",
    stopReason: "end_turn",
    label: "Turn complete; checkpoint pending",
    checkpointPending: true
  };

  const merged = mergeHydratedNodes(hydrateTaskView(view), [liveCompletion]);
  assert.equal(merged.some((node) => node.id === liveCompletion.id), false);
  assert.deepEqual(
    [...new Set(merged.map((node) => node.turnId).filter(Boolean))],
    ["turn-1"]
  );
});

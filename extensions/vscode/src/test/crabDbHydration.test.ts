import assert from "node:assert/strict";
import test from "node:test";
import type { TaskView } from "../crabdb/TaskRepository";
import { hydrateTaskView, mergeHydratedNodes } from "../state/crabDbHydration";
import { applyRenderPatches, renderNodeSnapshotPatches } from "../shared/acpRenderReducers";
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
      tool_summaries: ["ACP prompt turn", "span_ended (completed)", "edited README.md"],
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
  assert.equal(nodes.some((node) => node.kind === "tool" && node.title === "ACP prompt turn"), false);
  assert.equal(nodes.some((node) => node.kind === "tool" && node.title === "span_ended (completed)"), false);
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

test("hydrates duplicate ACP message ids by consuming message-added events in order", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-duplicate-message-id",
          status: "completed",
          after_change: "ch_duplicate_message_id",
          ended_at: 7
        },
        messages: [
          {
            message_id: "msg-user",
            role: "user",
            body: "Run both checks",
            created_at: 1
          },
          {
            message_id: "msg-shared",
            role: "assistant",
            body: "First check is next.",
            created_at: 2
          },
          {
            message_id: "msg-shared",
            role: "assistant",
            body: "Second check is next.",
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
            message_id: "msg-shared",
            created_at: 2
          },
          {
            event_type: "tool_call",
            created_at: 3,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "first-check",
              title: "First check",
              kind: "execute",
              status: "completed"
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-shared",
            created_at: 4
          },
          {
            event_type: "tool_call",
            created_at: 5,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "second-check",
              title: "Second check",
              kind: "execute",
              status: "completed"
            }
          }
        ],
        checkpoint: "ch_duplicate_message_id"
      }
    ]
  };

  const nodes = hydrateTaskView(orderedView);
  assert.deepEqual(
    nodes.map((node) => (node.kind === "message" ? `${node.kind}:${node.role}:${node.text}` : `${node.kind}:${node.kind === "tool" ? node.toolCallId : ""}`)),
    [
      "message:user:Run both checks",
      "message:assistant:First check is next.",
      "tool:first-check",
      "message:assistant:Second check is next.",
      "tool:second-check",
      "checkpoint:"
    ]
  );
  assert.deepEqual(
    nodes.filter((node) => node.kind === "message").map((node) => node.id),
    [
      "crabdb-message:turn-duplicate-message-id:msg-user",
      "crabdb-message:turn-duplicate-message-id:msg-shared:1",
      "crabdb-message:turn-duplicate-message-id:msg-shared:2"
    ]
  );
});

test("hydrates duplicate tool ids across turns with unique render ids", () => {
  const duplicateToolView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-tool-a",
          status: "completed",
          after_change: "ch_tool_a",
          ended_at: 3
        },
        messages: [],
        events: [
          {
            event_type: "tool_call",
            created_at: 1,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "shared-tool",
              title: "Run shared tool A",
              kind: "execute",
              status: "completed",
              content: [
                {
                  type: "terminal",
                  terminalId: "shared-terminal",
                  stdout: "A"
                }
              ]
            }
          }
        ],
        checkpoint: "ch_tool_a"
      },
      {
        turn: {
          turn_id: "turn-tool-b",
          status: "completed",
          after_change: "ch_tool_b",
          ended_at: 6
        },
        messages: [],
        events: [
          {
            event_type: "tool_call",
            created_at: 4,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "shared-tool",
              title: "Run shared tool B",
              kind: "execute",
              status: "completed",
              content: [
                {
                  type: "terminal",
                  terminalId: "shared-terminal",
                  stdout: "B"
                }
              ]
            }
          }
        ],
        checkpoint: "ch_tool_b"
      }
    ]
  };

  const nodes = hydrateTaskView(duplicateToolView);
  assert.equal(new Set(nodes.map((node) => node.id)).size, nodes.length);
  const tools = nodes.filter((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool");
  assert.deepEqual(
    tools.map((node) => [node.id, node.turnId, node.title]),
    [
      ["tool:shared-tool", "turn-tool-a", "Run shared tool A"],
      ["tool:shared-tool:turn-tool-b:crabdb", "turn-tool-b", "Run shared tool B"]
    ]
  );
  const terminals = nodes.filter((node): node is Extract<RenderNode, { kind: "terminal" }> => node.kind === "terminal");
  assert.deepEqual(
    terminals.map((node) => [node.id, node.turnId, node.stdout]),
    [
      ["terminal:shared-tool:shared-terminal", "turn-tool-a", "A"],
      ["terminal:shared-tool:shared-terminal:turn-tool-b:crabdb", "turn-tool-b", "B"]
    ]
  );
});

test("preserves live transcript order when replacing a completed stream with hydrated nodes", () => {
  const current: RenderNode[] = [
    {
      id: "message:user:anonymous",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-order",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "user",
      content: [{ type: "text", text: "Count the repo" }],
      text: "Count the repo",
      streaming: false
    },
    {
      id: "message:assistant:anonymous",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-order",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      content: [{ type: "text", text: "I will inspect the repo first." }],
      text: "I will inspect the repo first.",
      streaming: false
    },
    {
      id: "tool:find-src",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-order",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      acpToolCallId: "find-src",
      toolCallId: "find-src",
      title: "Find src files",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: []
    },
    {
      id: "message:assistant:anonymous:2",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-order",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      content: [{ type: "text", text: "I will also check file types." }],
      text: "I will also check file types.",
      streaming: false
    },
    {
      id: "tool:breakdown",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-order",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      acpToolCallId: "breakdown",
      toolCallId: "breakdown",
      title: "Breakdown by file type",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: []
    },
    {
      id: "message:assistant:anonymous:3",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-order",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      content: [{ type: "text", text: "Here is the summary." }],
      text: "Here is the summary.",
      streaming: false
    }
  ];
  const persisted: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-live-order",
          status: "completed",
          after_change: "ch_live_order"
        },
        messages: [
          { role: "user", body: "Count the repo", created_at: 0 },
          { role: "assistant", body: "I will inspect the repo first.", created_at: 0 },
          { role: "assistant", body: "I will also check file types.", created_at: 0 },
          { role: "assistant", body: "Here is the summary.", created_at: 0 }
        ],
        events: [
          {
            event_type: "tool_call",
            created_at: 0,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "find-src",
              title: "Find src files",
              kind: "execute",
              status: "completed"
            }
          },
          {
            event_type: "tool_call",
            created_at: 0,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "breakdown",
              title: "Breakdown by file type",
              kind: "execute",
              status: "completed"
            }
          }
        ],
        checkpoint: "ch_live_order"
      }
    ]
  };

  const hydrated = hydrateTaskView(persisted);
  assert.deepEqual(
    hydrated.slice(0, 6).map((node) => (node.kind === "message" ? `${node.kind}:${node.role}:${node.text}` : `${node.kind}:${node.kind === "tool" ? node.toolCallId : ""}`)),
    [
      "message:user:Count the repo",
      "tool:find-src",
      "tool:breakdown",
      "message:assistant:I will inspect the repo first.",
      "message:assistant:I will also check file types.",
      "message:assistant:Here is the summary."
    ],
    "this fixture should reproduce the lossy CrabDB fallback order"
  );

  const merged = mergeHydratedNodes(hydrated, current);
  assert.deepEqual(
    merged.slice(0, 6).map((node) => (node.kind === "message" ? `${node.kind}:${node.role}:${node.text}` : `${node.kind}:${node.kind === "tool" ? node.toolCallId : ""}`)),
    [
      "message:user:Count the repo",
      "message:assistant:I will inspect the repo first.",
      "tool:find-src",
      "message:assistant:I will also check file types.",
      "tool:breakdown",
      "message:assistant:Here is the summary."
    ]
  );
  assert.deepEqual(merged.slice(0, 6).map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("uses message ids to reconcile repeated assistant text around tools", () => {
  const current: RenderNode[] = [
    {
      id: "message:user:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-duplicate-text",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "user",
      acpMessageId: "msg-user",
      content: [{ type: "text", text: "Repeat the status" }],
      text: "Repeat the status",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "message:assistant:msg-before",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-duplicate-text",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-before",
      content: [{ type: "text", text: "Done." }],
      text: "Done.",
      streaming: false,
      timelineOrder: 2
    },
    {
      id: "tool:repeat-check",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-duplicate-text",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      acpToolCallId: "repeat-check",
      toolCallId: "repeat-check",
      title: "Repeat check",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: [],
      timelineOrder: 3
    },
    {
      id: "message:assistant:msg-after",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-duplicate-text",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-after",
      content: [{ type: "text", text: "Done." }],
      text: "Done.",
      streaming: false,
      timelineOrder: 4
    }
  ];
  const persisted: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-duplicate-text",
          status: "completed",
          after_change: "ch_duplicate_text"
        },
        messages: [
          { message_id: "msg-user", role: "user", body: "Repeat the status", created_at: 0 },
          { message_id: "msg-after", role: "assistant", body: "Done.", created_at: 0 },
          { message_id: "msg-before", role: "assistant", body: "Done.", created_at: 0 }
        ],
        events: [
          {
            event_type: "tool_call",
            created_at: 0,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "repeat-check",
              title: "Repeat check",
              kind: "execute",
              status: "completed"
            }
          }
        ],
        checkpoint: "ch_duplicate_text"
      }
    ]
  };

  const hydrated = hydrateTaskView(persisted);
  assert.deepEqual(
    hydrated.filter((node) => node.kind === "message").map((node) => node.acpMessageId),
    ["msg-user", "msg-after", "msg-before"]
  );

  const merged = mergeHydratedNodes(hydrated, current);
  assert.deepEqual(
    merged.slice(0, 4).map((node) =>
      node.kind === "message" ? `${node.role}:${node.acpMessageId}:${node.text}` : `${node.kind}:${node.kind === "tool" ? node.toolCallId : ""}`
    ),
    [
      "user:msg-user:Repeat the status",
      "assistant:msg-before:Done.",
      "tool:repeat-check",
      "assistant:msg-after:Done."
    ]
  );
  assert.deepEqual(merged.slice(0, 4).map((node) => node.timelineOrder), [1, 2, 3, 4]);
});

test("keeps completed live assistant text until CrabDB hydration contains an equivalent message", () => {
  const current: RenderNode[] = [
    {
      id: "message:user:turn-partial",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-partial",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "user",
      content: [{ type: "text", text: "Summarize the repo" }],
      text: "Summarize the repo",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "message:assistant:turn-partial",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-partial",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      content: [{ type: "text", text: "The repo contains a VS Code extension." }],
      text: "The repo contains a VS Code extension.",
      streaming: false,
      timelineOrder: 2
    }
  ];
  const partialView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-partial",
          status: "completed",
          after_change: "ch_partial"
        },
        messages: [
          {
            message_id: "msg-user",
            role: "user",
            body: "Summarize the repo",
            created_at: 1
          }
        ],
        events: [
          {
            event_type: "message_added",
            message_id: "msg-user",
            created_at: 1
          }
        ],
        checkpoint: "ch_partial"
      }
    ]
  };

  const merged = mergeHydratedNodes(hydrateTaskView(partialView), current);

  assert.deepEqual(
    merged.map((node) => (node.kind === "message" ? `${node.source}:${node.role}:${node.text}` : `${node.source}:${node.kind}`)),
    [
      "crabdb:user:Summarize the repo",
      "acp-live:assistant:The repo contains a VS Code extension.",
      "crabdb:checkpoint"
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2, 3]);
});

test("keeps richer completed live assistant text when hydrated message is truncated", () => {
  const hydrated: RenderNode = {
    id: "crabdb-message:turn-truncated:msg-1",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-truncated",
    source: "crabdb",
    status: "completed",
    role: "assistant",
    acpMessageId: "msg-1",
    content: [{ type: "text", text: "The result is" }],
    text: "The result is",
    streaming: false,
    timelineOrder: 1
  };
  const live: RenderNode = {
    ...hydrated,
    id: "message:assistant:msg-1",
    source: "acp-live",
    content: [{ type: "text", text: "The result is fully rendered." }],
    text: "The result is fully rendered.",
    timelineOrder: 1
  };

  const merged = mergeHydratedNodes([hydrated], [live]);

  assert.deepEqual(
    merged.map((node) => (node.kind === "message" ? `${node.source}:${node.text}` : `${node.source}:${node.kind}`)),
    ["acp-live:The result is fully rendered."]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1]);
});

test("keeps completed live assistant text when hydrated message body diverges", () => {
  const hydrated: RenderNode = {
    id: "crabdb-message:turn-divergent:msg-1",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-divergent",
    source: "crabdb",
    status: "completed",
    role: "assistant",
    acpMessageId: "msg-1",
    content: [{ type: "text", text: "A different persisted answer with more words." }],
    text: "A different persisted answer with more words.",
    streaming: false,
    timelineOrder: 1
  };
  const live: RenderNode = {
    ...hydrated,
    id: "message:assistant:msg-1",
    source: "acp-live",
    content: [{ type: "text", text: "The live final answer." }],
    text: "The live final answer.",
    timelineOrder: 1
  };

  const merged = mergeHydratedNodes([hydrated], [live]);

  assert.deepEqual(
    merged.map((node) => (node.kind === "message" ? `${node.source}:${node.text}` : `${node.source}:${node.kind}`)),
    ["acp-live:The live final answer."]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1]);
});

test("keeps richer completed live tool details when hydrated tool is sparse", () => {
  const hydrated: RenderNode = {
    id: "tool:rich-tool",
    kind: "tool",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-rich-tool",
    source: "crabdb",
    status: "completed",
    acpToolCallId: "rich-tool",
    toolCallId: "rich-tool",
    title: "Tool call",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    content: [],
    timelineOrder: 1
  };
  const live: RenderNode = {
    ...hydrated,
    source: "acp-live",
    title: "Run npm test",
    toolKind: "execute",
    locations: [{ path: "package.json" }],
    content: [
      {
        type: "terminal",
        terminalId: "term-1",
        command: "npm test",
        stdout: "passing\n"
      }
    ],
    timelineOrder: 1
  };

  const merged = mergeHydratedNodes([hydrated], [live]);

  assert.deepEqual(
    merged.map((node) => (node.kind === "tool" ? `${node.source}:${node.title}:${node.content.length}` : `${node.source}:${node.kind}`)),
    ["acp-live:Run npm test:1"]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1]);
});

test("keeps older hydrated turns before live-order reconciliation for later turns", () => {
  const earlier: RenderNode = {
    id: "crabdb-message:turn-1:0",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    provider: "provider",
    source: "crabdb",
    status: "completed",
    role: "assistant",
    content: [{ type: "text", text: "Earlier turn" }],
    text: "Earlier turn",
    streaming: false
  };
  const activeUser: RenderNode = {
    id: "crabdb-message:turn-2:0",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-2",
    provider: "provider",
    source: "crabdb",
    status: "completed",
    role: "user",
    content: [{ type: "text", text: "Run another check" }],
    text: "Run another check",
    streaming: false
  };
  const activeAssistant: RenderNode = {
    id: "crabdb-message:turn-2:1",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-2",
    provider: "provider",
    source: "crabdb",
    status: "completed",
    role: "assistant",
    content: [{ type: "text", text: "I will run it." }],
    text: "I will run it.",
    streaming: false
  };
  const activeTool: RenderNode = {
    id: "tool:run-check",
    kind: "tool",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-2",
    provider: "provider",
    source: "crabdb",
    status: "completed",
    acpToolCallId: "run-check",
    toolCallId: "run-check",
    title: "Run check",
    toolKind: "execute",
    toolStatus: "completed",
    locations: [],
    content: []
  };
  const current: RenderNode[] = [
    { ...activeUser, id: "message:user:anonymous", source: "acp-live" },
    { ...activeAssistant, id: "message:assistant:anonymous", source: "acp-live" },
    { ...activeTool, source: "acp-live" }
  ];

  const merged = mergeHydratedNodes([earlier, activeUser, activeTool, activeAssistant], current);

  assert.deepEqual(
    merged.slice(0, 4).map((node) => (node.kind === "message" ? node.text : node.kind === "tool" ? node.toolCallId : node.kind)),
    ["Earlier turn", "Run another check", "I will run it.", "run-check"]
  );
  assert.deepEqual(merged.slice(0, 4).map((node) => node.timelineOrder), [1, 2, 3, 4]);
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

test("keeps in-progress live tools when older hydrated turns reuse provider ids", () => {
  const hydrated: RenderNode = {
    id: "tool:shared-provider-id",
    kind: "tool",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    provider: "provider",
    source: "crabdb",
    status: "completed",
    acpToolCallId: "shared-provider-id",
    toolCallId: "shared-provider-id",
    title: "Old read",
    toolKind: "read",
    toolStatus: "completed",
    locations: [],
    content: [],
    timelineOrder: 1
  };
  const live: RenderNode = {
    ...hydrated,
    turnId: "turn-2",
    source: "acp-live",
    status: "in_progress",
    title: "Current read",
    toolStatus: "in_progress",
    timelineOrder: 2
  };

  const merged = mergeHydratedNodes([hydrated], [live]);

  assert.deepEqual(
    merged.map((node) => `${node.source}:${node.kind === "tool" ? node.title : node.kind}:${node.id}`),
    [
      "crabdb:Old read:tool:shared-provider-id",
      "acp-live:Current read:tool:shared-provider-id:turn-2:acp-live"
    ]
  );
  assert.equal(new Set(merged.map((node) => node.id)).size, merged.length);
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);

  const patches = renderNodeSnapshotPatches([live], merged);
  assert.deepEqual(applyRenderPatches([live], patches), merged);
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

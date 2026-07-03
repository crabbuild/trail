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

test("hydrates persisted CrabDB message content blocks when body is absent", () => {
  const contentView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-content-blocks",
          status: "completed",
          after_change: "ch_content_blocks"
        },
        messages: [
          {
            message_id: "msg-content-user",
            role: "user",
            content: [
              {
                type: "text",
                text: "Read the attached context"
              }
            ],
            created_at: 1
          },
          {
            message_id: "msg-content-assistant",
            role: "assistant",
            content: [
              {
                type: "text",
                text: "The result is ready."
              },
              {
                type: "resource",
                resource: {
                  uri: "file:///README.md",
                  text: "Attached context"
                }
              },
              {
                type: "image",
                data: "ignored",
                mimeType: "image/png"
              }
            ],
            created_at: 2
          }
        ],
        checkpoint: "ch_content_blocks"
      }
    ]
  };

  const nodes = hydrateTaskView(contentView);
  const assistant = nodes.find((node) => node.kind === "message" && node.role === "assistant");

  assert.equal(assistant?.kind, "message");
  if (assistant?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  assert.equal(assistant.text, "The result is ready.Attached context[image]");
  assert.deepEqual(
    assistant.content.map((block) => block.type),
    ["text", "resource", "image"]
  );
});

test("hydrates persisted CrabDB message text fields when body is absent", () => {
  const textView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-text-fields",
          status: "completed",
          after_change: "ch_text_fields"
        },
        messages: [
          {
            message_id: "msg-user-text",
            role: "user",
            text: "Review the summary",
            created_at: 1
          },
          {
            message_id: "msg-assistant-content-text",
            role: "assistant",
            contentText: "The summary is ready.",
            created_at: 2
          },
          {
            message_id: "msg-assistant-message",
            role: "assistant",
            message: "No further action needed.",
            created_at: 3
          }
        ],
        checkpoint: "ch_text_fields"
      }
    ]
  };

  const nodes = hydrateTaskView(textView);
  const messages = nodes.filter((node) => node.kind === "message");

  assert.deepEqual(
    messages.map((node) => `${node.role}:${node.text}`),
    [
      "user:Review the summary",
      "assistant:The summary is ready.",
      "assistant:No further action needed."
    ]
  );
});

test("hydrates persisted CrabDB string content when body is absent", () => {
  const textView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-string-content",
          status: "completed",
          after_change: "ch_string_content"
        },
        messages: [
          {
            message_id: "msg-string-content",
            role: "assistant",
            content: "Rendered from a string content field.",
            created_at: 1
          }
        ],
        checkpoint: "ch_string_content"
      }
    ]
  };

  const nodes = hydrateTaskView(textView);
  const assistant = nodes.find((node) => node.kind === "message" && node.role === "assistant");

  assert.equal(assistant?.kind, "message");
  if (assistant?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  assert.equal(assistant.text, "Rendered from a string content field.");
});

test("falls back to persisted CrabDB message row aliases when canonical content is empty", () => {
  const textView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-empty-row-content-alias",
          status: "completed",
          after_change: "ch_empty_row_content_alias"
        },
        messages: [
          {
            message_id: "msg-empty-row-content-alias",
            role: "assistant",
            content: "",
            contentDelta: "Rendered from row alias.",
            created_at: 1
          }
        ],
        checkpoint: "ch_empty_row_content_alias"
      }
    ]
  };

  const nodes = hydrateTaskView(textView);
  const assistant = nodes.find((node) => node.kind === "message" && node.role === "assistant");

  assert.equal(assistant?.kind, "message");
  if (assistant?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  assert.equal(assistant.text, "Rendered from row alias.");
});

test("hydrates persisted CrabDB mixed content arrays when body is absent", () => {
  const contentView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-mixed-content-array",
          status: "completed",
          after_change: "ch_mixed_content_array"
        },
        messages: [
          {
            message_id: "msg-mixed-content-array",
            role: "assistant",
            content: [
              "Rendered ",
              {
                text: "from "
              },
              {
                type: "text",
                text: "mixed content."
              }
            ],
            created_at: 1
          }
        ],
        checkpoint: "ch_mixed_content_array"
      }
    ]
  };

  const nodes = hydrateTaskView(contentView);
  const assistant = nodes.find((node) => node.kind === "message" && node.role === "assistant");

  assert.equal(assistant?.kind, "message");
  if (assistant?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  assert.equal(assistant.text, "Rendered from mixed content.");
  assert.deepEqual(
    assistant.content.map((block) => block.type),
    ["text", "text", "text"]
  );
});

test("hydrates persisted CrabDB text content blocks with aliased text fields", () => {
  const contentView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-text-block-aliases",
          status: "completed",
          after_change: "ch_text_block_aliases"
        },
        messages: [
          {
            message_id: "msg-text-block-aliases",
            role: "assistant",
            content: [
              {
                type: "text",
                content: "Rendered from content. "
              },
              {
                type: "text",
                value: "Rendered from value."
              }
            ],
            created_at: 1
          }
        ],
        checkpoint: "ch_text_block_aliases"
      }
    ]
  };

  const nodes = hydrateTaskView(contentView);
  const assistant = nodes.find((node) => node.kind === "message" && node.role === "assistant");

  assert.equal(assistant?.kind, "message");
  if (assistant?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  assert.equal(assistant.text, "Rendered from content. Rendered from value.");
});

test("hydrates persisted CrabDB message rows wrapped in nested message objects", () => {
  const contentView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-nested-message-rows",
          status: "completed",
          after_change: "ch_nested_message_rows"
        },
        messages: [
          {
            message: {
              message_id: "msg-nested-row-user",
              role: "user",
              text: "Render the nested user row."
            },
            created_at: 1
          },
          {
            message: {
              id: "msg-nested-row-assistant",
              role: "assistant",
              content: {
                type: "text",
                value: "Render the nested assistant row."
              }
            },
            created_at: 2
          }
        ],
        checkpoint: "ch_nested_message_rows"
      }
    ]
  };

  const nodes = hydrateTaskView(contentView);

  assert.deepEqual(
    nodes.filter((node) => node.kind === "message").map((node) => `${node.role}:${node.acpMessageId}:${node.text}`),
    [
      "user:msg-nested-row-user:Render the nested user row.",
      "assistant:msg-nested-row-assistant:Render the nested assistant row."
    ]
  );
});

test("hydrates root transcript messages and events when turn wrappers are absent", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_root",
      updatedAt: "2026-06-27T00:00:05.000Z"
    },
    turns: [],
    messages: [
      {
        message_id: "msg-user",
        turn_id: "turn-root",
        role: "user",
        body: "Inspect the root transcript",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        message_id: "msg-assistant",
        turn_id: "turn-root",
        role: "assistant",
        body: "The root transcript rendered.",
        created_at: "2026-06-27T00:00:04.000Z"
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-root",
        message_id: "msg-user",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        event_type: "tool_call",
        turn_id: "turn-root",
        created_at: "2026-06-27T00:00:02.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-root",
          title: "Read root transcript",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-root",
        message_id: "msg-assistant",
        created_at: "2026-06-27T00:00:04.000Z"
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-root:user:Inspect the root transcript",
      "tool:turn-root:Read root transcript",
      "message:turn-root:assistant:The root transcript rendered.",
      "checkpoint:turn-root:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4]);
});

test("keeps separate root transcript turn scopes when wrappers are absent", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_beta",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message_id: "msg-alpha-user",
        turn_id: "turn-alpha",
        role: "user",
        body: "Start the first turn",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        message_id: "msg-alpha-assistant",
        turn_id: "turn-alpha",
        role: "assistant",
        body: "First turn is complete.",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        message_id: "msg-beta-user",
        turn_id: "turn-beta",
        role: "user",
        body: "Start the second turn",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        message_id: "msg-beta-assistant",
        turn_id: "turn-beta",
        role: "assistant",
        body: "Second turn is complete.",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-alpha-user",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-alpha-assistant",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-beta-user",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        event_type: "tool_call",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:04.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-beta",
          title: "Inspect beta",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-beta-assistant",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-alpha:user:Start the first turn",
      "message:turn-alpha:assistant:First turn is complete.",
      "message:turn-beta:user:Start the second turn",
      "tool:turn-beta:Inspect beta",
      "message:turn-beta:assistant:Second turn is complete.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("uses root message-added events to scope messages without turn ids", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_scoped",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message_id: "msg-alpha-user",
        role: "user",
        body: "Start alpha without a message turn id",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        message_id: "msg-alpha-assistant",
        role: "assistant",
        body: "Alpha stayed scoped.",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        message_id: "msg-beta-user",
        role: "user",
        body: "Start beta without a message turn id",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        message_id: "msg-beta-assistant",
        role: "assistant",
        body: "Beta stayed scoped.",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-alpha-user",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-alpha-assistant",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-beta-user",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        event_type: "tool_call",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:04.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-beta",
          title: "Inspect beta",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-beta-assistant",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-alpha:user:Start alpha without a message turn id",
      "message:turn-alpha:assistant:Alpha stayed scoped.",
      "message:turn-beta:user:Start beta without a message turn id",
      "tool:turn-beta:Inspect beta",
      "message:turn-beta:assistant:Beta stayed scoped.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("uses numeric root message-added ids to scope messages without turn ids", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_numeric_scoped",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message_id: 101,
        role: "user",
        body: "Start alpha with numeric id",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        message_id: 102,
        role: "assistant",
        body: "Alpha numeric scope stayed correct.",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        message_id: 201,
        role: "user",
        body: "Start beta with numeric id",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        message_id: 202,
        role: "assistant",
        body: "Beta numeric scope stayed correct.",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: 101,
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: 102,
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: 201,
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        event_type: "tool_call",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:04.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-beta",
          title: "Inspect beta",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: 202,
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-alpha:user:101:Start alpha with numeric id",
      "message:turn-alpha:assistant:102:Alpha numeric scope stayed correct.",
      "message:turn-beta:user:201:Start beta with numeric id",
      "tool:turn-beta:Inspect beta",
      "message:turn-beta:assistant:202:Beta numeric scope stayed correct.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("uses nested root message-added event ids to scope messages without turn ids", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_nested_scoped",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message_id: "msg-alpha-user",
        role: "user",
        body: "Start alpha from nested scope",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        message_id: "msg-alpha-assistant",
        role: "assistant",
        body: "Alpha nested scope stayed correct.",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        message_id: "msg-beta-user",
        role: "user",
        body: "Start beta from nested scope",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        message_id: "msg-beta-assistant",
        role: "assistant",
        body: "Beta nested scope stayed correct.",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        created_at: "2026-06-27T00:00:01.000Z",
        payload: {
          message: {
            message_id: "msg-alpha-user"
          }
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        created_at: "2026-06-27T00:00:02.000Z",
        payload: {
          message: {
            id: "msg-alpha-assistant"
          }
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:03.000Z",
        payload: {
          message: {
            messageId: "msg-beta-user"
          }
        }
      },
      {
        event_type: "tool_call",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:04.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-beta",
          title: "Inspect beta",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:05.000Z",
        payload: {
          message: {
            id: "msg-beta-assistant"
          }
        }
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-alpha:user:msg-alpha-user:Start alpha from nested scope",
      "message:turn-alpha:assistant:msg-alpha-assistant:Alpha nested scope stayed correct.",
      "message:turn-beta:user:msg-beta-user:Start beta from nested scope",
      "tool:turn-beta:Inspect beta",
      "message:turn-beta:assistant:msg-beta-assistant:Beta nested scope stayed correct.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("uses wrapped numeric root message-added ids to scope messages without turn ids", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_wrapped_numeric_scoped",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message_id: { id: 301 },
        role: "user",
        body: "Start alpha with wrapped numeric id",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        message_id: { id: 302 },
        role: "assistant",
        body: "Alpha wrapped numeric scope stayed correct.",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        message_id: { id: 401 },
        role: "user",
        body: "Start beta with wrapped numeric id",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        message_id: { id: 402 },
        role: "assistant",
        body: "Beta wrapped numeric scope stayed correct.",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: { id: 301 },
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: { id: 302 },
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: { id: 401 },
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        event_type: "tool_call",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:04.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-wrapped-beta",
          title: "Inspect wrapped beta",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: { id: 402 },
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-alpha:user:301:Start alpha with wrapped numeric id",
      "message:turn-alpha:assistant:302:Alpha wrapped numeric scope stayed correct.",
      "message:turn-beta:user:401:Start beta with wrapped numeric id",
      "tool:turn-beta:Inspect wrapped beta",
      "message:turn-beta:assistant:402:Beta wrapped numeric scope stayed correct.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("uses root message-added events to scope nested message rows without turn ids", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_nested_rows_scoped",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message: {
          message_id: "msg-alpha-user",
          role: "user",
          body: "Start alpha from a nested row",
          created_at: "2026-06-27T00:00:01.000Z"
        }
      },
      {
        message: {
          message_id: "msg-alpha-assistant",
          role: "assistant",
          body: "Alpha nested row stayed scoped.",
          created_at: "2026-06-27T00:00:02.000Z"
        }
      },
      {
        message: {
          message_id: "msg-beta-user",
          role: "user",
          body: "Start beta from a nested row",
          created_at: "2026-06-27T00:00:03.000Z"
        }
      },
      {
        message: {
          message_id: "msg-beta-assistant",
          role: "assistant",
          body: "Beta nested row stayed scoped.",
          created_at: "2026-06-27T00:00:05.000Z"
        }
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-alpha-user",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-alpha-assistant",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-beta-user",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        event_type: "tool_call",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:04.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-beta",
          title: "Inspect beta",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-beta-assistant",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-alpha:user:msg-alpha-user:Start alpha from a nested row",
      "message:turn-alpha:assistant:msg-alpha-assistant:Alpha nested row stayed scoped.",
      "message:turn-beta:user:msg-beta-user:Start beta from a nested row",
      "tool:turn-beta:Inspect beta",
      "message:turn-beta:assistant:msg-beta-assistant:Beta nested row stayed scoped.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("orders root nested message rows by nested timestamps when turn wrappers are absent", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_nested_timestamp_order",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message: {
          message_id: "msg-beta-user",
          turn_id: "turn-beta",
          role: "user",
          body: "Start beta from a nested row",
          created_at: "2026-06-27T00:00:03.000Z"
        }
      },
      {
        message: {
          message_id: "msg-beta-assistant",
          turn_id: "turn-beta",
          role: "assistant",
          body: "Beta nested row finished.",
          created_at: "2026-06-27T00:00:04.000Z"
        }
      },
      {
        message: {
          message_id: "msg-alpha-user",
          turn_id: "turn-alpha",
          role: "user",
          body: "Start alpha from a nested row",
          created_at: "2026-06-27T00:00:01.000Z"
        }
      },
      {
        message: {
          message_id: "msg-alpha-assistant",
          turn_id: "turn-alpha",
          role: "assistant",
          body: "Alpha nested row finished.",
          created_at: "2026-06-27T00:00:02.000Z"
        }
      }
    ],
    events: []
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:${node.turnId}:`
    )),
    [
      "message:turn-alpha:user:msg-alpha-user:Start alpha from a nested row",
      "message:turn-alpha:assistant:msg-alpha-assistant:Alpha nested row finished.",
      "message:turn-beta:user:msg-beta-user:Start beta from a nested row",
      "message:turn-beta:assistant:msg-beta-assistant:Beta nested row finished.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5]);
});

test("uses root message-added event order to scope repeated message ids without turn ids", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_repeated_root",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message_id: "msg-alpha-user",
        role: "user",
        body: "Start alpha",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        message_id: "msg-shared",
        role: "assistant",
        body: "Alpha answer.",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        message_id: "msg-beta-user",
        role: "user",
        body: "Start beta",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        message_id: "msg-shared",
        role: "assistant",
        body: "Beta answer.",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-alpha-user",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-shared",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-beta-user",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        event_type: "tool_call",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:04.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-beta",
          title: "Inspect beta",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-shared",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-alpha:user:msg-alpha-user:Start alpha",
      "message:turn-alpha:assistant:msg-shared:Alpha answer.",
      "message:turn-beta:user:msg-beta-user:Start beta",
      "tool:turn-beta:Inspect beta",
      "message:turn-beta:assistant:msg-shared:Beta answer.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("uses root message timestamps to scope repeated message ids when messages arrive out of order", () => {
  const rootView: TaskView = {
    ...view,
    task: {
      ...view.task,
      latestCheckpoint: "ch_repeated_root_out_of_order",
      updatedAt: "2026-06-27T00:00:06.000Z"
    },
    turns: [],
    messages: [
      {
        message_id: "msg-alpha-user",
        role: "user",
        body: "Start alpha",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        message_id: "msg-shared",
        role: "assistant",
        body: "Beta answer.",
        created_at: "2026-06-27T00:00:05.000Z"
      },
      {
        message_id: "msg-beta-user",
        role: "user",
        body: "Start beta",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        message_id: "msg-shared",
        role: "assistant",
        body: "Alpha answer.",
        created_at: "2026-06-27T00:00:02.000Z"
      }
    ],
    events: [
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-alpha-user",
        created_at: "2026-06-27T00:00:01.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-alpha",
        message_id: "msg-shared",
        created_at: "2026-06-27T00:00:02.000Z"
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-beta-user",
        created_at: "2026-06-27T00:00:03.000Z"
      },
      {
        event_type: "tool_call",
        turn_id: "turn-beta",
        created_at: "2026-06-27T00:00:04.000Z",
        payload: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-beta",
          title: "Inspect beta",
          kind: "read",
          status: "completed"
        }
      },
      {
        event_type: "message_added",
        turn_id: "turn-beta",
        message_id: "msg-shared",
        created_at: "2026-06-27T00:00:05.000Z"
      }
    ]
  };

  const nodes = hydrateTaskView(rootView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.turnId}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:${node.turnId}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:turn-alpha:user:msg-alpha-user:Start alpha",
      "message:turn-alpha:assistant:msg-shared:Alpha answer.",
      "message:turn-beta:user:msg-beta-user:Start beta",
      "tool:turn-beta:Inspect beta",
      "message:turn-beta:assistant:msg-shared:Beta answer.",
      "checkpoint:turn-beta:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
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

test("hydrates flattened persisted ACP tool events", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        ...(view.turns[0] as Record<string, unknown>),
        tool_summaries: [],
        events: [
          {
            event_type: "tool_call",
            created_at: 3,
            sessionUpdate: "tool_call",
            toolCallId: "tool-flat",
            title: "Run flattened command",
            kind: "execute",
            status: "pending",
            rawInput: {
              command: "npm run check"
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 4,
            sessionUpdate: "tool_call_update",
            toolCallId: "tool-flat",
            status: "completed",
            content: [
              {
                type: "terminal",
                terminalId: "term-flat",
                command: "npm run check",
                status: "exited",
                stdout: "clean"
              }
            ]
          }
        ]
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const tool = nodes.find((node) => node.kind === "tool");
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.title, "Run flattened command");
  assert.equal(tool?.toolStatus, "completed");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.stdout, "clean");
});

test("infers persisted ACP tool events from event type and snake case payload fields", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        ...(view.turns[0] as Record<string, unknown>),
        tool_summaries: [],
        events: [
          {
            event_type: "tool_call",
            created_at: 3,
            payload: {
              tool_call_id: "tool-snake",
              title: "Run snake case command",
              kind: "execute",
              status: "pending",
              raw_input: {
                command: "npm run check"
              }
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 4,
            payload: {
              tool_call_id: "tool-snake",
              kind: "execute",
              status: "completed",
              raw_input: {
                command: "npm run check"
              },
              raw_output: {
                stdout: "snake clean"
              }
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
  assert.equal(tool?.title, "Run snake case command");
  assert.equal(tool?.toolStatus, "completed");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.command, "npm run check");
  assert.equal(terminal?.stdout, "snake clean");
});

test("infers persisted ACP tool events from nested tool payloads", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        ...(view.turns[0] as Record<string, unknown>),
        tool_summaries: [],
        events: [
          {
            event_type: "tool_call",
            created_at: 3,
            payload: {
              tool_call: {
                id: "tool-nested-payload",
                name: "Run nested command",
                kind: "execute",
                status: "pending",
                raw_input: {
                  command: "npm run nested"
                }
              }
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 4,
            payload: {
              toolCall: {
                id: "tool-nested-payload",
                kind: "execute",
                status: "completed",
                raw_input: {
                  command: "npm run nested"
                },
                raw_output: {
                  stdout: "nested clean"
                }
              }
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
  assert.equal(tool?.title, "Run nested command");
  assert.equal(tool?.toolStatus, "completed");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.command, "npm run nested");
  assert.equal(terminal?.stdout, "nested clean");
});

test("recovers persisted ACP tool update output from direct payload fields", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        ...(view.turns[0] as Record<string, unknown>),
        tool_summaries: [],
        events: [
          {
            event_type: "tool_call",
            created_at: 3,
            payload: {
              tool_call_id: "tool-direct-output",
              title: "Run direct output command",
              kind: "execute",
              status: "pending",
              raw_input: {
                command: "npm run direct"
              }
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 4,
            payload: {
              tool_call_id: "tool-direct-output",
              kind: "execute",
              status: "completed",
              raw_input: {
                command: "npm run direct"
              },
              stdout: "direct clean"
            }
          }
        ]
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const terminal = nodes.find((node) => node.kind === "terminal");

  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.command, "npm run direct");
  assert.equal(terminal?.stdout, "direct clean");
});

test("hydrates singular persisted ACP tool content and location records", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        ...(view.turns[0] as Record<string, unknown>),
        tool_summaries: [],
        events: [
          {
            event_type: "tool_call",
            created_at: 3,
            payload: {
              tool_call_id: "tool-singular-persisted",
              title: "Run singular persisted command",
              kind: "execute",
              status: "pending",
              location: {
                path: "package.json",
                line: 12
              }
            }
          },
          {
            event_type: "tool_call_update",
            created_at: 4,
            payload: {
              tool_call_id: "tool-singular-persisted",
              kind: "execute",
              status: "completed",
              content: {
                type: "terminal",
                terminalId: "term-singular-persisted",
                command: "npm run singular",
                status: "exited",
                stdout: "persisted singular ok"
              }
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
  assert.deepEqual(tool?.locations, [{ path: "package.json", line: 12 }]);
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.terminalId, "term-singular-persisted");
  assert.equal(terminal?.stdout, "persisted singular ok");
});

test("infers persisted tool spans from span events without ACP session updates", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        ...(view.turns[0] as Record<string, unknown>),
        tool_summaries: [],
        events: [
          {
            event_type: "span_started",
            created_at: 3,
            payload: {
              span_id: "span-tool",
              span_type: "tool",
              name: "Run span command",
              attributes: {
                kind: "execute",
                raw_input: {
                  command: "npm test"
                }
              }
            }
          },
          {
            event_type: "span_ended",
            created_at: 4,
            payload: {
              span_id: "span-tool",
              status: "completed",
              result: {
                kind: "execute",
                raw_input: {
                  command: "npm test"
                },
                raw_output: {
                  stdout: "span ok"
                }
              }
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
  assert.equal(tool?.title, "Run span command");
  assert.equal(tool?.toolStatus, "completed");
  assert.equal(terminal?.kind, "terminal");
  assert.equal(terminal?.command, "npm test");
  assert.equal(terminal?.stdout, "span ok");
});

test("hydrates persisted ACP message and thought events without message rows", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-acp-message-events",
          status: "completed",
          after_change: "ch_acp_message_events",
          ended_at: 6
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "acp_user_message_chunk",
            created_at: 1,
            payload: {
              message_id: "msg-user-event",
              content: "Please inspect the event transcript"
            }
          },
          {
            event_type: "tool_call",
            created_at: 2,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-event",
              title: "Read event log",
              kind: "read",
              status: "completed"
            }
          },
          {
            event_type: "agent_thought_chunk",
            created_at: 3,
            message_id: "thought-event",
            content: {
              type: "text",
              text: "Checking persisted events"
            }
          },
          {
            event_type: "agent_message_chunk",
            created_at: 4,
            message_id: "msg-assistant-event",
            content: {
              type: "text",
              value: "The persisted events rendered."
            }
          }
        ],
        checkpoint: "ch_acp_message_events"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.acpMessageId}:${node.text}`
        : node.kind === "thought"
          ? `${node.kind}:${node.acpMessageId}:${node.content.map((block) => block.type === "text" ? block.text : block.type).join("")}`
          : `${node.kind}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:user:msg-user-event:Please inspect the event transcript",
      "tool:Read event log",
      "thought:thought-event:Checking persisted events",
      "message:assistant:msg-assistant-event:The persisted events rendered.",
      "checkpoint:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5]);
});

test("hydrates raw persisted ACP session update payloads", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-raw-session-updates",
          status: "completed",
          after_change: "ch_raw_session_updates",
          ended_at: 6
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "acp_session_update",
            created_at: 1,
            payload: {
              method: "session/update",
              params: {
                sessionId: "sess-raw",
                update: {
                  sessionUpdate: "agent_message_chunk",
                  messageId: "msg-raw-session",
                  content: {
                    type: "text",
                    text: "Rendered from raw session update."
                  }
                }
              }
            }
          },
          {
            event_type: "acp_session_update",
            created_at: 2,
            payload: {
              update: {
                sessionUpdate: "tool_call",
                toolCallId: "tool-raw-session",
                title: "Run raw session command",
                kind: "execute",
                status: "pending",
                rawInput: {
                  command: "npm test"
                }
              }
            }
          },
          {
            event_type: "acp_session_update",
            created_at: 3,
            payload: {
              session_update: {
                sessionUpdate: "tool_call_update",
                toolCallId: "tool-raw-session",
                status: "completed",
                content: [
                  {
                    type: "terminal",
                    terminalId: "term-raw-session",
                    command: "npm test",
                    status: "exited",
                    stdout: "raw ok"
                  }
                ]
              }
            }
          }
        ],
        checkpoint: "ch_raw_session_updates"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.acpMessageId}:${node.text}`
        : node.kind === "tool"
          ? `${node.kind}:${node.toolCallId}:${node.title}:${node.toolStatus}`
          : node.kind === "terminal"
            ? `${node.kind}:${node.terminalId}:${node.stdout}`
            : `${node.kind}:`
    )),
    [
      "message:assistant:msg-raw-session:Rendered from raw session update.",
      "tool:tool-raw-session:Run raw session command:completed",
      "terminal:term-raw-session:raw ok",
      "checkpoint:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4]);
});

test("hydrates persisted ACP control updates from event types", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-control-updates",
          status: "completed",
          after_change: "ch_control_updates",
          ended_at: 6
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "current_mode_update",
            created_at: 1,
            payload: {
              current_mode_id: "code"
            }
          },
          {
            event_type: "available_commands_update",
            created_at: 2,
            payload: {
              command_names: ["/compact", "/review"]
            }
          },
          {
            event_type: "config_option_update",
            created_at: 3,
            payload: {
              config_options: [
                {
                  id: "model",
                  name: "Model",
                  type: "select"
                }
              ]
            }
          },
          {
            event_type: "session_info_update",
            created_at: 4,
            payload: {
              title: "Recovered session",
              updated_at: "2026-06-27T01:00:00.000Z"
            }
          },
          {
            event_type: "usage_update",
            created_at: 5,
            payload: {
              used: 120,
              size: 200,
              cost: {
                usd: 0.01
              }
            }
          }
        ],
        checkpoint: "ch_control_updates"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const mode = nodes.find((node) => node.kind === "mode");
  const commands = nodes.find((node) => node.kind === "commands");
  const config = nodes.find((node) => node.kind === "config");
  const session = nodes.find((node) => node.kind === "session");
  const usage = nodes.find((node) => node.kind === "usage");

  assert.equal(nodes.some((node) => node.kind === "unknown"), false);
  assert.equal(mode?.kind, "mode");
  assert.equal(mode?.modeId, "code");
  assert.equal(commands?.kind, "commands");
  assert.deepEqual(commands?.availableCommands.map((command) => command.name), ["/compact", "/review"]);
  assert.equal(config?.kind, "config");
  assert.equal(config?.configOptions[0]?.id, "model");
  assert.equal(session?.kind, "session");
  assert.equal(session?.title, "Recovered session");
  assert.equal(session?.sessionUpdatedAt, "2026-06-27T01:00:00.000Z");
  assert.equal(usage?.kind, "usage");
  assert.equal(usage?.used, 120);
  assert.equal(usage?.size, 200);
});

test("hydrates persisted ACP message text from delta aliases", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-delta-message-aliases",
          status: "completed",
          after_change: "ch_delta_message_aliases",
          ended_at: 4
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "agent_message_chunk",
            created_at: 1,
            message_id: "msg-delta-string",
            payload: {
              delta: "Rendered from a delta string."
            }
          },
          {
            event_type: "agent_message_chunk",
            created_at: 2,
            message_id: "msg-delta-object",
            payload: {
              content_delta: {
                type: "text",
                value: "Rendered from a delta object."
              }
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-added-delta",
            created_at: 3,
            payload: {
              role: "assistant",
              contentDelta: "Rendered from a message_added delta."
            }
          }
        ],
        checkpoint: "ch_delta_message_aliases"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:`
    )),
    [
      "message:assistant:msg-delta-string:Rendered from a delta string.",
      "message:assistant:msg-delta-object:Rendered from a delta object.",
      "message:assistant:msg-added-delta:Rendered from a message_added delta.",
      "checkpoint:"
    ]
  );
  assert.deepEqual(
    nodes
      .filter((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message")
      .map((node) => `${node.status}:${node.streaming}`),
    ["completed:false", "completed:false", "completed:false"]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4]);
});

test("falls back to persisted ACP message aliases when canonical content is empty", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-empty-content-alias",
          status: "completed",
          after_change: "ch_empty_content_alias",
          ended_at: 3
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "agent_message_chunk",
            created_at: 1,
            payload: {
              message_id: "msg-empty-content-alias",
              content: "",
              contentDelta: "Rendered from the fallback alias."
            }
          }
        ],
        checkpoint: "ch_empty_content_alias"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const message = nodes.find((node) => node.kind === "message");

  assert.equal(message?.kind, "message");
  if (message?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  assert.equal(message.acpMessageId, "msg-empty-content-alias");
  assert.equal(message.text, "Rendered from the fallback alias.");
});

test("hydrates persisted ACP message chunks from nested message payloads", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-nested-message-chunks",
          status: "completed",
          after_change: "ch_nested_message_chunks",
          ended_at: 3
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "agent_message_chunk",
            created_at: 1,
            payload: {
              message: {
                id: "msg-nested-chunk",
                role: "assistant",
                content: [
                  {
                    type: "text",
                    value: "Nested assistant chunk rendered."
                  }
                ]
              }
            }
          },
          {
            event_type: "agent_thought_chunk",
            created_at: 2,
            payload: {
              message: {
                message_id: "thought-nested-chunk",
                content: {
                  type: "text",
                  content: "Nested thought chunk rendered."
                }
              }
            }
          }
        ],
        checkpoint: "ch_nested_message_chunks"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.acpMessageId}:${node.text}`
        : node.kind === "thought"
          ? `${node.kind}:${node.acpMessageId}:${node.content.map((block) => block.type === "text" ? block.text : block.type).join("")}`
          : `${node.kind}:`
    )),
    [
      "message:assistant:msg-nested-chunk:Nested assistant chunk rendered.",
      "thought:thought-nested-chunk:Nested thought chunk rendered.",
      "checkpoint:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3]);
});

test("hydrates persisted ACP message chunk content arrays as structured blocks", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-message-content-array",
          status: "completed",
          after_change: "ch_message_content_array",
          ended_at: 2
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "agent_message_chunk",
            created_at: 1,
            payload: {
              message_id: "msg-content-array",
              content: [
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
              ]
            }
          }
        ],
        checkpoint: "ch_message_content_array"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const message = nodes.find((node) => node.kind === "message");

  assert.equal(message?.kind, "message");
  assert.equal(message?.text, "Rendered with context.Context file (README.md)");
  assert.deepEqual(message?.content.map((block) => block.type), ["text", "resource_link"]);
});

test("hydrates persisted ACP message chunk content arrays with untyped text aliases", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-untyped-content-array-aliases",
          status: "completed",
          after_change: "ch_untyped_content_array_aliases",
          ended_at: 3
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "agent_message_chunk",
            created_at: 1,
            payload: {
              message_id: "msg-untyped-content-array-aliases",
              content: [
                { content: "Rendered from untyped content. " },
                { value: "Rendered from untyped value." }
              ]
            }
          }
        ],
        checkpoint: "ch_untyped_content_array_aliases"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);
  const message = nodes.find((node): node is Extract<RenderNode, { kind: "message" }> => node.kind === "message");

  assert.equal(message?.acpMessageId, "msg-untyped-content-array-aliases");
  assert.equal(message?.text, "Rendered from untyped content. Rendered from untyped value.");
  assert.deepEqual(message?.content, [
    { type: "text", text: "Rendered from untyped content. " },
    { type: "text", text: "Rendered from untyped value." }
  ]);
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2]);
});

test("hydrates message-added payloads when message rows are missing", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-message-added-payloads",
          status: "completed",
          after_change: "ch_message_added_payloads",
          ended_at: 4
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "message_added",
            message_id: "msg-event-user",
            created_at: 1,
            payload: {
              role: "user",
              body: "Render the user payload"
            }
          },
          {
            event_type: "tool_call",
            created_at: 2,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-message-added",
              title: "Inspect payloads",
              kind: "read",
              status: "completed"
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-event-assistant",
            created_at: 3,
            payload: {
              role: "assistant",
              content: [
                {
                  type: "text",
                  content: "Rendered from the event payload."
                }
              ]
            }
          }
        ],
        checkpoint: "ch_message_added_payloads"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:${node.kind === "tool" ? node.title : ""}`
    )),
    [
      "message:user:msg-event-user:Render the user payload",
      "tool:Inspect payloads",
      "message:assistant:msg-event-assistant:Rendered from the event payload.",
      "checkpoint:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4]);
});

test("hydrates nested message-added payloads when message rows are missing", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-nested-message-added-payloads",
          status: "completed",
          after_change: "ch_nested_message_added_payloads",
          ended_at: 3
        },
        messages: [],
        tool_summaries: [],
        events: [
          {
            event_type: "message_added",
            created_at: 1,
            payload: {
              message: {
                message_id: "msg-nested-user",
                role: "user",
                text: "Render the nested user payload"
              }
            }
          },
          {
            event_type: "message_added",
            created_at: 2,
            payload: {
              message: {
                id: "msg-nested-assistant",
                role: "assistant",
                content: {
                  type: "text",
                  value: "Render the nested assistant payload"
                }
              }
            }
          }
        ],
        checkpoint: "ch_nested_message_added_payloads"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:`
    )),
    [
      "message:user:msg-nested-user:Render the nested user payload",
      "message:assistant:msg-nested-assistant:Render the nested assistant payload",
      "checkpoint:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3]);
});

test("uses nested message-added ids to match existing message rows", () => {
  const eventView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-nested-message-added-row-match",
          status: "completed",
          after_change: "ch_nested_message_added_row_match",
          ended_at: 3
        },
        messages: [
          {
            message_id: "msg-nested-row-user",
            role: "user",
            body: "Use the canonical row for the user",
            created_at: 1
          },
          {
            message_id: "msg-nested-row-assistant",
            role: "assistant",
            body: "Use the canonical row for the assistant",
            created_at: 2
          }
        ],
        tool_summaries: [],
        events: [
          {
            event_type: "message_added",
            created_at: 1,
            payload: {
              message: {
                message_id: "msg-nested-row-user",
                role: "user",
                text: "Stale nested user payload"
              }
            }
          },
          {
            event_type: "message_added",
            created_at: 2,
            payload: {
              message: {
                id: "msg-nested-row-assistant",
                role: "assistant",
                text: "Stale nested assistant payload"
              }
            }
          }
        ],
        checkpoint: "ch_nested_message_added_row_match"
      }
    ]
  };

  const nodes = hydrateTaskView(eventView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.kind}:`
    )),
    [
      "message:user:msg-nested-row-user:Use the canonical row for the user",
      "message:assistant:msg-nested-row-assistant:Use the canonical row for the assistant",
      "checkpoint:"
    ]
  );
  assert.equal(nodes.filter((node) => node.kind === "message").length, 2);
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3]);
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

test("weaves unplaced hydrated messages into partial message-added event timelines", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-partial-message-events",
          status: "completed",
          after_change: "ch_partial_message_events",
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
            event_type: "tool_call",
            created_at: 3,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-read",
              title: "Read README.md",
              kind: "read",
              status: "completed"
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-after",
            created_at: 5
          }
        ],
        checkpoint: "ch_partial_message_events"
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
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5]);
});

test("hydrates reopened ACP turns by recorded event time when events are returned out of order", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-out-of-order-events",
          status: "completed",
          after_change: "ch_out_of_order_events",
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
            message_id: "msg-after",
            created_at: 5
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
            message_id: "msg-user",
            created_at: 1
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
            event_type: "message_added",
            message_id: "msg-before",
            created_at: 2
          }
        ],
        checkpoint: "ch_out_of_order_events"
      }
    ]
  };

  const nodes = hydrateTaskView(orderedView);

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.text}`
        : `${node.kind}:${node.kind === "tool" ? `${node.toolCallId}:${node.toolStatus}` : ""}`
    )),
    [
      "message:user:Please inspect README",
      "message:assistant:I will read it first.",
      "tool:tool-read:completed",
      "message:assistant:The read is done.",
      "checkpoint:"
    ]
  );
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5]);
});

test("hydrates reopened ACP turns by ISO string event time", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-string-timestamps",
          status: "completed",
          after_change: "ch_string_timestamps",
          ended_at: "2026-06-27T00:00:06.000Z"
        },
        messages: [
          {
            message_id: "msg-user",
            role: "user",
            body: "Please inspect README",
            created_at: "2026-06-27T00:00:01.000Z"
          },
          {
            message_id: "msg-before",
            role: "assistant",
            body: "I will read it first.",
            created_at: "2026-06-27T00:00:02.000Z"
          },
          {
            message_id: "msg-after",
            role: "assistant",
            body: "The read is done.",
            created_at: "2026-06-27T00:00:05.000Z"
          }
        ],
        events: [
          {
            event_type: "message_added",
            message_id: "msg-after",
            created_at: "2026-06-27T00:00:05.000Z"
          },
          {
            event_type: "tool_call_update",
            created_at: "2026-06-27T00:00:04.000Z",
            payload: {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool-read",
              status: "completed"
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-user",
            created_at: "2026-06-27T00:00:01.000Z"
          },
          {
            event_type: "tool_call",
            created_at: "2026-06-27T00:00:03.000Z",
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "tool-read",
              title: "Read README.md",
              kind: "read",
              status: "pending"
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-before",
            created_at: "2026-06-27T00:00:02.000Z"
          }
        ],
        checkpoint: "ch_string_timestamps"
      }
    ]
  };

  const nodes = hydrateTaskView(orderedView);
  const tool = nodes.find((node) => node.kind === "tool");

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.text}`
        : `${node.kind}:${node.kind === "tool" ? `${node.toolCallId}:${node.toolStatus}` : ""}`
    )),
    [
      "message:user:Please inspect README",
      "message:assistant:I will read it first.",
      "tool:tool-read:completed",
      "message:assistant:The read is done.",
      "checkpoint:"
    ]
  );
  assert.equal(nodes[0]?.createdAt, "2026-06-27T00:00:01.000Z");
  assert.equal(tool?.createdAt, "2026-06-27T00:00:03.000Z");
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5]);
});

test("hydrates reopened ACP turns by nested event payload time", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-nested-event-timestamps",
          status: "completed",
          after_change: "ch_nested_event_timestamps",
          ended_at: "2026-06-27T00:00:06.000Z"
        },
        messages: [
          {
            message_id: "msg-user",
            role: "user",
            body: "Please inspect README",
            created_at: "2026-06-27T00:00:01.000Z"
          },
          {
            message_id: "msg-before",
            role: "assistant",
            body: "I will read it first.",
            created_at: "2026-06-27T00:00:02.000Z"
          },
          {
            message_id: "msg-after",
            role: "assistant",
            body: "The read is done.",
            created_at: "2026-06-27T00:00:05.000Z"
          }
        ],
        events: [
          {
            event_type: "message_added",
            payload: {
              message: {
                message_id: "msg-after",
                created_at: "2026-06-27T00:00:05.000Z"
              }
            }
          },
          {
            event_type: "acp_session_update",
            payload: {
              method: "session/update",
              params: {
                update: {
                  sessionUpdate: "tool_call_update",
                  toolCallId: "tool-nested-time",
                  status: "completed",
                  created_at: "2026-06-27T00:00:04.000Z"
                }
              }
            }
          },
          {
            event_type: "message_added",
            payload: {
              message: {
                message_id: "msg-user",
                created_at: "2026-06-27T00:00:01.000Z"
              }
            }
          },
          {
            event_type: "acp_session_update",
            payload: {
              method: "session/update",
              params: {
                update: {
                  sessionUpdate: "tool_call",
                  toolCallId: "tool-nested-time",
                  title: "Read README.md",
                  kind: "read",
                  status: "pending",
                  created_at: "2026-06-27T00:00:03.000Z"
                }
              }
            }
          },
          {
            event_type: "message_added",
            payload: {
              message: {
                message_id: "msg-before",
                created_at: "2026-06-27T00:00:02.000Z"
              }
            }
          }
        ],
        checkpoint: "ch_nested_event_timestamps"
      }
    ]
  };

  const nodes = hydrateTaskView(orderedView);
  const tool = nodes.find((node) => node.kind === "tool");

  assert.deepEqual(
    nodes.map((node) => (
      node.kind === "message"
        ? `${node.kind}:${node.role}:${node.text}`
        : `${node.kind}:${node.kind === "tool" ? `${node.toolCallId}:${node.toolStatus}` : ""}`
    )),
    [
      "message:user:Please inspect README",
      "message:assistant:I will read it first.",
      "tool:tool-nested-time:completed",
      "message:assistant:The read is done.",
      "checkpoint:"
    ]
  );
  assert.equal(tool?.createdAt, "2026-06-27T00:00:03.000Z");
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5]);
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

test("hydrates duplicate ACP message ids by event time when message records are out of order", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-duplicate-message-id-out-of-order",
          status: "completed",
          after_change: "ch_duplicate_message_id_out_of_order",
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
            body: "Second check is next.",
            created_at: 4
          },
          {
            message_id: "msg-shared",
            role: "assistant",
            body: "First check is next.",
            created_at: 2
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
        checkpoint: "ch_duplicate_message_id_out_of_order"
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
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("hydrates duplicate ACP message ids by camelCase event time", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-camel-case-event-time",
          status: "completed",
          after_change: "ch_camel_case_event_time",
          endedAt: 7
        },
        messages: [
          {
            message_id: "msg-user",
            role: "user",
            body: "Run both checks",
            createdAt: 1
          },
          {
            message_id: "msg-shared",
            role: "assistant",
            body: "Second check is next.",
            createdAt: 4
          },
          {
            message_id: "msg-shared",
            role: "assistant",
            body: "First check is next.",
            createdAt: 2
          }
        ],
        events: [
          {
            event_type: "message_added",
            message_id: "msg-shared",
            createdAt: 4
          },
          {
            event_type: "tool_call",
            createdAt: 5,
            payload: {
              sessionUpdate: "tool_call",
              toolCallId: "second-check",
              title: "Second check",
              kind: "execute",
              status: "completed"
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-user",
            createdAt: 1
          },
          {
            event_type: "tool_call",
            createdAt: 3,
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
            createdAt: 2
          }
        ],
        checkpoint: "ch_camel_case_event_time"
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
  assert.deepEqual(nodes.map((node) => node.timelineOrder), [1, 2, 3, 4, 5, 6]);
});

test("replaces the matching duplicated-message-id continuation from CrabDB hydration", () => {
  const current: RenderNode[] = [
    {
      id: "message:user:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-duplicate-message-id-merge",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "user",
      acpMessageId: "msg-user",
      content: [{ type: "text", text: "Run both checks" }],
      text: "Run both checks",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "message:assistant:msg-shared:1",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-duplicate-message-id-merge",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-shared",
      content: [{ type: "text", text: "First check is next." }],
      text: "First check is next.",
      streaming: false,
      timelineOrder: 2
    },
    {
      id: "tool:first-check",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-duplicate-message-id-merge",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      acpToolCallId: "first-check",
      toolCallId: "first-check",
      title: "First check",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: [],
      timelineOrder: 3
    },
    {
      id: "message:assistant:msg-shared:2",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-duplicate-message-id-merge",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-shared",
      content: [{ type: "text", text: "Second check is" }],
      text: "Second check is",
      streaming: false,
      timelineOrder: 4
    }
  ];
  const persisted: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-duplicate-message-id-merge",
          status: "completed",
          after_change: "ch_duplicate_message_id_merge"
        },
        messages: [
          { message_id: "msg-user", role: "user", body: "Run both checks", created_at: 1 },
          { message_id: "msg-shared", role: "assistant", body: "First check is next.", created_at: 2 },
          { message_id: "msg-shared", role: "assistant", body: "Second check is next.", created_at: 4 }
        ],
        events: [
          { event_type: "message_added", message_id: "msg-user", created_at: 1 },
          { event_type: "message_added", message_id: "msg-shared", created_at: 2 },
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
          { event_type: "message_added", message_id: "msg-shared", created_at: 4 }
        ],
        checkpoint: "ch_duplicate_message_id_merge"
      }
    ]
  };

  const merged = mergeHydratedNodes(hydrateTaskView(persisted), current);

  assert.deepEqual(
    merged.slice(0, 5).map((node) => (
      node.kind === "message"
        ? `${node.source}:${node.role}:${node.acpMessageId}:${node.text}`
        : `${node.source}:${node.kind}:${node.kind === "tool" ? node.toolCallId : ""}`
    )),
    [
      "crabdb:user:msg-user:Run both checks",
      "crabdb:assistant:msg-shared:First check is next.",
      "crabdb:tool:first-check",
      "crabdb:assistant:msg-shared:Second check is next.",
      "crabdb:checkpoint:"
    ]
  );
  assert.equal(
    merged.some((node) => node.kind === "message" && node.source === "acp-live" && node.text === "Second check is"),
    false
  );
  assert.deepEqual(merged.slice(0, 5).map((node) => node.timelineOrder), [1, 2, 3, 4, 5]);
});

test("keeps split live duplicated-message-id segments when CrabDB hydration is cumulative", () => {
  const current: RenderNode[] = [
    {
      id: "message:assistant:msg-shared:1",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-cumulative-message-id",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-shared",
      content: [{ type: "text", text: "Before the tool." }],
      text: "Before the tool.",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "tool:read-context",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-cumulative-message-id",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      acpToolCallId: "read-context",
      toolCallId: "read-context",
      title: "Read context",
      toolKind: "read",
      toolStatus: "completed",
      locations: [],
      content: [],
      timelineOrder: 2
    },
    {
      id: "message:assistant:msg-shared:2",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-cumulative-message-id",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-shared",
      content: [{ type: "text", text: "After the tool." }],
      text: "After the tool.",
      streaming: false,
      timelineOrder: 3
    }
  ];
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-cumulative-message-id:msg-shared",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-cumulative-message-id",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-shared",
      content: [{ type: "text", text: "Before the tool. After the tool." }],
      text: "Before the tool. After the tool.",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "tool:read-context",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-cumulative-message-id",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      acpToolCallId: "read-context",
      toolCallId: "read-context",
      title: "Read context",
      toolKind: "read",
      toolStatus: "completed",
      locations: [],
      content: [],
      timelineOrder: 2
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.slice(0, 3).map((node) => (
      node.kind === "message"
        ? `${node.source}:${node.text}`
        : `${node.source}:${node.kind}:${node.kind === "tool" ? node.toolCallId : ""}`
    )),
    [
      "acp-live:Before the tool.",
      "crabdb:tool:read-context",
      "acp-live:After the tool."
    ]
  );
  assert.equal(
    merged.some((node) => node.kind === "message" && node.source === "crabdb" && node.text === "Before the tool. After the tool."),
    false
  );
  assert.deepEqual(merged.slice(0, 3).map((node) => node.timelineOrder), [1, 2, 3]);
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

test("keeps missing earlier hydrated messages before matched live messages", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-partial-anchor:user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-partial-anchor",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      acpMessageId: "msg-user",
      content: [{ type: "text", text: "Explain the extension" }],
      text: "Explain the extension",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "crabdb-message:turn-partial-anchor:assistant",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-partial-anchor",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-assistant",
      content: [{ type: "text", text: "It renders ACP updates." }],
      text: "It renders ACP updates.",
      streaming: false,
      timelineOrder: 2
    }
  ];
  const hydratedAssistant = hydrated[1];
  assert.equal(hydratedAssistant?.kind, "message");
  if (hydratedAssistant?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  const assistantMessage: Extract<RenderNode, { kind: "message" }> = hydratedAssistant;
  const current: RenderNode[] = [
    {
      ...assistantMessage,
      id: "message:assistant:msg-assistant",
      source: "acp-live",
      timelineOrder: 1
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (node.kind === "message" ? `${node.source}:${node.role}:${node.text}` : `${node.source}:${node.kind}`)),
    [
      "crabdb:user:Explain the extension",
      "crabdb:assistant:It renders ACP updates."
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);
});

test("reconciles completed live messages with hydrated messages when the live turn id is missing", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-missing-live-turn:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-missing-live-turn",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      acpMessageId: "msg-user",
      content: [{ type: "text", text: "Explain rendering" }],
      text: "Explain rendering",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "crabdb-message:turn-missing-live-turn:msg-assistant",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-missing-live-turn",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-assistant",
      content: [{ type: "text", text: "Rendering is stable." }],
      text: "Rendering is stable.",
      streaming: false,
      timelineOrder: 2
    }
  ];
  const hydratedAssistant = hydrated[1];
  assert.equal(hydratedAssistant?.kind, "message");
  if (hydratedAssistant?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  const assistantMessage: Extract<RenderNode, { kind: "message" }> = hydratedAssistant;
  const current: RenderNode[] = [
    {
      ...assistantMessage,
      id: "message:assistant:msg-assistant",
      source: "acp-live",
      turnId: undefined,
      timelineOrder: 1
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (node.kind === "message" ? `${node.source}:${node.role}:${node.text}` : `${node.source}:${node.kind}`)),
    [
      "crabdb:user:Explain rendering",
      "crabdb:assistant:Rendering is stable."
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);
});

test("reconciles missing-turn live messages when hydrated text extends live text", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-missing-live-turn-prefix:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-missing-live-turn-prefix",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      acpMessageId: "msg-user",
      content: [{ type: "text", text: "Explain final rendering" }],
      text: "Explain final rendering",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "crabdb-message:turn-missing-live-turn-prefix:msg-assistant",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-missing-live-turn-prefix",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-assistant",
      content: [{ type: "text", text: "Rendering is stable after completion." }],
      text: "Rendering is stable after completion.",
      streaming: false,
      timelineOrder: 2
    }
  ];
  const hydratedAssistant = hydrated[1];
  assert.equal(hydratedAssistant?.kind, "message");
  if (hydratedAssistant?.kind !== "message") {
    throw new Error("expected hydrated assistant message");
  }
  const assistantMessage: Extract<RenderNode, { kind: "message" }> = hydratedAssistant;
  const current: RenderNode[] = [
    {
      ...assistantMessage,
      id: "message:assistant:msg-assistant",
      source: "acp-live",
      turnId: undefined,
      content: [{ type: "text", text: "Rendering is stable" }],
      text: "Rendering is stable",
      timelineOrder: 1
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (node.kind === "message" ? `${node.source}:${node.role}:${node.text}` : `${node.source}:${node.kind}`)),
    [
      "crabdb:user:Explain final rendering",
      "crabdb:assistant:Rendering is stable after completion."
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);
});

test("reconciles completed live turn with hydrated turn when the user prompt matches", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-crabdb:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      content: [{ type: "text", text: "What I have in current repo" }],
      text: "What I have in current repo",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "tool:list-files",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      acpToolCallId: "list-files",
      toolCallId: "list-files",
      title: "Listed files",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: [],
      timelineOrder: 2
    },
    {
      id: "crabdb-message:turn-crabdb:msg-assistant",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-assistant",
      content: [{ type: "text", text: "Here is the summary." }],
      text: "Here is the summary.",
      streaming: false,
      timelineOrder: 3
    }
  ];
  const hydratedUser = hydrated[0] as Extract<RenderNode, { kind: "message" }>;
  const hydratedTool = hydrated[1] as Extract<RenderNode, { kind: "tool" }>;
  const hydratedAssistant = hydrated[2] as Extract<RenderNode, { kind: "message" }>;
  const current: RenderNode[] = [
    {
      ...hydratedUser,
      id: "message:user:turn-live",
      turnId: "turn-live",
      source: "acp-live",
      timelineOrder: 1
    },
    {
      ...hydratedTool,
      id: "tool:list-files:live",
      turnId: "turn-live",
      source: "acp-live",
      timelineOrder: 2
    },
    {
      ...hydratedAssistant,
      id: "message:assistant:msg-assistant",
      turnId: "turn-live",
      source: "acp-live",
      content: [{ type: "text", text: "Here is the summary" }],
      text: "Here is the summary",
      timelineOrder: 3
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (
      node.kind === "message"
        ? `${node.source}:${node.turnId}:${node.role}:${node.text}`
        : `${node.source}:${node.turnId}:${node.kind === "tool" ? node.toolCallId : ""}`
    )),
    [
      "crabdb:turn-crabdb:user:What I have in current repo",
      "crabdb:turn-crabdb:list-files",
      "crabdb:turn-crabdb:assistant:Here is the summary."
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2, 3]);
});

test("drops checkpoint-pending live completion when aliased hydrated turn has a checkpoint", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-crabdb-checkpoint:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb-checkpoint",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      content: [{ type: "text", text: "what I have in current repo and how many lines of code ?" }],
      text: "what I have in current repo and how many lines of code ?",
      streaming: false,
      updatedAt: "2026-06-27T00:10:00.000Z",
      timelineOrder: 1
    },
    {
      id: "tool:count-lines",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb-checkpoint",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      acpToolCallId: "count-lines",
      toolCallId: "count-lines",
      title: "Ran line count",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: [],
      updatedAt: "2026-06-27T00:10:01.000Z",
      timelineOrder: 2
    },
    {
      id: "crabdb-message:turn-crabdb-checkpoint:msg-assistant",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb-checkpoint",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-assistant-checkpoint",
      content: [{ type: "text", text: "So the repo has about 5,357 lines of code." }],
      text: "So the repo has about 5,357 lines of code.",
      streaming: false,
      updatedAt: "2026-06-27T00:10:02.000Z",
      timelineOrder: 3
    },
    {
      id: "crabdb-checkpoint:turn-crabdb-checkpoint",
      kind: "checkpoint",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb-checkpoint",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      checkpointId: "ch_done",
      label: "Checkpoint ch_done",
      updatedAt: "2026-06-27T00:10:03.000Z",
      timelineOrder: 4
    }
  ];
  const hydratedUser = hydrated[0] as Extract<RenderNode, { kind: "message" }>;
  const hydratedTool = hydrated[1] as Extract<RenderNode, { kind: "tool" }>;
  const hydratedAssistant = hydrated[2] as Extract<RenderNode, { kind: "message" }>;
  const current: RenderNode[] = [
    {
      ...hydratedUser,
      id: "message:user:turn-live-checkpoint",
      turnId: "turn-live-checkpoint",
      source: "acp-live",
      timelineOrder: 1
    },
    {
      ...hydratedTool,
      id: "tool:count-lines:live",
      turnId: "turn-live-checkpoint",
      source: "acp-live",
      timelineOrder: 2
    },
    {
      ...hydratedAssistant,
      id: "message:assistant:msg-assistant-checkpoint",
      turnId: "turn-live-checkpoint",
      source: "acp-live",
      content: [{ type: "text", text: "So the repo has about 5,357 lines of code" }],
      text: "So the repo has about 5,357 lines of code",
      timelineOrder: 3
    },
    {
      id: "completion:turn-live-checkpoint",
      kind: "completion",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-checkpoint",
      provider: "provider",
      source: "acp-live",
      status: "pending",
      stopReason: "end_turn",
      label: "Turn complete; checkpoint pending",
      checkpointPending: true,
      updatedAt: "2026-06-27T00:10:04.000Z",
      timelineOrder: 4
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (
      node.kind === "message"
        ? `${node.source}:${node.turnId}:${node.role}:${node.text}`
        : `${node.source}:${node.turnId}:${node.kind}:${node.status}`
    )),
    [
      "crabdb:turn-crabdb-checkpoint:user:what I have in current repo and how many lines of code ?",
      "crabdb:turn-crabdb-checkpoint:tool:completed",
      "crabdb:turn-crabdb-checkpoint:assistant:So the repo has about 5,357 lines of code.",
      "crabdb:turn-crabdb-checkpoint:checkpoint:completed"
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2, 3, 4]);
});

test("reconciles hydrated transcript while keeping only checkpoint-pending completion", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-crabdb-pending:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb-pending",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      content: [{ type: "text", text: "what I have in current repo and how many lines of code ?" }],
      text: "what I have in current repo and how many lines of code ?",
      streaming: false,
      updatedAt: "2026-06-27T00:10:00.000Z",
      timelineOrder: 1
    },
    {
      id: "tool:count-lines-pending",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb-pending",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      acpToolCallId: "count-lines",
      toolCallId: "count-lines",
      title: "Ran line count",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: [],
      updatedAt: "2026-06-27T00:10:01.000Z",
      timelineOrder: 2
    },
    {
      id: "crabdb-message:turn-crabdb-pending:msg-assistant",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-crabdb-pending",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-assistant-pending",
      content: [{ type: "text", text: "So the repo has about 5,357 lines of code." }],
      text: "So the repo has about 5,357 lines of code.",
      streaming: false,
      updatedAt: "2026-06-27T00:10:02.000Z",
      timelineOrder: 3
    }
  ];
  const hydratedUser = hydrated[0] as Extract<RenderNode, { kind: "message" }>;
  const hydratedTool = hydrated[1] as Extract<RenderNode, { kind: "tool" }>;
  const hydratedAssistant = hydrated[2] as Extract<RenderNode, { kind: "message" }>;
  const current: RenderNode[] = [
    {
      ...hydratedUser,
      id: "message:user:turn-live-pending",
      turnId: "turn-live-pending",
      source: "acp-live",
      timelineOrder: 1
    },
    {
      ...hydratedTool,
      id: "tool:count-lines-pending:live",
      turnId: "turn-live-pending",
      source: "acp-live",
      timelineOrder: 2
    },
    {
      ...hydratedAssistant,
      id: "message:assistant:msg-assistant-pending",
      turnId: "turn-live-pending",
      source: "acp-live",
      content: [{ type: "text", text: "So the repo has about 5,357 lines of code" }],
      text: "So the repo has about 5,357 lines of code",
      timelineOrder: 3
    },
    {
      id: "completion:turn-live-pending",
      kind: "completion",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-pending",
      provider: "provider",
      source: "acp-live",
      status: "pending",
      stopReason: "end_turn",
      label: "Turn complete; checkpoint pending",
      checkpointPending: true,
      updatedAt: "2026-06-27T00:10:04.000Z",
      timelineOrder: 4
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (
      node.kind === "message"
        ? `${node.source}:${node.turnId}:${node.role}:${node.text}`
        : `${node.source}:${node.turnId}:${node.kind}:${node.status}`
    )),
    [
      "crabdb:turn-crabdb-pending:user:what I have in current repo and how many lines of code ?",
      "crabdb:turn-crabdb-pending:tool:completed",
      "crabdb:turn-crabdb-pending:assistant:So the repo has about 5,357 lines of code.",
      "acp-live:turn-crabdb-pending:completion:pending"
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2, 3, 4]);

  const settled = mergeHydratedNodes(
    [
      ...hydrated,
      {
        id: "crabdb-checkpoint:turn-crabdb-pending",
        kind: "checkpoint",
        taskId: "task-1",
        lane: "lane-1",
        turnId: "turn-crabdb-pending",
        provider: "provider",
        source: "crabdb",
        status: "completed",
        checkpointId: "ch_done",
        label: "Checkpoint ch_done",
        updatedAt: "2026-06-27T00:10:05.000Z",
        timelineOrder: 4
      }
    ],
    merged
  );
  assert.equal(settled.some((node) => node.kind === "completion"), false);
  assert.deepEqual(
    settled.map((node) => `${node.source}:${node.turnId}:${node.kind}:${node.status}`),
    [
      "crabdb:turn-crabdb-pending:message:completed",
      "crabdb:turn-crabdb-pending:tool:completed",
      "crabdb:turn-crabdb-pending:message:completed",
      "crabdb:turn-crabdb-pending:checkpoint:completed"
    ]
  );
});

test("reconciles newly claimed CrabDB task with live new-task prompt scope", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-crabdb-new-task:msg-user",
      kind: "message",
      taskId: "task-claimed",
      lane: "lane-claimed",
      turnId: "turn-crabdb-new-task",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      content: [{ type: "text", text: "what I have in current repo and how many lines of code ?" }],
      text: "what I have in current repo and how many lines of code ?",
      streaming: false,
      updatedAt: "2026-06-27T00:10:00.000Z",
      timelineOrder: 1
    },
    {
      id: "tool:count-lines-new-task",
      kind: "tool",
      taskId: "task-claimed",
      lane: "lane-claimed",
      turnId: "turn-crabdb-new-task",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      acpToolCallId: "count-lines",
      toolCallId: "count-lines",
      title: "Ran line count",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: [],
      updatedAt: "2026-06-27T00:10:01.000Z",
      timelineOrder: 2
    },
    {
      id: "crabdb-message:turn-crabdb-new-task:msg-assistant",
      kind: "message",
      taskId: "task-claimed",
      lane: "lane-claimed",
      turnId: "turn-crabdb-new-task",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      acpMessageId: "msg-assistant-new-task",
      content: [{ type: "text", text: "So the repo has about 5,357 lines of code." }],
      text: "So the repo has about 5,357 lines of code.",
      streaming: false,
      updatedAt: "2026-06-27T00:10:02.000Z",
      timelineOrder: 3
    }
  ];
  const hydratedUser = hydrated[0] as Extract<RenderNode, { kind: "message" }>;
  const hydratedTool = hydrated[1] as Extract<RenderNode, { kind: "tool" }>;
  const hydratedAssistant = hydrated[2] as Extract<RenderNode, { kind: "message" }>;
  const current: RenderNode[] = [
    {
      ...hydratedUser,
      id: "message:user:turn-live-new-task",
      taskId: "new-task",
      lane: "new-task",
      turnId: "turn-live-new-task",
      source: "acp-live",
      timelineOrder: 1
    },
    {
      ...hydratedTool,
      id: "tool:count-lines-new-task:live",
      taskId: "new-task",
      lane: "new-task",
      turnId: "turn-live-new-task",
      source: "acp-live",
      timelineOrder: 2
    },
    {
      ...hydratedAssistant,
      id: "message:assistant:msg-assistant-new-task",
      taskId: "new-task",
      lane: "new-task",
      turnId: "turn-live-new-task",
      source: "acp-live",
      content: [{ type: "text", text: "So the repo has about 5,357 lines of code" }],
      text: "So the repo has about 5,357 lines of code",
      timelineOrder: 3
    },
    {
      id: "completion:turn-live-new-task",
      kind: "completion",
      taskId: "new-task",
      lane: "new-task",
      turnId: "turn-live-new-task",
      provider: "provider",
      source: "acp-live",
      status: "pending",
      stopReason: "end_turn",
      label: "Turn complete; checkpoint pending",
      checkpointPending: true,
      updatedAt: "2026-06-27T00:10:04.000Z",
      timelineOrder: 4
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (
      node.kind === "message"
        ? `${node.source}:${node.taskId}:${node.lane}:${node.turnId}:${node.role}:${node.text}`
        : `${node.source}:${node.taskId}:${node.lane}:${node.turnId}:${node.kind}:${node.status}`
    )),
    [
      "crabdb:task-claimed:lane-claimed:turn-crabdb-new-task:user:what I have in current repo and how many lines of code ?",
      "crabdb:task-claimed:lane-claimed:turn-crabdb-new-task:tool:completed",
      "crabdb:task-claimed:lane-claimed:turn-crabdb-new-task:assistant:So the repo has about 5,357 lines of code.",
      "acp-live:task-claimed:lane-claimed:turn-crabdb-new-task:completion:pending"
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2, 3, 4]);
});

test("does not reconcile a checkpoint-pending repeat prompt with an older hydrated turn", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-old-repeat:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-old-repeat",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      content: [{ type: "text", text: "What I have in current repo" }],
      text: "What I have in current repo",
      streaming: false,
      createdAt: "2026-06-27T00:00:00.000Z",
      timelineOrder: 1
    },
    {
      id: "crabdb-message:turn-old-repeat:msg-assistant",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-old-repeat",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "assistant",
      content: [{ type: "text", text: "Old summary." }],
      text: "Old summary.",
      streaming: false,
      createdAt: "2026-06-27T00:00:01.000Z",
      timelineOrder: 2
    }
  ];
  const current: RenderNode[] = [
    {
      id: "message:user:turn-live-repeat",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-repeat",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "user",
      content: [{ type: "text", text: "What I have in current repo" }],
      text: "What I have in current repo",
      streaming: false,
      updatedAt: "2026-06-27T00:10:00.000Z",
      timelineOrder: 3
    },
    {
      id: "message:assistant:turn-live-repeat",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-repeat",
      provider: "provider",
      source: "acp-live",
      status: "completed",
      role: "assistant",
      content: [{ type: "text", text: "New summary." }],
      text: "New summary.",
      streaming: false,
      updatedAt: "2026-06-27T00:10:01.000Z",
      timelineOrder: 4
    },
    {
      id: "completion:turn-live-repeat",
      kind: "completion",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-live-repeat",
      provider: "provider",
      source: "acp-live",
      status: "pending",
      stopReason: "end_turn",
      label: "Turn complete; checkpoint pending",
      checkpointPending: true,
      updatedAt: "2026-06-27T00:10:02.000Z",
      timelineOrder: 5
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (
      node.kind === "message"
        ? `${node.source}:${node.turnId}:${node.role}:${node.text}`
        : `${node.source}:${node.turnId}:${node.kind}:${node.status}`
    )),
    [
      "crabdb:turn-old-repeat:user:What I have in current repo",
      "crabdb:turn-old-repeat:assistant:Old summary.",
      "acp-live:turn-live-repeat:user:What I have in current repo",
      "acp-live:turn-live-repeat:assistant:New summary.",
      "acp-live:turn-live-repeat:completion:pending"
    ]
  );
});

test("reconciles completed live tools with hydrated tools when the live turn id is missing", () => {
  const hydrated: RenderNode[] = [
    {
      id: "crabdb-message:turn-missing-live-tool:msg-user",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-missing-live-tool",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      role: "user",
      acpMessageId: "msg-user",
      content: [{ type: "text", text: "Run status" }],
      text: "Run status",
      streaming: false,
      timelineOrder: 1
    },
    {
      id: "tool:missing-live-turn-tool",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-missing-live-tool",
      provider: "provider",
      source: "crabdb",
      status: "completed",
      acpToolCallId: "tool-status",
      toolCallId: "tool-status",
      title: "Run status",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: [
        {
          type: "terminal",
          terminalId: "term-status",
          command: "npm test",
          stdout: "ok\n"
        }
      ],
      timelineOrder: 2
    }
  ];
  const current: RenderNode[] = [
    {
      ...hydrated[1]!,
      source: "acp-live",
      turnId: undefined,
      timelineOrder: 1
    }
  ];

  const merged = mergeHydratedNodes(hydrated, current);

  assert.deepEqual(
    merged.map((node) => (node.kind === "message" ? `${node.source}:${node.role}:${node.text}` : `${node.source}:${node.kind}:${node.kind === "tool" ? node.title : ""}`)),
    [
      "crabdb:user:Run status",
      "crabdb:tool:Run status"
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);
});

test("keeps distinct same-tool terminal children when hydrating one terminal", () => {
  const hydratedTerminal: Extract<RenderNode, { kind: "terminal" }> = {
    id: "terminal:multi-terminal-tool:term-b",
    kind: "terminal",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-multi-terminal",
    provider: "provider",
    source: "crabdb",
    status: "completed",
    acpToolCallId: "multi-terminal-tool",
    terminalId: "term-b",
    command: "npm run b",
    terminalStatus: "completed",
    stdout: "bbbbbbbb\n",
    timelineOrder: 2
  };
  const liveA: Extract<RenderNode, { kind: "terminal" }> = {
    ...hydratedTerminal,
    id: "terminal:multi-terminal-tool:term-a",
    source: "acp-live",
    terminalId: "term-a",
    command: "npm run a",
    stdout: "a\n",
    timelineOrder: 1
  };
  const liveB: Extract<RenderNode, { kind: "terminal" }> = {
    ...hydratedTerminal,
    source: "acp-live",
    stdout: "b\n",
    timelineOrder: 2
  };

  const merged = mergeHydratedNodes([hydratedTerminal], [liveA, liveB]);

  assert.deepEqual(
    merged.map((node) => node.kind === "terminal" ? `${node.source}:${node.terminalId}:${node.stdout}` : `${node.source}:${node.kind}`),
    [
      "acp-live:term-a:a\n",
      "crabdb:term-b:bbbbbbbb\n"
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);
});

test("keeps distinct same-tool diff children when hydrating one diff", () => {
  const hydratedDiff: Extract<RenderNode, { kind: "diff" }> = {
    id: "diff:multi-diff-tool:src/b.ts",
    kind: "diff",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-multi-diff",
    provider: "provider",
    source: "crabdb",
    status: "completed",
    acpToolCallId: "multi-diff-tool",
    path: "src/b.ts",
    oldText: "old b",
    newText: "new b with hydrated details",
    timelineOrder: 2
  };
  const liveA: Extract<RenderNode, { kind: "diff" }> = {
    ...hydratedDiff,
    id: "diff:multi-diff-tool:src/a.ts",
    source: "acp-live",
    path: "src/a.ts",
    oldText: "old a",
    newText: "new a",
    timelineOrder: 1
  };
  const liveB: Extract<RenderNode, { kind: "diff" }> = {
    ...hydratedDiff,
    source: "acp-live",
    newText: "new b",
    timelineOrder: 2
  };

  const merged = mergeHydratedNodes([hydratedDiff], [liveA, liveB]);

  assert.deepEqual(
    merged.map((node) => node.kind === "diff" ? `${node.source}:${node.path}:${node.newText}` : `${node.source}:${node.kind}`),
    [
      "acp-live:src/a.ts:new a",
      "crabdb:src/b.ts:new b with hydrated details"
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);
});

test("keeps distinct same-request approval children when hydrating one approval", () => {
  const approval = (
    toolCallId: string,
    source: RenderNode["source"],
    status: RenderNode["status"],
    title: string,
    timelineOrder: number
  ): Extract<RenderNode, { kind: "approval" }> => {
    const tool: Extract<RenderNode, { kind: "tool" }> = {
      id: `tool:${toolCallId}`,
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-shared-approval",
      provider: "provider",
      source,
      status,
      acpToolCallId: toolCallId,
      toolCallId,
      title: `Tool ${toolCallId}`,
      toolKind: "execute",
      toolStatus: status,
      locations: [],
      content: [],
      timelineOrder
    };
    return {
      id: "approval:shared-request",
      kind: "approval",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-shared-approval",
      provider: "provider",
      source,
      status,
      acpToolCallId: toolCallId,
      requestId: "shared-request",
      title,
      tool,
      options: [{ optionId: "allow", label: "Allow" }],
      timelineOrder
    };
  };
  const hydratedApproval = approval(
    "tool-b",
    "crabdb",
    "completed",
    "Approve second command with persisted details",
    2
  );
  const liveA = approval("tool-a", "acp-live", "pending", "Approve first command", 1);
  const liveB = approval("tool-b", "acp-live", "pending", "Approve second command", 2);

  const merged = mergeHydratedNodes([hydratedApproval], [liveA, liveB]);

  assert.deepEqual(
    merged.map((node) => node.kind === "approval" ? `${node.source}:${node.tool.toolCallId}:${node.title}` : `${node.source}:${node.kind}`),
    [
      "acp-live:tool-a:Approve first command",
      "crabdb:tool-b:Approve second command with persisted details"
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);
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

test("keeps completed live transcript when hydration has no transcript yet", () => {
  const current: RenderNode[] = [
    {
      id: "message:user:turn-empty-hydration",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-empty-hydration",
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
      id: "message:assistant:turn-empty-hydration",
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-empty-hydration",
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

  const merged = mergeHydratedNodes([], current);

  assert.deepEqual(
    merged.map((node) => (node.kind === "message" ? `${node.source}:${node.role}:${node.text}` : `${node.source}:${node.kind}`)),
    [
      "acp-live:user:Summarize the repo",
      "acp-live:assistant:The repo contains a VS Code extension."
    ]
  );
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1, 2]);
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

test("keeps richer completed live assistant content when hydrated text is equivalent", () => {
  const hydrated: RenderNode = {
    id: "crabdb-message:turn-rich-content:msg-1",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-rich-content",
    source: "crabdb",
    status: "completed",
    role: "assistant",
    acpMessageId: "msg-1",
    content: [{ type: "text", text: "Rendered with context.Context file (README.md)" }],
    text: "Rendered with context.Context file (README.md)",
    streaming: false,
    timelineOrder: 1
  };
  const live: RenderNode = {
    ...hydrated,
    id: "message:assistant:msg-1",
    source: "acp-live",
    content: [
      { type: "text", text: "Rendered with context." },
      {
        type: "resource_link",
        uri: "file:///workspace/README.md",
        name: "README.md",
        title: "Context file"
      }
    ],
    timelineOrder: 1
  };

  const merged = mergeHydratedNodes([hydrated], [live]);

  const message = merged.find((node) => node.kind === "message");
  assert.equal(message?.kind, "message");
  assert.equal(message?.source, "acp-live");
  assert.deepEqual(message?.content.map((block) => block.type), ["text", "resource_link"]);
  assert.deepEqual(merged.map((node) => node.timelineOrder), [1]);
});

test("keeps richer completed live assistant content when hydrated text extends live text", () => {
  const hydrated: RenderNode = {
    id: "crabdb-message:turn-rich-content-prefix:msg-1",
    kind: "message",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-rich-content-prefix",
    source: "crabdb",
    status: "completed",
    role: "assistant",
    acpMessageId: "msg-1",
    content: [{ type: "text", text: "Rendered with context. Context file (README.md)" }],
    text: "Rendered with context. Context file (README.md)",
    streaming: false,
    timelineOrder: 1
  };
  const live: RenderNode = {
    ...hydrated,
    id: "message:assistant:msg-1",
    source: "acp-live",
    content: [
      { type: "text", text: "Rendered with context." },
      {
        type: "resource_link",
        uri: "file:///workspace/README.md",
        name: "README.md",
        title: "Context file"
      }
    ],
    text: "Rendered with context.",
    timelineOrder: 1
  };

  const merged = mergeHydratedNodes([hydrated], [live]);

  const message = merged.find((node) => node.kind === "message");
  assert.equal(message?.kind, "message");
  assert.equal(message?.source, "acp-live");
  assert.deepEqual(message?.content.map((block) => block.type), ["text", "resource_link"]);
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

test("keeps failed live tool lifecycle when hydrated tool content is otherwise equivalent", () => {
  const hydrated: RenderNode = {
    id: "tool:failed-tool",
    kind: "tool",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-failed-tool",
    source: "crabdb",
    status: "completed",
    acpToolCallId: "failed-tool",
    toolCallId: "failed-tool",
    title: "Run npm test",
    toolKind: "execute",
    toolStatus: "completed",
    locations: [],
    content: [
      {
        type: "terminal",
        terminalId: "term-1",
        command: "npm test",
        stdout: "failed\n",
        stderr: "boom\n"
      }
    ],
    timelineOrder: 1
  };
  const live: RenderNode = {
    ...hydrated,
    source: "acp-live",
    status: "failed",
    toolStatus: "failed",
    timelineOrder: 1
  };

  const merged = mergeHydratedNodes([hydrated], [live]);
  const tool = merged.find((node) => node.kind === "tool");

  assert.equal(tool?.kind, "tool");
  assert.equal(tool?.source, "acp-live");
  assert.equal(tool?.status, "failed");
  assert.equal(tool?.toolStatus, "failed");
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

test("infers persisted ACP plan events from event type and entries payload", () => {
  const orderedView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-plan-inferred",
          status: "completed",
          after_change: "ch_plan_inferred",
          ended_at: 5
        },
        messages: [
          {
            message_id: "msg-user-plan-inferred",
            role: "user",
            body: "Please make a plan",
            created_at: 1
          },
          {
            message_id: "msg-after-plan-inferred",
            role: "assistant",
            body: "Plan is ready.",
            created_at: 4
          }
        ],
        events: [
          {
            event_type: "message_added",
            message_id: "msg-user-plan-inferred",
            created_at: 1
          },
          {
            event_type: "plan_update",
            created_at: 2,
            payload: {
              entries: [
                {
                  title: "Inspect files",
                  status: "pending"
                }
              ]
            }
          },
          {
            event_type: "message_added",
            message_id: "msg-after-plan-inferred",
            created_at: 4
          }
        ],
        checkpoint: "ch_plan_inferred"
      }
    ]
  };

  const nodes = hydrateTaskView(orderedView);

  assert.deepEqual(
    nodes.map((node) => (node.kind === "message" ? `${node.role}:${node.text}` : node.kind)),
    [
      "user:Please make a plan",
      "plan",
      "assistant:Plan is ready.",
      "checkpoint"
    ]
  );
  const plan = nodes[1];
  assert.equal(plan?.kind, "plan");
  if (plan?.kind !== "plan") {
    throw new Error("expected inferred plan update");
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

test("drops checkpoint-pending completion placeholders after matching CrabDB checkpoint hydration", () => {
  const liveCompletion: RenderNode = {
    id: "completion:turn-1",
    kind: "completion",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
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

test("keeps checkpoint-pending completion placeholders for live turns missing from hydration", () => {
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
    checkpointPending: true,
    timelineOrder: 10
  };

  const merged = mergeHydratedNodes(hydrateTaskView(view), [liveCompletion]);
  const completion = merged.find((node) => node.id === liveCompletion.id);

  assert.equal(completion?.kind, "completion");
  assert.equal(completion?.status, "pending");
  assert.equal(completion?.turnId, "turn-live");
  assert.equal(completion?.kind === "completion" ? completion.checkpointPending : undefined, true);
});

test("keeps checkpoint-pending completion placeholders until the matching CrabDB checkpoint exists", () => {
  const noCheckpointView: TaskView = {
    ...view,
    turns: [
      {
        turn: {
          turn_id: "turn-1",
          status: "completed"
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
        tool_summaries: []
      }
    ]
  };
  const liveCompletion: RenderNode = {
    id: "completion:turn-1",
    kind: "completion",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    source: "acp-live",
    status: "pending",
    stopReason: "end_turn",
    label: "Turn complete; checkpoint pending",
    checkpointPending: true,
    timelineOrder: 10
  };

  const merged = mergeHydratedNodes(hydrateTaskView(noCheckpointView), [liveCompletion]);
  const completion = merged.find((node) => node.id === liveCompletion.id);

  assert.equal(completion?.kind, "completion");
  assert.equal(completion?.status, "pending");
  assert.equal(completion?.turnId, "turn-1");
  assert.equal(completion?.kind === "completion" ? completion.checkpointPending : undefined, true);
});

test("keeps failed live completion events after CrabDB hydration", () => {
  const liveCompletion: RenderNode = {
    id: "completion:turn-failed",
    kind: "completion",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-failed",
    source: "acp-live",
    status: "failed",
    stopReason: "max_tokens",
    label: "Stopped after reaching the token limit",
    checkpointPending: false,
    timelineOrder: 10
  };

  const merged = mergeHydratedNodes(hydrateTaskView(view), [liveCompletion]);
  const completion = merged.find((node) => node.id === liveCompletion.id);

  assert.equal(completion?.kind, "completion");
  assert.equal(completion?.status, "failed");
  assert.equal(completion?.stopReason, "max_tokens");
  assert.equal(completion?.checkpointPending, false);
});

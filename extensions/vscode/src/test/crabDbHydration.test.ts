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

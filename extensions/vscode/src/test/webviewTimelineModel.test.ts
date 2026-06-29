import assert from "node:assert/strict";
import test from "node:test";
import {
  buildToolActivitySummary,
  filterTimelineNodes,
  isTimelineFilter,
  sortTimelineNodes,
  transcriptTimelineNodes,
  timelineFilterCounts,
  timelineNodeBucket,
  timelineNodeSearchText,
  timelineSearchTokens
} from "../webview/timelineModel";
import type { RenderNode } from "../shared/renderModel";

const base = {
  taskId: "task-1",
  lane: "lane-1",
  source: "acp-live" as const,
  status: "completed" as const
};

const nodes: RenderNode[] = [
  {
    ...base,
    id: "message:1",
    kind: "message",
    role: "assistant",
    content: [{ type: "text", text: "Updated README" }],
    text: "Updated README",
    streaming: false
  },
  {
    ...base,
    id: "tool:1",
    kind: "tool",
    toolCallId: "tool-1",
    title: "Read package.json",
    toolKind: "read",
    toolStatus: "completed",
    locations: [{ path: "package.json" }],
    content: []
  },
  {
    ...base,
    id: "diff:1",
    kind: "diff",
    path: "README.md",
    oldText: "old",
    newText: "new"
  },
  {
    ...base,
    id: "approval:1",
    kind: "approval",
    requestId: "approval-1",
    title: "Run migration",
    tool: {
      ...base,
      id: "tool:approval",
      kind: "tool",
      toolCallId: "tool-approval",
      title: "Run migration",
      toolKind: "execute",
      toolStatus: "pending",
      locations: [{ path: "db/schema.sql", line: 7 }],
      content: []
    },
    options: [{ optionId: "allow", label: "Allow once", description: "Run database migration" }]
  },
  {
    ...base,
    id: "checkpoint:1",
    kind: "checkpoint",
    checkpointId: "ch_1",
    label: "Checkpoint saved"
  }
];

test("buckets timeline nodes into transcript filter groups", () => {
  assert.equal(timelineNodeBucket(nodes[0] as RenderNode), "chat");
  assert.equal(timelineNodeBucket(nodes[1] as RenderNode), "tools");
  assert.equal(timelineNodeBucket(nodes[2] as RenderNode), "diffs");
  assert.equal(timelineNodeBucket(nodes[3] as RenderNode), "approvals");
  assert.equal(timelineNodeBucket(nodes[4] as RenderNode), "events");
});

test("counts timeline filter groups", () => {
  assert.deepEqual(timelineFilterCounts(nodes), {
    all: 5,
    chat: 1,
    tools: 1,
    diffs: 1,
    approvals: 1,
    events: 1
  });
});

test("counts approval-gated tools without rendering a separate approval row", () => {
  const permissionTool: RenderNode = {
    ...base,
    id: "tool:permission",
    kind: "tool",
    toolCallId: "tool-permission",
    title: "Run git log",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: [],
    permission: {
      requestId: "request-1",
      title: "Permission required",
      status: "pending",
      options: [
        { optionId: "allow_always", label: "Always allow" },
        { optionId: "allow", label: "Allow" },
        { optionId: "reject", label: "Reject" }
      ]
    }
  };

  assert.equal(timelineNodeBucket(permissionTool), "tools");
  assert.deepEqual(timelineFilterCounts([permissionTool]), {
    all: 1,
    chat: 0,
    tools: 1,
    diffs: 0,
    approvals: 1,
    events: 0
  });
  assert.deepEqual(filterTimelineNodes([permissionTool], "approvals", "").map((node) => node.id), ["tool:permission"]);
  assert.deepEqual(filterTimelineNodes([permissionTool], "approvals", "always allow").map((node) => node.id), ["tool:permission"]);
});

test("removes internal session controls from transcript-visible nodes", () => {
  const internalNodes: RenderNode[] = [
    ...nodes,
    {
      ...base,
      id: "commands:1",
      kind: "commands",
      availableCommands: [{ name: "help", description: "" }]
    },
    {
      ...base,
      id: "tool:internal",
      kind: "tool",
      toolCallId: "internal",
      title: "ACP prompt turn",
      toolKind: "other",
      toolStatus: "completed",
      locations: [],
      content: []
    },
    {
      ...base,
      id: "session:1",
      kind: "session",
      title: "Provider title"
    },
    {
      ...base,
      id: "unknown:span",
      kind: "unknown",
      label: "span_ended (completed)",
      payload: {}
    }
  ];

  assert.deepEqual(transcriptTimelineNodes(internalNodes).map((node) => node.id), nodes.map((node) => node.id));
});

test("sorts transcript nodes by durable stream order before timestamps", () => {
  const ordered = sortTimelineNodes([
    {
      ...base,
      id: "tool:late-created-missing-time",
      kind: "tool",
      timelineOrder: 3,
      toolCallId: "tool-late",
      title: "Run command",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: []
    },
    {
      ...base,
      id: "message:assistant:first",
      kind: "message",
      timelineOrder: 2,
      createdAt: "2026-06-27T00:00:04.000Z",
      role: "assistant",
      content: [{ type: "text", text: "I will check the repo." }],
      text: "I will check the repo.",
      streaming: false
    },
    {
      ...base,
      id: "message:user:first",
      kind: "message",
      timelineOrder: 1,
      createdAt: "2026-06-27T00:00:05.000Z",
      role: "user",
      content: [{ type: "text", text: "Count it." }],
      text: "Count it.",
      streaming: false
    },
    {
      ...base,
      id: "message:assistant:after",
      kind: "message",
      timelineOrder: 4,
      createdAt: "2026-06-27T00:00:03.000Z",
      role: "assistant",
      content: [{ type: "text", text: "Here is the result." }],
      text: "Here is the result.",
      streaming: false
    }
  ]);

  assert.deepEqual(ordered.map((node) => node.id), [
    "message:user:first",
    "message:assistant:first",
    "tool:late-created-missing-time",
    "message:assistant:after"
  ]);
});

test("filters timeline nodes by group and search query", () => {
  assert.deepEqual(filterTimelineNodes(nodes, "tools", "").map((node) => node.id), ["tool:1"]);
  assert.deepEqual(filterTimelineNodes(nodes, "all", "schema").map((node) => node.id), ["approval:1"]);
  assert.deepEqual(filterTimelineNodes(nodes, "diffs", "readme").map((node) => node.id), ["diff:1"]);
  assert.deepEqual(filterTimelineNodes(nodes, "chat", "schema"), []);
});

test("filters timeline nodes with multi-term search across node fields", () => {
  assert.deepEqual(filterTimelineNodes(nodes, "all", "schema migration").map((node) => node.id), ["approval:1"]);
  assert.deepEqual(filterTimelineNodes(nodes, "all", "migration schema").map((node) => node.id), ["approval:1"]);
  assert.deepEqual(filterTimelineNodes(nodes, "tools", "package read").map((node) => node.id), ["tool:1"]);
  assert.deepEqual(filterTimelineNodes(nodes, "all", "read schema"), []);
});

test("tokenizes timeline search queries defensively", () => {
  assert.deepEqual(timelineSearchTokens("  README   schema\tmigration  "), ["readme", "schema", "migration"]);
  assert.deepEqual(timelineSearchTokens(""), []);
});

test("normalizes timeline search text and filter identifiers", () => {
  assert.equal(timelineNodeSearchText(nodes[3] as RenderNode).includes("Run database migration"), true);
  assert.equal(isTimelineFilter("approvals"), true);
  assert.equal(isTimelineFilter("approval"), false);
});

test("summarizes visible tool activity by operation, risk, and touched paths", () => {
  const mixedNodes: RenderNode[] = [
    nodes[1] as RenderNode,
    {
      ...base,
      id: "tool:edit",
      kind: "tool",
      toolCallId: "tool-edit",
      title: "Edit source file",
      toolKind: "edit",
      toolStatus: "completed",
      locations: [{ path: "src/app.ts" }],
      content: [{ type: "diff", path: "src/app.ts", oldText: "old", newText: "new" }]
    },
    {
      ...base,
      id: "terminal:1",
      kind: "terminal",
      terminalId: "terminal-1",
      title: "Run tests",
      command: "npm test",
      status: "in_progress",
      terminalStatus: "running"
    },
    {
      ...base,
      id: "diff:source",
      kind: "diff",
      path: "src/app.ts",
      oldText: "old",
      newText: "new"
    },
    nodes[3] as RenderNode
  ];

  const summary = buildToolActivitySummary(mixedNodes, 2);

  assert.equal(summary.total, 5);
  assert.equal(summary.tone, "risk");
  assert.equal(summary.label, "Needs inspection");
  assert.match(summary.detail, /1 read-only/);
  assert.match(summary.detail, /2 changes/);
  assert.match(summary.detail, /1 command/);
  assert.match(summary.detail, /1 approval/);
  assert.equal(summary.metrics.find((metric) => metric.label === "operations")?.value, "5");
  assert.equal(summary.metrics.find((metric) => metric.label === "approvals")?.tone, "risk");
  assert.deepEqual(
    summary.paths.map((path) => [path.path, path.count, path.tone]),
    [
      ["src/app.ts", 2, "warning"],
      ["db/schema.sql", 1, "risk"]
    ]
  );
});

test("summarizes empty visible tool activity for filtered transcripts", () => {
  const summary = buildToolActivitySummary([nodes[0] as RenderNode]);

  assert.equal(summary.total, 0);
  assert.equal(summary.tone, "empty");
  assert.equal(summary.metrics.length, 0);
  assert.equal(summary.paths.length, 0);
  assert.match(summary.detail, /current transcript filter/);
});

test("summarizes generic provider tools by inferred operation", () => {
  const summary = buildToolActivitySummary([
    {
      ...base,
      id: "tool:bash",
      kind: "tool",
      toolCallId: "tool-bash",
      title: "Bash",
      toolKind: "other",
      toolStatus: "completed",
      locations: [],
      content: []
    },
    {
      ...base,
      id: "tool:patch",
      kind: "tool",
      toolCallId: "tool-patch",
      title: "tool_call_update",
      toolKind: "other",
      toolStatus: "completed",
      locations: [{ path: "src/app.ts" }],
      rawInput: { toolName: "apply_patch" },
      content: []
    }
  ]);

  assert.equal(summary.total, 2);
  assert.match(summary.detail, /1 change/);
  assert.match(summary.detail, /1 command/);
  assert.equal(summary.paths[0]?.path, "src/app.ts");
  assert.equal(summary.paths[0]?.tone, "warning");
});

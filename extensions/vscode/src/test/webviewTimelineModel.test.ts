import assert from "node:assert/strict";
import test from "node:test";
import {
  buildTimelineConversationGroups,
  buildToolActivitySummary,
  filterTimelineNodes,
  hasTimelineDisplayStructuralChange,
  isInlineToolDiffNode,
  isTimelineFilter,
  sortTimelineNodes,
  timelineDisplayPatchChanges,
  timelineDisplayNodes,
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

test("groups completed transcripts by user prompt boundaries instead of low-level tool turn ids", () => {
  const fragmented: RenderNode[] = [
    {
      ...base,
      id: "message:user:prompt",
      kind: "message",
      turnId: "prompt-turn",
      role: "user",
      content: [{ type: "text", text: "Inspect the repo" }],
      text: "Inspect the repo",
      streaming: false
    },
    {
      ...base,
      id: "tool:read-package",
      kind: "tool",
      turnId: "tool-turn-1",
      toolCallId: "read-package",
      title: "Read package.json",
      toolKind: "read",
      toolStatus: "completed",
      locations: [{ path: "package.json" }],
      content: []
    },
    {
      ...base,
      id: "tool:read-readme",
      kind: "tool",
      turnId: "tool-turn-2",
      toolCallId: "read-readme",
      title: "Read README.md",
      toolKind: "read",
      toolStatus: "completed",
      locations: [{ path: "README.md" }],
      content: []
    },
    {
      ...base,
      id: "message:assistant:answer",
      kind: "message",
      turnId: "assistant-turn",
      role: "assistant",
      content: [{ type: "text", text: "The repo contains the extension." }],
      text: "The repo contains the extension.",
      streaming: false
    }
  ];

  const groups = buildTimelineConversationGroups(fragmented);

  assert.equal(groups.length, 1);
  assert.equal(groups[0]?.turnId, "prompt-turn");
  assert.deepEqual(groups[0]?.nodes.map((node) => node.id), [
    "message:user:prompt",
    "tool:read-package",
    "tool:read-readme",
    "message:assistant:answer"
  ]);
});

test("starts a new visible turn at the next user prompt", () => {
  const twoPrompts: RenderNode[] = [
    {
      ...base,
      id: "message:user:first",
      kind: "message",
      turnId: "turn-first",
      role: "user",
      content: [{ type: "text", text: "First prompt" }],
      text: "First prompt",
      streaming: false
    },
    {
      ...base,
      id: "tool:first",
      kind: "tool",
      turnId: "tool-first",
      toolCallId: "tool-first",
      title: "First tool",
      toolKind: "read",
      toolStatus: "completed",
      locations: [],
      content: []
    },
    {
      ...base,
      id: "message:user:second",
      kind: "message",
      turnId: "turn-second",
      role: "user",
      content: [{ type: "text", text: "Second prompt" }],
      text: "Second prompt",
      streaming: false
    },
    {
      ...base,
      id: "message:assistant:second",
      kind: "message",
      turnId: "assistant-second",
      role: "assistant",
      content: [{ type: "text", text: "Second answer" }],
      text: "Second answer",
      streaming: false
    }
  ];

  const groups = buildTimelineConversationGroups(twoPrompts);

  assert.deepEqual(
    groups.map((group) => [group.turnId, group.nodes.map((node) => node.id)]),
    [
      ["turn-first", ["message:user:first", "tool:first"]],
      ["turn-second", ["message:user:second", "message:assistant:second"]]
    ]
  );
});

test("falls back to low-level turn scopes when a filtered view has no user prompt", () => {
  const toolOnly: RenderNode[] = [
    {
      ...base,
      id: "tool:one",
      kind: "tool",
      turnId: "tool-turn-one",
      toolCallId: "one",
      title: "Read one",
      toolKind: "read",
      toolStatus: "completed",
      locations: [],
      content: []
    },
    {
      ...base,
      id: "tool:two",
      kind: "tool",
      turnId: "tool-turn-two",
      toolCallId: "two",
      title: "Read two",
      toolKind: "read",
      toolStatus: "completed",
      locations: [],
      content: []
    }
  ];

  const groups = buildTimelineConversationGroups(toolOnly);

  assert.deepEqual(
    groups.map((group) => [group.turnId, group.nodes.map((node) => node.id)]),
    [
      ["tool-turn-one", ["tool:one"]],
      ["tool-turn-two", ["tool:two"]]
    ]
  );
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

test("merges approval rows into tools only within the same render scope", () => {
  const oldTool: RenderNode = {
    ...base,
    id: "tool:shared-old",
    kind: "tool",
    turnId: "turn-1",
    acpSessionId: "session-1",
    toolCallId: "shared-tool",
    title: "Old command",
    toolKind: "execute",
    toolStatus: "completed",
    locations: [],
    content: []
  };
  const currentTool: RenderNode = {
    ...oldTool,
    id: "tool:shared-current",
    turnId: "turn-2",
    title: "Current command",
    toolStatus: "pending"
  };
  const currentApproval: RenderNode = {
    ...base,
    id: "approval:shared-current",
    kind: "approval",
    turnId: "turn-2",
    acpSessionId: "session-1",
    requestId: "approval-current",
    title: "Allow current command",
    tool: currentTool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending"
  };

  const displayed = timelineDisplayNodes([oldTool, currentTool, currentApproval]);

  assert.deepEqual(displayed.map((node) => node.id), ["tool:shared-old", "tool:shared-current"]);
  assert.equal(displayed[0]?.kind, "tool");
  assert.equal(displayed[1]?.kind, "tool");
  if (displayed[0]?.kind !== "tool" || displayed[1]?.kind !== "tool") {
    throw new Error("expected tool display nodes");
  }
  assert.equal(displayed[0].permission, undefined);
  assert.equal(displayed[1].permission?.requestId, "approval-current");
});

test("merges approval rows into tools when provider and display tool ids differ", () => {
  const tool: RenderNode = {
    ...base,
    id: "tool:display-command",
    kind: "tool",
    turnId: "turn-provider-alias",
    acpSessionId: "session-1",
    toolCallId: "display-command",
    acpToolCallId: "provider-command",
    title: "Run aliased command",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: []
  };
  const approvalTool: RenderNode = {
    ...tool,
    id: "tool:approval-wrapper",
    toolCallId: "approval-wrapper",
    acpToolCallId: "provider-command",
    title: "Approve aliased command"
  };
  const approval: RenderNode = {
    ...base,
    id: "approval:provider-alias",
    kind: "approval",
    turnId: "turn-provider-alias",
    acpSessionId: "session-1",
    acpToolCallId: "provider-command",
    requestId: "approval-provider-alias",
    title: "Allow aliased command",
    tool: approvalTool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending"
  };

  const displayed = timelineDisplayNodes([tool, approval]);

  assert.deepEqual(displayed.map((node) => node.id), ["tool:display-command"]);
  assert.equal(displayed[0]?.kind, "tool");
  if (displayed[0]?.kind !== "tool") {
    throw new Error("expected folded approval tool row");
  }
  assert.equal(displayed[0].permission?.requestId, "approval-provider-alias");
});

test("maps folded approval additions to the visible tool row", () => {
  const tool: RenderNode = {
    ...base,
    id: "tool:approval-added",
    kind: "tool",
    turnId: "turn-approval-added",
    acpSessionId: "session-1",
    toolCallId: "approval-added",
    title: "Run approval-gated command",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: []
  };
  const approval: RenderNode = {
    ...base,
    id: "approval:added",
    kind: "approval",
    turnId: "turn-approval-added",
    acpSessionId: "session-1",
    requestId: "approval-added",
    title: "Allow command",
    tool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending"
  };

  const changes = timelineDisplayPatchChanges(
    [tool],
    [tool, approval],
    {
      changedNodeIds: new Set(),
      addedNodeIds: new Set([approval.id]),
      removedNodeIds: new Set()
    }
  );

  assert.deepEqual([...changes.changedNodeIds], ["tool:approval-added"]);
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.removedNodeIds], []);
});

test("keeps card-only display changes out of timeline structure hydration", () => {
  const before: RenderNode = {
    ...base,
    id: "message:assistant:content-only",
    kind: "message",
    role: "assistant",
    content: [{ type: "text", text: "Hello" }],
    text: "Hello",
    streaming: false,
    turnId: "turn-card-only",
    acpSessionId: "session-1"
  };
  const after: RenderNode = {
    ...before,
    content: [{ type: "text", text: "Hello world" }],
    text: "Hello world"
  };
  const changes = timelineDisplayPatchChanges(
    [before],
    [after],
    {
      changedNodeIds: new Set([after.id]),
      addedNodeIds: new Set(),
      removedNodeIds: new Set()
    }
  );

  assert.equal(hasTimelineDisplayStructuralChange([before], [after], changes), false);
});

test("treats folded approval additions as display structural changes", () => {
  const tool: RenderNode = {
    ...base,
    id: "tool:approval-display-structure",
    kind: "tool",
    turnId: "turn-approval-display-structure",
    acpSessionId: "session-1",
    toolCallId: "approval-display-structure",
    title: "Run approval-gated command",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: []
  };
  const approval: RenderNode = {
    ...base,
    id: "approval:display-structure",
    kind: "approval",
    turnId: "turn-approval-display-structure",
    acpSessionId: "session-1",
    requestId: "approval-display-structure",
    title: "Allow command",
    tool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending"
  };
  const changes = timelineDisplayPatchChanges(
    [tool],
    [tool, approval],
    {
      changedNodeIds: new Set(),
      addedNodeIds: new Set([approval.id]),
      removedNodeIds: new Set()
    }
  );

  assert.equal(hasTimelineDisplayStructuralChange([tool], [tool, approval], changes), true);
});

test("maps folded approval status changes to the visible tool row", () => {
  const tool: RenderNode = {
    ...base,
    id: "tool:approval-updated",
    kind: "tool",
    turnId: "turn-approval-updated",
    acpSessionId: "session-1",
    toolCallId: "approval-updated",
    title: "Run approval-updated command",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: []
  };
  const pendingApproval: RenderNode = {
    ...base,
    id: "approval:updated",
    kind: "approval",
    turnId: "turn-approval-updated",
    acpSessionId: "session-1",
    requestId: "approval-updated",
    title: "Allow command",
    tool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending"
  };
  const completedApproval: RenderNode = {
    ...pendingApproval,
    status: "completed"
  };

  const changes = timelineDisplayPatchChanges(
    [tool, pendingApproval],
    [tool, completedApproval],
    {
      changedNodeIds: new Set([completedApproval.id]),
      addedNodeIds: new Set(),
      removedNodeIds: new Set()
    }
  );

  assert.deepEqual([...changes.changedNodeIds], ["tool:approval-updated"]);
  assert.deepEqual([...changes.addedNodeIds], []);
  assert.deepEqual([...changes.removedNodeIds], []);
});

test("treats tool grouping status changes as display structural changes", () => {
  const pending: RenderNode = {
    ...base,
    id: "tool:grouping-status",
    kind: "tool",
    turnId: "turn-grouping-status",
    acpSessionId: "session-1",
    toolCallId: "grouping-status",
    title: "Run grouped command",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: []
  };
  const completed: RenderNode = {
    ...pending,
    toolStatus: "completed"
  };
  const changes = timelineDisplayPatchChanges(
    [pending],
    [completed],
    {
      changedNodeIds: new Set([completed.id]),
      addedNodeIds: new Set(),
      removedNodeIds: new Set()
    }
  );

  assert.equal(hasTimelineDisplayStructuralChange([pending], [completed], changes), true);
});

test("keeps reused tool approvals visible when their matching tool is absent in that scope", () => {
  const oldTool: RenderNode = {
    ...base,
    id: "tool:shared-old",
    kind: "tool",
    turnId: "turn-1",
    acpSessionId: "session-1",
    toolCallId: "shared-tool",
    title: "Old command",
    toolKind: "execute",
    toolStatus: "completed",
    locations: [],
    content: []
  };
  const laterApprovalTool: Extract<RenderNode, { kind: "tool" }> = {
    ...oldTool,
    id: "tool:shared-later",
    turnId: "turn-2",
    title: "Later command",
    toolStatus: "pending"
  };
  const laterApproval: RenderNode = {
    ...base,
    id: "approval:shared-later",
    kind: "approval",
    turnId: "turn-2",
    acpSessionId: "session-1",
    requestId: "approval-later",
    title: "Allow later command",
    tool: laterApprovalTool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending"
  };

  const displayed = timelineDisplayNodes([oldTool, laterApproval]);

  assert.deepEqual(displayed.map((node) => node.id), ["tool:shared-old", "approval:shared-later"]);
  assert.equal(displayed[0]?.kind === "tool" ? displayed[0].permission : undefined, undefined);
  assert.equal(displayed[1]?.kind === "tool" ? displayed[1].permission?.requestId : undefined, "approval-later");
});

test("preserves standalone approval timeline order when converting it to a tool row", () => {
  const before: RenderNode = {
    ...base,
    id: "message:before-approval",
    kind: "message",
    role: "assistant",
    content: [{ type: "text", text: "I need permission." }],
    text: "I need permission.",
    streaming: false,
    timelineOrder: 1
  };
  const approval: RenderNode = {
    ...base,
    id: "approval:ordered",
    kind: "approval",
    requestId: "approval-ordered",
    title: "Allow command",
    tool: {
      ...base,
      id: "tool:ordered-approval",
      kind: "tool",
      toolCallId: "ordered-approval",
      title: "Run command",
      toolKind: "execute",
      toolStatus: "pending",
      locations: [],
      content: []
    },
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending",
    timelineOrder: 2
  };
  const after: RenderNode = {
    ...base,
    id: "message:after-approval",
    kind: "message",
    role: "assistant",
    content: [{ type: "text", text: "Permission is pending." }],
    text: "Permission is pending.",
    streaming: false,
    timelineOrder: 3
  };

  const displayed = timelineDisplayNodes([after, approval, before]);
  const approvalRow = displayed.find((node) => node.id === "approval:ordered");
  const ordered = sortTimelineNodes(displayed);

  assert.equal(approvalRow?.timelineOrder, 2);
  assert.deepEqual(ordered.map((node) => node.id), [
    "message:before-approval",
    "approval:ordered",
    "message:after-approval"
  ]);
});

test("keeps folded approval rows at the earliest approval or tool timeline position", () => {
  const afterApprovalMessage: RenderNode = {
    ...base,
    id: "message:after-folded-approval",
    kind: "message",
    role: "assistant",
    content: [{ type: "text", text: "Waiting on approval." }],
    text: "Waiting on approval.",
    streaming: false,
    timelineOrder: 3
  };
  const approvalTool: Extract<RenderNode, { kind: "tool" }> = {
    ...base,
    id: "tool:approval-arrives-first",
    kind: "tool",
    turnId: "turn-approval-first",
    acpSessionId: "session-1",
    toolCallId: "approval-arrives-first",
    title: "Run gated command",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: [],
    timelineOrder: 4,
    createdAt: "2026-06-27T00:00:04.000Z"
  };
  const approval: RenderNode = {
    ...base,
    id: "approval:arrives-first",
    kind: "approval",
    turnId: "turn-approval-first",
    acpSessionId: "session-1",
    requestId: "approval-arrives-first",
    title: "Allow gated command",
    tool: approvalTool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending",
    timelineOrder: 2,
    createdAt: "2026-06-27T00:00:02.000Z"
  };

  const displayed = sortTimelineNodes(timelineDisplayNodes([afterApprovalMessage, approval, approvalTool]));

  assert.deepEqual(displayed.map((node) => node.id), ["tool:approval-arrives-first", "message:after-folded-approval"]);
  assert.equal(displayed[0]?.timelineOrder, 2);
  assert.equal(displayed[0]?.createdAt, "2026-06-27T00:00:02.000Z");
});

test("keeps folded approvals, late assistant continuations, and completion markers in visible stream order", () => {
  const first: RenderNode = {
    ...base,
    id: "message:visible-final:first",
    kind: "message",
    role: "assistant",
    content: [{ type: "text", text: "I will run this." }],
    text: "I will run this.",
    streaming: false,
    turnId: "turn-visible-final",
    acpSessionId: "session-1",
    timelineOrder: 1
  };
  const approvalTool: Extract<RenderNode, { kind: "tool" }> = {
    ...base,
    id: "tool:visible-final-gated",
    kind: "tool",
    turnId: "turn-visible-final",
    acpSessionId: "session-1",
    toolCallId: "visible-final-gated",
    title: "Run gated command",
    toolKind: "execute",
    toolStatus: "completed",
    locations: [],
    content: [],
    timelineOrder: 3
  };
  const approval: RenderNode = {
    ...base,
    id: "approval:visible-final-gated",
    kind: "approval",
    turnId: "turn-visible-final",
    acpSessionId: "session-1",
    requestId: "approval-visible-final-gated",
    title: "Allow gated command",
    tool: approvalTool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "completed",
    timelineOrder: 2
  };
  const late: RenderNode = {
    ...base,
    id: "message:visible-final:late",
    kind: "message",
    role: "assistant",
    content: [{ type: "text", text: "Final answer." }],
    text: "Final answer.",
    streaming: false,
    turnId: "turn-visible-final",
    acpSessionId: "session-1",
    timelineOrder: 4
  };
  const beforeCompletion: RenderNode = {
    ...base,
    id: "completion:visible-final",
    kind: "completion",
    turnId: "turn-visible-final",
    acpSessionId: "session-1",
    stopReason: "end_turn",
    label: "Turn complete; checkpoint pending",
    checkpointPending: true,
    status: "pending",
    timelineOrder: 3
  };
  const afterCompletion: RenderNode = {
    ...beforeCompletion,
    timelineOrder: 5
  };
  const beforeNodes = [first, approvalTool, approval, beforeCompletion];
  const nextNodes = [beforeCompletion, late, approvalTool, first, approval].map((node) =>
    node.id === beforeCompletion.id ? afterCompletion : node
  );

  const displayed = sortTimelineNodes(timelineDisplayNodes(nextNodes));
  const foldedTool = displayed.find((node) => node.id === approvalTool.id);

  assert.deepEqual(displayed.map((node) => node.id), [
    "message:visible-final:first",
    "tool:visible-final-gated",
    "message:visible-final:late",
    "completion:visible-final"
  ]);
  assert.equal(foldedTool?.kind === "tool" ? foldedTool.permission?.requestId : undefined, approval.requestId);
  assert.equal(displayed.at(-1)?.kind, "completion");

  const displayChanges = timelineDisplayPatchChanges(
    beforeNodes,
    nextNodes,
    {
      changedNodeIds: new Set([afterCompletion.id]),
      addedNodeIds: new Set([late.id]),
      removedNodeIds: new Set()
    }
  );

  assert.deepEqual([...displayChanges.addedNodeIds], [late.id]);
  assert.equal(displayChanges.changedNodeIds.has(afterCompletion.id), true);
  assert.equal(displayChanges.changedNodeIds.has(approval.id), false);
  assert.equal(hasTimelineDisplayStructuralChange(beforeNodes, nextNodes, displayChanges), true);
});

test("maps late matching tool additions over standalone approval rows", () => {
  const tool: RenderNode = {
    ...base,
    id: "tool:late-approval-tool",
    kind: "tool",
    turnId: "turn-late-approval-tool",
    acpSessionId: "session-1",
    toolCallId: "late-approval-tool",
    title: "Run late approval tool",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: [],
    timelineOrder: 4
  };
  const approval: RenderNode = {
    ...base,
    id: "approval:late-tool",
    kind: "approval",
    turnId: "turn-late-approval-tool",
    acpSessionId: "session-1",
    requestId: "approval-late-tool",
    title: "Allow late tool",
    tool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "pending",
    timelineOrder: 2
  };

  const changes = timelineDisplayPatchChanges(
    [approval],
    [approval, tool],
    {
      changedNodeIds: new Set(),
      addedNodeIds: new Set([tool.id]),
      removedNodeIds: new Set()
    }
  );

  assert.deepEqual([...changes.changedNodeIds], []);
  assert.deepEqual([...changes.addedNodeIds], ["tool:late-approval-tool"]);
  assert.deepEqual([...changes.removedNodeIds], ["approval:late-tool"]);
});

test("keeps extra same-tool approval events visible when one approval is merged into the tool", () => {
  const tool: RenderNode = {
    ...base,
    id: "tool:repeated-approval",
    kind: "tool",
    turnId: "turn-approval-repeat",
    acpSessionId: "session-1",
    toolCallId: "repeated-approval-tool",
    title: "Run repeated approval tool",
    toolKind: "execute",
    toolStatus: "pending",
    locations: [],
    content: [],
    timelineOrder: 1
  };
  const resolvedApproval: RenderNode = {
    ...base,
    id: "approval:repeated-resolved",
    kind: "approval",
    turnId: "turn-approval-repeat",
    acpSessionId: "session-1",
    requestId: "approval-resolved",
    title: "Resolved approval",
    tool,
    options: [{ optionId: "allow", label: "Allow" }],
    status: "completed",
    timelineOrder: 2
  };
  const pendingApproval: RenderNode = {
    ...resolvedApproval,
    id: "approval:repeated-pending",
    requestId: "approval-pending",
    title: "Pending approval",
    status: "pending",
    timelineOrder: 3
  };

  const displayed = timelineDisplayNodes([tool, resolvedApproval, pendingApproval]);

  assert.deepEqual(displayed.map((node) => node.id), ["tool:repeated-approval", "approval:repeated-resolved"]);
  assert.equal(displayed[0]?.kind, "tool");
  assert.equal(displayed[1]?.kind, "tool");
  if (displayed[0]?.kind !== "tool" || displayed[1]?.kind !== "tool") {
    throw new Error("expected merged and standalone approval tool rows");
  }
  assert.equal(displayed[0].permission?.requestId, "approval-pending");
  assert.equal(displayed[1].permission?.requestId, "approval-resolved");
  assert.equal(displayed[1].status, "completed");
  assert.equal(displayed[1].toolStatus, "completed");
});

test("removes internal session controls from transcript-visible nodes", () => {
  const providerEvent: RenderNode = {
    ...base,
    id: "unknown:provider-event",
    kind: "unknown",
    label: "Unsupported ACP update: custom_progress",
    payload: {
      sessionUpdate: "custom_progress",
      detail: "Provider-specific event"
    }
  };
  const internalNodes: RenderNode[] = [
    ...nodes,
    providerEvent,
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

  assert.deepEqual(transcriptTimelineNodes(internalNodes).map((node) => node.id), [...nodes.map((node) => node.id), providerEvent.id]);
  assert.equal(timelineNodeBucket(providerEvent), "events");
  assert.deepEqual(filterTimelineNodes(transcriptTimelineNodes(internalNodes), "events", "custom_progress").map((node) => node.id), [providerEvent.id]);
});

test("folds inline tool diffs through scoped provider tool id aliases", () => {
  const matchingTool: RenderNode = {
    ...base,
    id: "tool:inline-diff",
    kind: "tool",
    turnId: "turn-inline-diff",
    acpSessionId: "session-1",
    toolCallId: "display-edit",
    acpToolCallId: "provider-edit",
    title: "Edit README",
    toolKind: "edit",
    toolStatus: "completed",
    locations: [{ path: "README.md" }],
    content: [{ type: "diff", path: "README.md", oldText: "old", newText: "new" }]
  };
  const reusedEarlierTool: RenderNode = {
    ...matchingTool,
    id: "tool:inline-diff-earlier",
    turnId: "turn-earlier"
  };
  const diff: RenderNode = {
    ...base,
    id: "diff:inline-diff",
    kind: "diff",
    turnId: "turn-inline-diff",
    acpSessionId: "session-1",
    acpToolCallId: "provider-edit",
    path: "README.md",
    oldText: "old",
    newText: "new"
  };

  assert.equal(isInlineToolDiffNode([matchingTool, diff], diff), true);
  assert.equal(isInlineToolDiffNode([reusedEarlierTool, diff], diff), false);
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

test("retains incoming sequence when timeline order and timestamps are unavailable", () => {
  const ordered = sortTimelineNodes([
    {
      ...base,
      id: "completion:no-order",
      kind: "completion",
      stopReason: "end_turn",
      label: "Turn complete",
      checkpointPending: false
    },
    {
      ...base,
      id: "message:no-order",
      kind: "message",
      role: "assistant",
      content: [{ type: "text", text: "No ordering metadata." }],
      text: "No ordering metadata.",
      streaming: false
    },
    {
      ...base,
      id: "tool:no-order",
      kind: "tool",
      toolCallId: "no-order",
      title: "Run without timestamps",
      toolKind: "execute",
      toolStatus: "completed",
      locations: [],
      content: []
    }
  ]);

  assert.deepEqual(ordered.map((node) => node.id), [
    "completion:no-order",
    "message:no-order",
    "tool:no-order"
  ]);
});

test("sorts by updated time when created time is malformed", () => {
  const ordered = sortTimelineNodes([
    {
      ...base,
      id: "message:assistant:bad-created",
      kind: "message",
      createdAt: "not-a-date",
      updatedAt: "2026-06-27T00:00:02.000Z",
      role: "assistant",
      content: [{ type: "text", text: "Recovered timestamp" }],
      text: "Recovered timestamp",
      streaming: false
    },
    {
      ...base,
      id: "message:assistant:later",
      kind: "message",
      createdAt: "2026-06-27T00:00:03.000Z",
      role: "assistant",
      content: [{ type: "text", text: "Later timestamp" }],
      text: "Later timestamp",
      streaming: false
    }
  ]);

  assert.deepEqual(ordered.map((node) => node.id), ["message:assistant:bad-created", "message:assistant:later"]);
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

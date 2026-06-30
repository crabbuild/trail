import assert from "node:assert/strict";
import test from "node:test";
import { RenderStreamScheduler } from "../shared/renderStreamScheduler";
import type { RenderPatch } from "../shared/renderModel";

function messagePatch(text: string, id = "message:assistant:msg-1", turnId = "turn-1"): RenderPatch {
  return {
    type: "upsert",
    node: {
      id,
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId,
      acpSessionId: "session-1",
      acpMessageId: "msg-1",
      provider: "test",
      source: "acp-live",
      status: "in_progress",
      createdAt: "2026-06-27T00:00:00.000Z",
      updatedAt: "2026-06-27T00:00:00.000Z",
      role: "assistant",
      content: [{ type: "text", text }],
      text,
      streaming: true
    }
  };
}

function thoughtPatch(text: string, id = "thought:msg-1", turnId = "turn-1"): RenderPatch {
  return {
    type: "upsert",
    node: {
      id,
      kind: "thought",
      taskId: "task-1",
      lane: "lane-1",
      turnId,
      acpSessionId: "session-1",
      acpMessageId: "msg-1",
      provider: "test",
      source: "acp-live",
      status: "in_progress",
      createdAt: "2026-06-27T00:00:00.000Z",
      updatedAt: "2026-06-27T00:00:00.000Z",
      content: [{ type: "text", text }],
      ephemeral: true
    }
  };
}

function toolPatch(): RenderPatch {
  return {
    type: "upsert",
    node: {
      id: "tool:read",
      kind: "tool",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-1",
      acpSessionId: "session-1",
      provider: "test",
      source: "acp-live",
      status: "completed",
      raw: {
        sessionUpdate: "tool_call",
        status: "completed"
      },
      toolCallId: "read",
      title: "Read README.md",
      toolKind: "read",
      toolStatus: "completed",
      locations: [{ path: "README.md" }],
      content: []
    }
  };
}

function terminalPatch(
  output: string,
  turnId = "turn-1",
  options: { command?: string | undefined; cwd?: string | undefined } = {}
): RenderPatch {
  return {
    type: "upsert",
    node: {
      id: "terminal:read",
      kind: "terminal",
      taskId: "task-1",
      lane: "lane-1",
      turnId,
      acpSessionId: "session-1",
      acpToolCallId: "read",
      provider: "test",
      source: "acp-live",
      status: "in_progress",
      terminalId: "read",
      title: "Read README.md",
      command: options.command,
      cwd: options.cwd,
      stdout: output
    }
  };
}

function diffPatch(newText: string, oldText?: string | null | undefined): RenderPatch {
  const node: Extract<NonNullable<RenderPatch["node"]>, { kind: "diff" }> = {
    id: "diff:edit:README.md",
    kind: "diff",
    taskId: "task-1",
    lane: "lane-1",
    turnId: "turn-1",
    acpSessionId: "session-1",
    acpToolCallId: "edit",
    provider: "test",
    source: "acp-live",
    status: "in_progress",
    path: "README.md",
    newText
  };
  if (oldText !== undefined) {
    node.oldText = oldText;
  }
  return {
    type: "upsert",
    node
  };
}

test("coalesces streamed text patches by node id", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([messagePatch("Hello ")]);
  scheduler.push([messagePatch("world")]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message patch");
  }
  assert.equal(node.text, "Hello world");
  assert.deepEqual(node.content, [{ type: "text", text: "Hello world" }]);
  assert.equal(scheduler.stats().coalesced, 1);
});

test("coalesces anonymous streamed text patches when message ids arrive late", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });
  const anonymous = messagePatch("Hello ", "message:assistant:anonymous");
  const identified = messagePatch("world", "message:assistant:msg-promoted");
  if (anonymous.node?.kind !== "message" || identified.node?.kind !== "message") {
    throw new Error("expected message patches");
  }
  anonymous.node.acpMessageId = undefined;
  identified.node.acpMessageId = "msg-promoted";

  scheduler.push([anonymous]);
  scheduler.push([identified]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message patch");
  }
  assert.equal(node.id, "message:assistant:anonymous");
  assert.equal(node.acpMessageId, "msg-promoted");
  assert.equal(node.text, "Hello world");
  assert.deepEqual(node.content, [{ type: "text", text: "Hello world" }]);
  assert.equal(scheduler.stats().coalesced, 1);
});

test("coalesces streamed text patches when session ids arrive late", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });
  const withoutSession = messagePatch("Hello ", "message:assistant:msg-session-promoted");
  const withSession = messagePatch("world", "message:assistant:msg-session-promoted");
  if (withoutSession.node?.kind !== "message" || withSession.node?.kind !== "message") {
    throw new Error("expected message patches");
  }
  delete withoutSession.node.acpSessionId;
  withSession.node.acpSessionId = "session-1";

  scheduler.push([withoutSession]);
  scheduler.push([withSession]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message patch");
  }
  assert.equal(node.id, "message:assistant:msg-session-promoted");
  assert.equal(node.acpSessionId, "session-1");
  assert.equal(node.text, "Hello world");
  assert.deepEqual(node.content, [{ type: "text", text: "Hello world" }]);
  assert.equal(scheduler.stats().coalesced, 1);
});

test("preserves queued message stream scope and order when later chunks omit metadata", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });
  const first = messagePatch("Hello ");
  const second = messagePatch("world");
  if (first.node?.kind !== "message" || second.node?.kind !== "message") {
    throw new Error("expected message patches");
  }
  first.node.timelineOrder = 7;
  delete second.node.acpSessionId;

  scheduler.push([first]);
  scheduler.push([second]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message patch");
  }
  assert.equal(node.acpSessionId, "session-1");
  assert.equal(node.timelineOrder, 7);
  assert.equal(node.text, "Hello world");
});

test("preserves queued thought stream scope and order when later chunks omit metadata", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });
  const first = thoughtPatch("Looking ");
  const second = thoughtPatch("deeper");
  if (first.node?.kind !== "thought" || second.node?.kind !== "thought") {
    throw new Error("expected thought patches");
  }
  first.node.timelineOrder = 8;
  delete second.node.acpSessionId;

  scheduler.push([first]);
  scheduler.push([second]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "thought");
  if (node?.kind !== "thought") {
    throw new Error("expected thought patch");
  }
  assert.equal(node.acpSessionId, "session-1");
  assert.equal(node.timelineOrder, 8);
  assert.deepEqual(node.content, [{ type: "text", text: "Looking deeper" }]);
});

test("coalesces streamed text patches with aliased text fields", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });
  const first = messagePatch("Hello ");
  const second = messagePatch("world");
  if (first.node?.kind !== "message" || second.node?.kind !== "message") {
    throw new Error("expected message patches");
  }
  first.node.content = [{ type: "text", content: "Hello " }];
  second.node.content = [{ type: "text", value: "world" }];

  scheduler.push([first]);
  scheduler.push([second]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message patch");
  }
  assert.equal(node.text, "Hello world");
  assert.deepEqual(node.content, [{ type: "text", content: "Hello ", text: "Hello world" }]);
});

test("coalesces accumulated streamed message patches without duplicating text", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([messagePatch("Hello ")]);
  scheduler.push([messagePatch("Hello world")]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "message");
  if (node?.kind !== "message") {
    throw new Error("expected message patch");
  }
  assert.equal(node.text, "Hello world");
  assert.deepEqual(node.content, [{ type: "text", text: "Hello world" }]);
});

test("coalesces accumulated streamed thought patches without duplicating text", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([thoughtPatch("Looking ")]);
  scheduler.push([thoughtPatch("Looking deeper")]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "thought");
  if (node?.kind !== "thought") {
    throw new Error("expected thought patch");
  }
  assert.deepEqual(node.content, [{ type: "text", text: "Looking deeper" }]);
});

test("does not coalesce streamed text patches with reused ids across turns", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([messagePatch("First turn", "message:assistant:msg-1", "turn-1")]);
  scheduler.push([messagePatch("Second turn", "message:assistant:msg-1", "turn-2")]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 2);
  const nodes = batches[0]?.map((patch) => patch.node);
  assert.deepEqual(
    nodes?.map((node) => node?.kind === "message" ? [node.turnId, node.text] : undefined),
    [
      ["turn-1", "First turn"],
      ["turn-2", "Second turn"]
    ]
  );
  assert.equal(scheduler.stats().coalesced, 0);
});

test("flushes queued stream patches before ordering-sensitive patches", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([messagePatch("Before tool")]);
  scheduler.push([toolPatch()]);

  assert.equal(batches.length, 2);
  assert.equal(batches[0]?.[0]?.node?.kind, "message");
  assert.equal(batches[1]?.[0]?.node?.kind, "tool");
});

test("preserves mixed queued patch order when structural patches force a flush", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    flushMs: 10_000,
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "terminal"
  });

  scheduler.push([terminalPatch("running\n")]);
  scheduler.push([messagePatch("After terminal")]);
  scheduler.push([toolPatch()]);

  assert.equal(batches.length, 2);
  assert.deepEqual(batches[0]?.map((patch) => patch.node?.kind), ["terminal", "message"]);
  assert.deepEqual(batches[1]?.map((patch) => patch.node?.kind), ["tool"]);
});

test("keeps structural patches from one ACP update in a single batch", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([
    toolPatch(),
    {
      type: "upsert",
      node: {
        id: "diff:read:README.md",
        kind: "diff",
        taskId: "task-1",
        lane: "lane-1",
        turnId: "turn-1",
        acpSessionId: "session-1",
        acpToolCallId: "read",
        provider: "test",
        source: "acp-live",
        status: "completed",
        path: "README.md",
        oldText: "old",
        newText: "new"
      }
    }
  ]);

  assert.equal(batches.length, 1);
  assert.deepEqual(batches[0]?.map((patch) => patch.node?.kind), ["tool", "diff"]);
});

test("coalesces configured live component patches by node id", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "terminal"
  });

  scheduler.push([terminalPatch("one")]);
  scheduler.push([terminalPatch("two")]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "terminal");
  if (node?.kind !== "terminal") {
    throw new Error("expected terminal patch");
  }
  assert.equal(node.stdout, "two");
  assert.equal(scheduler.stats().coalesced, 1);
});

test("coalesced component patches preserve metadata from queued updates", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "terminal"
  });

  scheduler.push([terminalPatch("", "turn-1", { command: "npm test", cwd: "/workspace" })]);
  scheduler.push([terminalPatch("running\n")]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "terminal");
  if (node?.kind !== "terminal") {
    throw new Error("expected terminal patch");
  }
  assert.equal(node.command, "npm test");
  assert.equal(node.cwd, "/workspace");
  assert.equal(node.stdout, "running\n");
  assert.equal(scheduler.stats().coalesced, 1);
});

test("coalesced terminal patches append explicit stdout deltas", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "terminal"
  });
  const delta = terminalPatch("", "turn-1");
  if (delta.node?.kind !== "terminal") {
    throw new Error("expected terminal patch");
  }
  delta.node.stdout = undefined;
  (delta.node as unknown as Record<string, unknown>).stdoutDelta = "two\n";

  scheduler.push([terminalPatch("one\n")]);
  scheduler.push([delta]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "terminal");
  if (node?.kind !== "terminal") {
    throw new Error("expected terminal patch");
  }
  assert.equal(node.stdout, "one\ntwo\n");
});

test("coalesced diff patches preserve previous before text", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "diff"
  });

  scheduler.push([diffPatch("draft", "before")]);
  scheduler.push([diffPatch("after")]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "diff");
  if (node?.kind !== "diff") {
    throw new Error("expected diff patch");
  }
  assert.equal(node.oldText, "before");
  assert.equal(node.newText, "after");
  assert.equal(node.createdAt, undefined);
  assert.equal(scheduler.stats().coalesced, 1);
});

test("coalesced stale terminal updates preserve completed lifecycle state", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "terminal"
  });
  const completed = terminalPatch("done");
  if (completed.node?.kind !== "terminal") {
    throw new Error("expected terminal patch");
  }
  completed.node.status = "completed";
  completed.node.terminalStatus = "completed";
  const staleActive = terminalPatch("done\nlate output");
  if (staleActive.node?.kind !== "terminal") {
    throw new Error("expected terminal patch");
  }
  staleActive.node.terminalStatus = "running";

  scheduler.push([completed]);
  scheduler.push([staleActive]);
  scheduler.flush();

  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "terminal");
  if (node?.kind !== "terminal") {
    throw new Error("expected terminal patch");
  }
  assert.equal(node.status, "completed");
  assert.equal(node.terminalStatus, "completed");
  assert.equal(node.stdout, "done\nlate output");
});

test("coalesced stale diff updates preserve completed lifecycle state", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "diff"
  });
  const completed = diffPatch("after", "before");
  if (completed.node?.kind !== "diff") {
    throw new Error("expected diff patch");
  }
  completed.node.status = "completed";
  const staleActive = diffPatch("after\nlate note");

  scheduler.push([completed]);
  scheduler.push([staleActive]);
  scheduler.flush();

  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "diff");
  if (node?.kind !== "diff") {
    throw new Error("expected diff patch");
  }
  assert.equal(node.status, "completed");
  assert.equal(node.oldText, "before");
  assert.equal(node.newText, "after\nlate note");
});

test("does not coalesce configured component patches with reused ids across turns", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "terminal"
  });

  scheduler.push([terminalPatch("first output", "turn-1")]);
  scheduler.push([terminalPatch("second output", "turn-2")]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 2);
  const nodes = batches[0]?.map((patch) => patch.node);
  assert.deepEqual(
    nodes?.map((node) => node?.kind === "terminal" ? [node.turnId, node.stdout] : undefined),
    [
      ["turn-1", "first output"],
      ["turn-2", "second output"]
    ]
  );
  assert.equal(scheduler.stats().coalesced, 0);
});

test("flushes older queued component patches before token timer output", async () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    flushMs: 0,
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "terminal"
  });

  scheduler.push([terminalPatch("component one")]);
  scheduler.push([messagePatch("token")]);
  await new Promise((resolve) => setTimeout(resolve, 5));

  assert.equal(batches.length, 1);
  assert.deepEqual(batches[0]?.map((patch) => patch.node?.kind), ["terminal", "message"]);

  scheduler.flush();
  assert.equal(batches.length, 1);
});

test("keeps later component coalescing independent from token flushes", async () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    flushMs: 0,
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "terminal"
  });

  scheduler.push([messagePatch("token")]);
  scheduler.push([terminalPatch("component one")]);
  await new Promise((resolve) => setTimeout(resolve, 5));

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.[0]?.node?.kind, "message");

  scheduler.flush();
  assert.equal(batches.length, 2);
  assert.equal(batches[1]?.[0]?.node?.kind, "terminal");
});

test("coalesced status-less tool updates preserve completed lifecycle state", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "tool"
  });
  const completed = toolPatch();
  const statuslessUpdate: RenderPatch = {
    type: "upsert",
    node: {
      ...completed.node!,
      status: "in_progress",
      raw: {
        sessionUpdate: "tool_call_update"
      },
      title: "Tool call",
      toolKind: "other",
      toolStatus: "in_progress",
      locations: [],
      content: [
        {
          type: "content",
          content: {
            type: "text",
            text: "README contents"
          }
        }
      ]
    } as Extract<NonNullable<RenderPatch["node"]>, { kind: "tool" }>
  };

  scheduler.push([completed]);
  scheduler.push([statuslessUpdate]);
  scheduler.flush();

  assert.equal(batches.length, 1);
  assert.equal(batches[0]?.length, 1);
  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "tool");
  if (node?.kind !== "tool") {
    throw new Error("expected tool patch");
  }
  assert.equal(node.status, "completed");
  assert.equal(node.toolStatus, "completed");
  assert.equal(node.title, "Read README.md");
  assert.equal(node.toolKind, "read");
  assert.equal(node.content.length, 1);
});

test("coalesced stale explicit tool updates preserve completed lifecycle state", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "tool"
  });
  const completed = toolPatch();
  const staleActive: RenderPatch = {
    type: "upsert",
    node: {
      ...completed.node!,
      status: "in_progress",
      raw: {
        sessionUpdate: "tool_call_update",
        status: "in_progress"
      },
      title: "Tool call",
      toolKind: "other",
      toolStatus: "in_progress",
      content: [
        {
          type: "content",
          content: {
            type: "text",
            text: "late tool output"
          }
        }
      ]
    } as Extract<NonNullable<RenderPatch["node"]>, { kind: "tool" }>
  };

  scheduler.push([completed]);
  scheduler.push([staleActive]);
  scheduler.flush();

  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "tool");
  if (node?.kind !== "tool") {
    throw new Error("expected tool patch");
  }
  assert.equal(node.status, "completed");
  assert.equal(node.toolStatus, "completed");
  assert.equal(node.title, "Read README.md");
  assert.equal(node.toolKind, "read");
  assert.equal(node.content.length, 1);
});

test("coalesced tool patches merge distinct content blocks", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "tool"
  });
  const first: RenderPatch = {
    type: "upsert",
    node: {
      ...toolPatch().node!,
      status: "in_progress",
      raw: {
        sessionUpdate: "tool_call",
        status: "in_progress"
      },
      toolStatus: "in_progress",
      locations: [{ path: "README.md" }],
      content: [
        {
          type: "diff",
          path: "README.md",
          oldText: "before",
          newText: "after"
        }
      ]
    } as Extract<NonNullable<RenderPatch["node"]>, { kind: "tool" }>
  };
  const second: RenderPatch = {
    type: "upsert",
    node: {
      ...toolPatch().node!,
      status: "in_progress",
      raw: {
        sessionUpdate: "tool_call_update",
        status: "in_progress"
      },
      title: "Tool call",
      toolKind: "other",
      toolStatus: "in_progress",
      locations: [],
      content: [
        {
          type: "terminal",
          terminalId: "term-readme",
          command: "cat README.md",
          stdout: "after"
        }
      ]
    } as Extract<NonNullable<RenderPatch["node"]>, { kind: "tool" }>
  };

  scheduler.push([first]);
  scheduler.push([second]);
  scheduler.flush();

  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "tool");
  if (node?.kind !== "tool") {
    throw new Error("expected tool patch");
  }
  assert.deepEqual(node.content.map((item) => item.type), ["diff", "terminal"]);
  assert.deepEqual(node.locations.map((location) => location.path), ["README.md"]);
  assert.equal(node.title, "Read README.md");
  assert.equal(node.toolKind, "read");
});

test("coalesced tool patches append terminal content stdout deltas", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), {
    componentFlushMs: 10_000,
    shouldCoalescePatch: (patch) => patch.node?.kind === "tool"
  });
  const first: RenderPatch = {
    type: "upsert",
    node: {
      ...toolPatch().node!,
      status: "in_progress",
      raw: {
        sessionUpdate: "tool_call",
        status: "in_progress"
      },
      toolStatus: "in_progress",
      content: [
        {
          type: "terminal",
          terminalId: "term-readme",
          command: "cat README.md",
          stdout: "line 1\n"
        }
      ]
    } as Extract<NonNullable<RenderPatch["node"]>, { kind: "tool" }>
  };
  const second: RenderPatch = {
    type: "upsert",
    node: {
      ...toolPatch().node!,
      status: "in_progress",
      raw: {
        sessionUpdate: "tool_call_update",
        status: "in_progress"
      },
      toolStatus: "in_progress",
      content: [
        {
          type: "terminal",
          terminalId: "term-readme",
          stdoutDelta: "line 2\n"
        } as unknown as Extract<NonNullable<RenderPatch["node"]>, { kind: "tool" }>["content"][number]
      ]
    } as Extract<NonNullable<RenderPatch["node"]>, { kind: "tool" }>
  };

  scheduler.push([first]);
  scheduler.push([second]);
  scheduler.flush();

  const node = batches[0]?.[0]?.node;
  assert.equal(node?.kind, "tool");
  if (node?.kind !== "tool") {
    throw new Error("expected tool patch");
  }
  const terminal = node.content[0] as Record<string, unknown> | undefined;
  assert.equal(terminal?.command, "cat README.md");
  assert.equal(terminal?.stdout, "line 1\nline 2\n");
});

test("dispose drops queued stream patches without emitting stale output", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([messagePatch("stale")]);
  scheduler.dispose();
  scheduler.flush();

  assert.deepEqual(batches, []);
});

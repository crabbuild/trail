import assert from "node:assert/strict";
import test from "node:test";
import { RenderStreamScheduler } from "../shared/renderStreamScheduler";
import type { RenderPatch } from "../shared/renderModel";

function messagePatch(text: string, id = "message:assistant:msg-1"): RenderPatch {
  return {
    type: "upsert",
    node: {
      id,
      kind: "message",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-1",
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
      toolCallId: "read",
      title: "Read README.md",
      toolKind: "read",
      toolStatus: "completed",
      locations: [{ path: "README.md" }],
      content: []
    }
  };
}

function terminalPatch(output: string): RenderPatch {
  return {
    type: "upsert",
    node: {
      id: "terminal:read",
      kind: "terminal",
      taskId: "task-1",
      lane: "lane-1",
      turnId: "turn-1",
      acpSessionId: "session-1",
      acpToolCallId: "read",
      provider: "test",
      source: "acp-live",
      status: "in_progress",
      terminalId: "read",
      title: "Read README.md",
      stdout: output
    }
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

test("flushes queued stream patches before ordering-sensitive patches", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([messagePatch("Before tool")]);
  scheduler.push([toolPatch()]);

  assert.equal(batches.length, 2);
  assert.equal(batches[0]?.[0]?.node?.kind, "message");
  assert.equal(batches[1]?.[0]?.node?.kind, "tool");
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

test("keeps component coalescing independent from token flushes", async () => {
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
  assert.equal(batches[0]?.[0]?.node?.kind, "message");

  scheduler.flush();
  assert.equal(batches.length, 2);
  assert.equal(batches[1]?.[0]?.node?.kind, "terminal");
});

test("dispose drops queued stream patches without emitting stale output", () => {
  const batches: RenderPatch[][] = [];
  const scheduler = new RenderStreamScheduler((patches) => batches.push(patches), { flushMs: 10_000 });

  scheduler.push([messagePatch("stale")]);
  scheduler.dispose();
  scheduler.flush();

  assert.deepEqual(batches, []);
});

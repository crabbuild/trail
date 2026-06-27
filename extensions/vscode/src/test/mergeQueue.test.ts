import assert from "node:assert/strict";
import test from "node:test";
import { normalizeMergeQueueList } from "../shared/mergeQueue";

test("normalizes daemon merge queue entries", () => {
  const entries = normalizeMergeQueueList([
    {
      queue_id: "mq_1",
      source_ref: "refs/lanes/agent-a",
      target_ref: "main",
      status: "queued",
      priority: 10,
      created_at: 1782570000,
      updated_at: 1782570100
    }
  ]);

  assert.equal(entries.length, 1);
  assert.equal(entries[0]?.id, "mq_1");
  assert.equal(entries[0]?.sourceRef, "refs/lanes/agent-a");
  assert.equal(entries[0]?.targetRef, "main");
  assert.equal(entries[0]?.status, "queued");
  assert.equal(entries[0]?.priority, 10);
  assert.equal(entries[0]?.createdAt, 1782570000);
});

test("normalizes wrapped merge queue payloads", () => {
  const entries = normalizeMergeQueueList({
    entries: [
      {
        id: "queue-custom",
        source: "agent-b",
        target: "release",
        status: "conflicted"
      }
    ]
  });

  assert.equal(entries.length, 1);
  assert.equal(entries[0]?.id, "queue-custom");
  assert.equal(entries[0]?.sourceRef, "agent-b");
  assert.equal(entries[0]?.targetRef, "release");
  assert.equal(entries[0]?.status, "conflicted");
  assert.equal(entries[0]?.priority, 0);
});

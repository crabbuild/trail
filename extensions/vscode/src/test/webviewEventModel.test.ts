import assert from "node:assert/strict";
import test from "node:test";
import { buildEventPresentation } from "../webview/eventModel";

test("presents checkpoints as durable CrabDB recovery points", () => {
  const event = buildEventPresentation({
    kind: "checkpoint",
    label: "Checkpoint ch_123",
    checkpointId: "ch_123",
    updatedAt: "10:45 AM"
  });

  assert.equal(event.title, "Checkpoint saved");
  assert.equal(event.tone, "success");
  assert.equal(event.statusLabel, "Durable");
  assert.equal(event.detail, "ch_123 can start follow-ups or restore this lane.");
  assert.equal(event.facts.find((fact) => fact.label === "Checkpoint")?.active, true);
  assert.equal(event.facts.find((fact) => fact.label === "Recovery")?.value, "follow-up / rewind");
  assert.equal(event.callout?.title, "Durable recovery point");
  assert.match(event.callout?.detail ?? "", /follow-up starts, rewind/);
  assert.deepEqual(
    event.actions?.map((action) => action.action),
    ["copyCheckpoint", "startFollowUp", "rewind"]
  );
  assert.equal(event.actions?.find((action) => action.action === "copyCheckpoint")?.target, "ch_123");
  assert.equal(event.actions?.find((action) => action.action === "rewind")?.target, "ch_123");
});

test("keeps long checkpoint ids readable without losing the full reference", () => {
  const checkpointId = "ch_0123456789abcdef0123456789abcdef";
  const event = buildEventPresentation({
    kind: "checkpoint",
    checkpointId
  });

  assert.match(event.detail, /^ch_0123456789a\.\.\./);
  assert.equal(event.facts.find((fact) => fact.label === "Checkpoint")?.value, checkpointId);
  assert.equal(event.actions?.find((action) => action.action === "rewind")?.target, checkpointId);
});

test("opens failed completion events by default", () => {
  const event = buildEventPresentation({
    kind: "completion",
    status: "failed",
    stopReason: "max_tokens",
    label: "Stopped after reaching token limit"
  });

  assert.equal(event.title, "Turn failed");
  assert.equal(event.tone, "risk");
  assert.equal(event.openByDefault, true);
  assert.equal(event.facts.find((fact) => fact.label === "Stop")?.value, "max_tokens");
});

test("marks pending completions as checkpoint waits", () => {
  const event = buildEventPresentation({
    kind: "completion",
    status: "pending",
    checkpointPending: true
  });

  assert.equal(event.title, "Turn finishing");
  assert.equal(event.tone, "warning");
  assert.equal(event.facts.find((fact) => fact.label === "Checkpoint")?.value, "pending");
});

test("classifies high context usage as risk", () => {
  const event = buildEventPresentation({
    kind: "usage",
    used: 95,
    size: 100,
    costLabel: "$0.25"
  });

  assert.equal(event.tone, "risk");
  assert.equal(event.statusLabel, "95%");
  assert.equal(event.openByDefault, true);
  assert.equal(event.facts.find((fact) => fact.label === "Cost")?.value, "$0.25");
});

test("includes session and unknown event facts", () => {
  const session = buildEventPresentation({
    kind: "session",
    sessionId: "session-123",
    sessionTitle: "Docs task",
    updatedAt: "11:00 AM"
  });
  const unknown = buildEventPresentation({ kind: "unknown", label: "Provider event" });

  assert.equal(session.facts.find((fact) => fact.label === "Session")?.active, true);
  assert.equal(unknown.tone, "warning");
  assert.equal(unknown.openByDefault, true);
});

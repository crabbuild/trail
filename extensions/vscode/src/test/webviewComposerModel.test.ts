import assert from "node:assert/strict";
import test from "node:test";
import {
  attachmentModeSummary,
  composerDraftState,
  composerMetrics,
  composerRailItems,
  composerSendBlockedReason,
  MAX_COMPOSER_DRAFT_CHARS
} from "../webview/composerModel";

test("blocks empty composer sends until there is prompt text or context", () => {
  assert.equal(
    composerSendBlockedReason({
      hasDraft: false,
      attachmentCount: 0
    }),
    "Write a message or attach context before sending."
  );

  assert.equal(
    composerSendBlockedReason({
      hasDraft: true,
      attachmentCount: 0
    }),
    undefined
  );

  assert.equal(
    composerSendBlockedReason({
      hasDraft: false,
      attachmentCount: 1
    }),
    undefined
  );
});

test("prioritizes running and permission composer blockers", () => {
  assert.equal(
    composerSendBlockedReason({
      hasDraft: true,
      attachmentCount: 2,
      sending: true
    }),
    "The current prompt is still running."
  );

  assert.equal(
    composerSendBlockedReason({
      hasDraft: true,
      attachmentCount: 2,
      sending: true,
      permissionPending: true
    }),
    "Resolve the permission request before sending."
  );
});

test("blocks prompt sends at the composer limit", () => {
  assert.equal(
    composerSendBlockedReason({
      hasDraft: true,
      attachmentCount: 0,
      draftChars: 10,
      maxChars: 10
    }),
    "Shorten the prompt or move bulky context into attachments before sending."
  );

  assert.equal(
    composerSendBlockedReason({
      hasDraft: true,
      attachmentCount: 0,
      draftChars: Array.from("hello 🌊").length,
      maxChars: 8
    }),
    undefined
  );

  assert.equal(
    composerSendBlockedReason({
      hasDraft: false,
      attachmentCount: 1,
      draftChars: 0,
      maxChars: 1
    }),
    undefined
  );
});

test("keeps permission and running blockers ahead of draft limit blockers", () => {
  assert.equal(
    composerSendBlockedReason({
      hasDraft: true,
      attachmentCount: 0,
      draftChars: 10,
      maxChars: 10,
      sending: true
    }),
    "The current prompt is still running."
  );

  assert.equal(
    composerSendBlockedReason({
      hasDraft: true,
      attachmentCount: 0,
      draftChars: 10,
      maxChars: 10,
      permissionPending: true
    }),
    "Resolve the permission request before sending."
  );
});

test("formats composer metrics with unicode-aware character counts", () => {
  assert.equal(composerMetrics("hi\n世界", 2, 10), "2 attachments - 5 chars - 2 lines - 5 left");
  assert.equal(composerMetrics("", 0, MAX_COMPOSER_DRAFT_CHARS), "0 attachments - 0 chars - 0 lines - 120,000 left");
});

test("models composer draft frame states for empty, context-only, and long prompts", () => {
  const empty = composerDraftState("", 0, 10);
  const contextOnly = composerDraftState("", 2, 10);
  const warning = composerDraftState("123456789", 0, 10);
  const limit = composerDraftState("1234567890!", 0, 10);

  assert.equal(empty.tone, "empty");
  assert.equal(empty.label, "Empty prompt");
  assert.equal(contextOnly.label, "Context-only prompt");
  assert.equal(warning.tone, "warning");
  assert.equal(warning.remaining, 1);
  assert.equal(warning.meterPercent, 90);
  assert.equal(limit.tone, "limit");
  assert.equal(limit.remaining, 0);
  assert.equal(limit.meterValue, 10);
  assert.equal(limit.meterPercent, 100);
});

test("summarizes attachment modes in encounter order", () => {
  assert.equal(attachmentModeSummary([]), "No context");
  assert.equal(attachmentModeSummary(["inline", "text", "inline", "link"]), "2 inline, 1 text, 1 link");
});

test("builds prompt context rail items for durable fast prompts", () => {
  const items = composerRailItems({
    statusTone: "context",
    statusLabel: "Context ready",
    attachmentModes: ["inline", "inline"],
    sendMode: "fast",
    providerCrabdbBacked: true
  });

  assert.deepEqual(
    items.map((item) => [item.id, item.value, item.tone]),
    [
      ["state", "Context ready", "ready"],
      ["context", "2 inline", "ready"],
      ["send", "Enter sends", "active"],
      ["route", "Trail route", "ready"]
    ]
  );
});

test("marks draft send mode and raw provider route in the prompt context rail", () => {
  const items = composerRailItems({
    statusTone: "waiting",
    statusLabel: "Permission required",
    attachmentModes: [],
    sendMode: "draft",
    providerCrabdbBacked: false
  });

  assert.equal(items.find((item) => item.id === "state")?.tone, "blocked");
  assert.equal(items.find((item) => item.id === "context")?.value, "No context");
  assert.equal(items.find((item) => item.id === "send")?.value, "Enter newline");
  assert.equal(items.find((item) => item.id === "route")?.tone, "warning");
});

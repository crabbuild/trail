import assert from "node:assert/strict";
import test from "node:test";
import { buildToolbarModel } from "../webview/toolbarModel";

function toolbarInput(overrides: Partial<Parameters<typeof buildToolbarModel>[0]> = {}) {
  return {
    taskStatus: "ready",
    lane: "agent-1",
    changedPaths: 1,
    providerLabel: "Claude Code via CrabDB",
    providerCrabdbBacked: true,
    sending: false,
    permissionPending: false,
    providerFailure: false,
    supportsFromRef: true,
    reviewVisible: false,
    sessionLabel: "Loaded session",
    sessionTone: "ready",
    acpSessionId: "session-1",
    modeLabel: "Plan",
    configCount: 2,
    commandCount: 3,
    capabilities: {
      embeddedContext: true,
      image: true,
      audio: false
    },
    coordinationLabels: [],
    coordinationSeverity: "ok",
    ...overrides
  };
}

test("promotes dry-run apply for ready changed tasks", () => {
  const model = buildToolbarModel(toolbarInput());

  assert.equal(model.runState.label, "Ready for dry-run");
  assert.equal(model.primaryAction.action, "dryRunApply");
  assert.equal(model.statusChips.find((chip) => chip.id === "provider")?.tone, "ok");
  assert.equal(model.capabilitySummary, "8/9 ready");
});

test("turns running state into a cancel action", () => {
  const model = buildToolbarModel(toolbarInput({ sending: true }));

  assert.equal(model.runState.tone, "active");
  assert.equal(model.primaryAction.action, "cancel");
  assert.equal(model.primaryAction.tone, "danger");
});

test("routes pending approvals back to the transcript", () => {
  const model = buildToolbarModel(toolbarInput({ permissionPending: true }));

  assert.equal(model.runState.label, "Permission required");
  assert.equal(model.primaryAction.action, "focusTranscript");
});

test("surfaces raw providers and coordination warnings in toolbar chips", () => {
  const model = buildToolbarModel(
    toolbarInput({
      providerCrabdbBacked: false,
      coordinationSeverity: "warning",
      coordinationLabels: ["missing test"]
    })
  );

  assert.equal(model.runState.tone, "warning");
  assert.equal(model.primaryAction.action, "focusReview");
  assert.equal(model.statusChips.find((chip) => chip.id === "provider")?.tone, "warning");
  assert.equal(model.statusChips.some((chip) => chip.value === "missing test"), true);
});

test("keeps long toolbar chips bounded while preserving full labels", () => {
  const longLane = "agent-claude-code-very-long-ui-hardening-lane-with-review-settings-and-diff-polish";
  const nextAction = "Dry-run apply before changing the main workspace and then inspect every review gate";
  const model = buildToolbarModel(toolbarInput({ lane: longLane, nextAction }));
  const laneChip = model.statusChips.find((chip) => chip.id === "lane");
  const nextChip = model.statusChips.find((chip) => chip.id === "next");

  assert.equal(laneChip?.value, longLane);
  assert.match(laneChip?.displayValue ?? "", /\.\.\./);
  assert.equal(laneChip?.accessibilityLabel, `Lane: ${longLane}`);
  assert.equal(nextChip?.value, nextAction);
  assert.match(nextChip?.displayValue ?? "", /^Dry-run apply/);
  assert.match(nextChip?.displayValue ?? "", /\.\.\./);
  assert.equal(nextChip?.accessibilityLabel, `Next: ${nextAction}`);
});

test("focuses the composer for empty idle tasks", () => {
  const model = buildToolbarModel(
    toolbarInput({
      taskStatus: "new",
      changedPaths: 0,
      modeLabel: undefined,
      configCount: 0,
      commandCount: 0,
      acpSessionId: undefined,
      sessionLabel: undefined,
      capabilities: undefined
    })
  );

  assert.equal(model.runState.label, "Ready");
  assert.equal(model.primaryAction.action, "focusComposer");
  assert.equal(model.capabilities.find((capability) => capability.id === "commands")?.enabled, false);
});

test("summarizes CrabDB workflow capabilities separately from prompt input", () => {
  const model = buildToolbarModel(toolbarInput());

  assert.equal(model.capabilities.find((capability) => capability.id === "durable-state")?.group, "workflow");
  assert.equal(model.capabilities.find((capability) => capability.id === "durable-state")?.enabled, true);
  assert.equal(model.capabilities.find((capability) => capability.id === "checkpoint-start")?.enabled, true);
  assert.equal(model.capabilities.find((capability) => capability.id === "inline")?.group, "input");
});

test("marks raw provider workflow capabilities unavailable", () => {
  const model = buildToolbarModel(
    toolbarInput({
      providerCrabdbBacked: false,
      supportsFromRef: false,
      commandCount: 0
    })
  );

  assert.equal(model.capabilities.find((capability) => capability.id === "durable-state")?.enabled, false);
  assert.equal(model.capabilities.find((capability) => capability.id === "review-gates")?.enabled, false);
  assert.equal(model.capabilities.find((capability) => capability.id === "checkpoint-start")?.enabled, false);
  assert.match(model.capabilities.find((capability) => capability.id === "durable-state")?.detail || "", /Raw provider/);
});

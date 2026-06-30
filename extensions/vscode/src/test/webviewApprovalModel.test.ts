import assert from "node:assert/strict";
import test from "node:test";
import {
  approvalActionLabel,
  approvalDecisionDescription,
  approvalDecisionTone,
  approvalImpactText,
  approvalScopeLabel,
  approvalStateLabel,
  approvalTone
} from "../webview/approvalModel";

test("labels approval states in user-facing language", () => {
  assert.equal(approvalStateLabel("pending"), "Needs decision");
  assert.equal(approvalStateLabel("completed"), "Approved");
  assert.equal(approvalStateLabel("cancelled"), "Rejected");
  assert.equal(approvalStateLabel("failed"), "Failed");
});

test("assigns approval tones from status and tool risk", () => {
  assert.equal(approvalTone({ status: "completed", toolKind: "execute" }), "success");
  assert.equal(approvalTone({ status: "cancelled", toolKind: "read" }), "risk");
  assert.equal(approvalTone({ status: "pending", toolKind: "execute" }), "risk");
  assert.equal(approvalTone({ status: "pending", toolKind: "edit" }), "warning");
  assert.equal(approvalTone({ status: "pending", toolKind: "read" }), "info");
});

test("summarizes approval scope and impact", () => {
  assert.equal(approvalScopeLabel(1, "lane-a"), "1 affected location");
  assert.equal(approvalScopeLabel(3, "lane-a"), "3 affected locations");
  assert.equal(approvalScopeLabel(0, "lane-a"), "Lane lane-a");
  assert.equal(approvalImpactText("edit", 2), "Edit 2 affected locations.");
  assert.equal(approvalImpactText("execute", 0), "Run a command in the current task.");
});

test("classifies approval decision buttons by tool risk", () => {
  assert.equal(approvalDecisionTone({ status: "pending", toolKind: "read" }), "primary");
  assert.match(approvalDecisionDescription("read"), /read-only/);
  assert.equal(approvalDecisionTone({ status: "pending", toolKind: "execute" }), "warning");
  assert.match(approvalDecisionDescription("execute"), /reviewing risk/);
  assert.equal(approvalDecisionTone({ status: "pending", toolKind: "delete" }), "risk");
  assert.match(approvalDecisionDescription("delete"), /destructive target/);
});

test("returns neutral decision tone after approval resolution", () => {
  assert.equal(approvalDecisionTone({ status: "completed", toolKind: "edit" }), "default");
  assert.equal(approvalDecisionDescription("unknown"), "Allow provider tool action.");
});

test("keeps approval action labels compact", () => {
  assert.equal(approvalActionLabel("Always Allow Read(/Users/example/project/README.md)", "always-allow"), "Always allow");
  assert.equal(approvalActionLabel("Allow once", "allow"), "Allow");
  assert.equal(approvalActionLabel("Run command after review", "run"), "Run command after review");
  assert.equal(approvalActionLabel("This label is too long for a compact approval button", "custom"), "This label is too long fo...");
});

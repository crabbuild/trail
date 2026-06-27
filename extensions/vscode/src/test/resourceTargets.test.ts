import assert from "node:assert/strict";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import { classifyResourceTarget, isPathInsideWorkspace } from "../shared/resourceTargets";

const workspaceRoot = path.join(os.tmpdir(), "crabdb-resource-targets", "workspace");

test("classifies relative workspace paths as workspace files", () => {
  const target = classifyResourceTarget("src/index.ts", workspaceRoot);

  assert.equal(target.kind, "workspace-file");
  assert.equal(target.path, path.join(workspaceRoot, "src/index.ts"));
});

test("classifies file URIs inside the workspace as workspace files", () => {
  const uri = pathToFileURL(path.join(workspaceRoot, "README.md")).toString();
  const target = classifyResourceTarget(uri, workspaceRoot);

  assert.equal(target.kind, "workspace-file");
  assert.equal(target.path, path.join(workspaceRoot, "README.md"));
});

test("classifies paths outside the workspace as external files", () => {
  const target = classifyResourceTarget("../outside.txt", workspaceRoot);

  assert.equal(target.kind, "external-file");
  assert.equal(isPathInsideWorkspace(target.path, workspaceRoot), false);
});

test("classifies http URLs as external URIs", () => {
  const target = classifyResourceTarget("https://example.com/report", workspaceRoot);

  assert.equal(target.kind, "external-uri");
  assert.equal(target.scheme, "https");
});

test("rejects unsupported URI schemes", () => {
  const target = classifyResourceTarget("vscode://file/workspace/readme", workspaceRoot);

  assert.equal(target.kind, "unsupported-uri");
  assert.equal(target.scheme, "vscode");
});

test("rejects empty targets", () => {
  const target = classifyResourceTarget("  ", workspaceRoot);

  assert.equal(target.kind, "invalid");
});

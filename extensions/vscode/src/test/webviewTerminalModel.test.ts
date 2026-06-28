import assert from "node:assert/strict";
import test from "node:test";
import { buildTerminalPresentation, terminalCommand, terminalStatusLabel } from "../webview/terminalModel";

test("summarizes successful terminal commands", () => {
  const model = buildTerminalPresentation({
    title: "Run tests",
    command: ["npm", "test"],
    cwd: "/repo",
    status: "completed",
    exitCode: 0,
    elapsedMs: 1250,
    stdout: "ok\npassed\n"
  });

  assert.equal(model.title, "Run tests");
  assert.equal(model.command, "npm test");
  assert.equal(model.cwd, "/repo");
  assert.equal(model.tone, "ok");
  assert.equal(model.statusLabel, "passed");
  assert.equal(model.metrics.some((metric) => metric.value === "1.3 s"), true);
  assert.equal(model.sections[0]?.label, "Stdout");
  assert.equal(model.sections[0]?.openByDefault, true);
});

test("marks non-zero exits and stderr as inspectable risk", () => {
  const model = buildTerminalPresentation({
    commandLine: "npm test",
    exit_code: 1,
    stderr: "failed hard"
  });

  assert.equal(model.tone, "risk");
  assert.equal(model.statusLabel, "exit 1");
  assert.equal(model.openByDefault, true);
  assert.equal(model.sections.find((section) => section.id === "stderr")?.openByDefault, true);
});

test("redacts and truncates terminal output sections", () => {
  const model = buildTerminalPresentation(
    {
      command: "curl https://example.com",
      output: `token=secret\n${"x".repeat(80)}`
    },
    30
  );

  const output = model.sections.find((section) => section.id === "output");
  assert.ok(output);
  assert.equal(output.truncated, true);
  assert.doesNotMatch(output.text, /secret/);
  assert.match(output.text, /\[truncated\]/);
});

test("normalizes command and status labels from variant fields", () => {
  assert.equal(terminalCommand({ command: ["git", "status", "--short"] }), "git status --short");
  assert.equal(terminalCommand({ command_line: "pnpm build" }), "pnpm build");
  assert.equal(terminalCommand({ cmd: "git remote -v" }), "git remote -v");
  assert.equal(terminalCommand({ bash_command: "find . -type f" }), "find . -type f");
  assert.equal(terminalCommand({ executable: "git", args: ["remote", "-v"] }), "git remote -v");
  assert.equal(terminalStatusLabel("in_progress", undefined), "running");
  assert.equal(terminalStatusLabel("completed", undefined), "passed");
});

test("keeps empty terminal previews directional", () => {
  const model = buildTerminalPresentation({
    terminalId: "term-1",
    status: "pending"
  });

  assert.equal(model.sections.length, 0);
  assert.equal(model.emptyText, "No terminal output preview is available.");
  assert.equal(model.tone, "warning");
});

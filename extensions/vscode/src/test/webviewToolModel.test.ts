import assert from "node:assert/strict";
import test from "node:test";
import { buildToolPresentation } from "../webview/toolModel";

test("classifies read tools as compact read-only operations", () => {
  const model = buildToolPresentation({
    title: "Read package.json",
    toolKind: "read",
    toolStatus: "completed",
    locations: [{ path: "package.json" }],
    content: [{ type: "content", content: { type: "text", text: "{}" } }]
  });

  assert.equal(model.operationLabel, "Read");
  assert.equal(model.kind, "read");
  assert.equal(model.summary, "package.json");
  assert.equal(model.riskTone, "ok");
  assert.equal(model.riskLabel, "Read-only");
  assert.equal(model.openByDefault, false);
  assert.equal(model.stats.some((stat) => stat.label === "location"), true);
  assert.deepEqual(
    model.actions.map((action) => action.kind),
    ["openLocation"]
  );
});

test("keeps completed edit diffs collapsed while marking them as workspace changes", () => {
  const model = buildToolPresentation({
    title: "Edit README.md",
    toolKind: "edit",
    toolStatus: "completed",
    locations: [{ path: "README.md" }],
    rawInput: { path: "README.md", oldString: "old" },
    content: [{ type: "diff", path: "README.md", oldText: "old", newText: "new" }]
  });

  assert.equal(model.riskTone, "warning");
  assert.equal(model.riskLabel, "Workspace change");
  assert.equal(model.openByDefault, false);
  assert.equal(model.summary, "1 diff in README.md");
  assert.equal(model.actions[0]?.kind, "focusDiff");
  assert.equal(model.actions[0]?.tone, "primary");
  assert.deepEqual(
    model.actions.map((action) => action.kind),
    ["focusDiff", "openLocation"]
  );
  assert.equal(model.actions.find((action) => action.kind === "openLocation")?.path, "README.md");
  assert.equal(model.stats.some((stat) => stat.label === "state"), false);
});

test("describes edit tools without previews as lifecycle work", () => {
  const running = buildToolPresentation({
    title: "Edit",
    toolKind: "edit",
    toolStatus: "in_progress",
    locations: [],
    content: []
  });
  const completed = buildToolPresentation({
    title: "Edit",
    toolKind: "edit",
    toolStatus: "completed",
    locations: [],
    content: []
  });
  const failed = buildToolPresentation({
    title: "Edit",
    toolKind: "edit",
    toolStatus: "failed",
    locations: [],
    rawInput: { path: "README.md" },
    content: []
  });

  assert.equal(running.summary, "Preparing workspace change");
  assert.equal(completed.summary, "Workspace change");
  assert.equal(failed.summary, "README.md");
  assert.equal(running.emptyText, "No diff preview available for this edit.");
  assert.deepEqual(
    failed.actions.map((action) => action.kind),
    ["openLocation"]
  );
});

test("keeps command tools focused on rendered terminal output", () => {
  const model = buildToolPresentation({
    title: "Run tests",
    toolKind: "execute",
    toolStatus: "completed",
    locations: [],
    rawInput: { command: ["npm", "test"], cwd: "/repo" },
    content: [{ type: "terminal", terminalId: "term-1", command: ["npm", "test"], stdout: "ok" }]
  });

  assert.equal(model.riskTone, "warning");
  assert.equal(model.riskLabel, "Command");
  assert.equal(model.openByDefault, false);
  assert.equal(model.summary, "npm test");
  assert.equal(model.facts.find((fact) => fact.label === "Command")?.value, "npm test");
  assert.equal(model.actions.length, 0);
});

test("keeps think tools focused on the note content", () => {
  const model = buildToolPresentation({
    title: "Find extension API surface",
    toolKind: "think",
    toolStatus: "pending",
    locations: [],
    rawInput: { prompt: "Find exported API surface" },
    rawOutput: { ok: true },
    content: [{ type: "content", content: { type: "text", text: "Search extension entry points." } }]
  });

  assert.equal(model.operationLabel, "Think");
  assert.equal(model.kind, "think");
  assert.equal(model.summary, "Thinking");
  assert.equal(model.riskTone, "ok");
  assert.equal(model.stats.length, 0);
  assert.equal(model.actions.length, 0);
});

test("keeps failed and destructive tools collapsed without details actions", () => {
  const failed = buildToolPresentation({
    title: "Delete generated file",
    toolKind: "delete",
    toolStatus: "failed",
    locations: [{ path: "tmp/generated.txt" }],
    content: []
  });

  assert.equal(failed.riskTone, "risk");
  assert.equal(failed.riskLabel, "Needs inspection");
  assert.equal(failed.openByDefault, false);
  assert.equal(failed.statusLabel, "failed");
  assert.deepEqual(
    failed.actions.map((action) => action.kind),
    ["openLocation"]
  );
});

test("uses CrabDB-specific empty text for persisted tool events", () => {
  const model = buildToolPresentation({
    title: "Persisted read",
    toolKind: "read",
    toolStatus: "completed",
    locations: [],
    content: [],
    source: "crabdb"
  });

  assert.match(model.emptyText, /CrabDB persisted/);
  assert.equal(model.actions.length, 0);
});

test("redacts and truncates raw input facts", () => {
  const model = buildToolPresentation({
    title: "Fetch secret",
    toolKind: "fetch",
    toolStatus: "completed",
    locations: [],
    rawInput: {
      url: `https://example.com/${"x".repeat(160)}?token=secret`
    },
    content: []
  });

  const fact = model.facts.find((item) => item.label === "Resource");
  assert.ok(fact);
  assert.ok(fact.value.length <= 110);
  assert.doesNotMatch(fact.value, /secret/);
  assert.equal(model.actions.length, 0);
});

test("redacts command summaries without generic copy affordances", () => {
  const model = buildToolPresentation({
    title: "Run deployment",
    toolKind: "execute",
    toolStatus: "completed",
    locations: [],
    rawInput: {
      command: "deploy --token secret --env prod"
    },
    content: []
  });

  assert.equal(model.actions.length, 0);
  assert.doesNotMatch(model.summary, /secret/);
  assert.match(model.summary, /\[REDACTED\]/);
  assert.match(model.facts.find((fact) => fact.label === "Command")?.value || "", /\[REDACTED\]/);
});

test("infers command presentation for generic tool payloads", () => {
  const model = buildToolPresentation({
    title: "Bash",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    rawInput: { command: "npm run lint" },
    content: []
  });

  assert.equal(model.operationLabel, "Run");
  assert.equal(model.summary, "npm run lint");
  assert.equal(model.icon, "terminal");
  assert.equal(model.tone, "risk");
  assert.equal(model.riskLabel, "Command");
  assert.equal(model.actions.length, 0);
});

test("infers edit presentation for generic diff blocks", () => {
  const model = buildToolPresentation({
    title: "tool_call_update",
    toolKind: "other",
    toolStatus: "completed",
    locations: [{ path: "src/index.ts" }],
    content: [{ type: "diff", path: "src/index.ts", oldText: "a", newText: "b" }]
  });

  assert.equal(model.operationLabel, "Edit");
  assert.equal(model.riskLabel, "Workspace change");
  assert.equal(model.openByDefault, false);
  assert.equal(model.summary, "1 diff in src/index.ts");
  assert.equal(model.actions[0]?.kind, "focusDiff");
});

test("infers fetch and search presentation for generic data lookups", () => {
  const fetch = buildToolPresentation({
    title: "Open resource",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    rawInput: { url: "https://example.com/docs" },
    content: []
  });
  const search = buildToolPresentation({
    title: "Provider search",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    rawInput: { pattern: "buildToolPresentation" },
    content: []
  });

  assert.equal(fetch.operationLabel, "Fetch");
  assert.equal(fetch.icon, "open");
  assert.equal(fetch.riskLabel, "Read-only");
  assert.equal(search.operationLabel, "Search");
  assert.equal(search.icon, "search");
  assert.equal(search.tone, "query");
});

test("infers read presentation for generic file path events", () => {
  const model = buildToolPresentation({
    title: "Read File",
    toolKind: "other",
    toolStatus: "completed",
    locations: [{ path: "README.md" }],
    rawInput: { path: "README.md" },
    content: []
  });

  assert.equal(model.operationLabel, "Read");
  assert.equal(model.icon, "file");
  assert.equal(model.riskTone, "ok");
  assert.equal(model.actions[0]?.kind, "openLocation");
});

test("infers provider-shaped generic tool names before rendering", () => {
  const bash = buildToolPresentation({
    title: "Bash",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    content: []
  });
  const read = buildToolPresentation({
    title: "Read src/core/AgentManager.ts (1 - 40)",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    content: [{ type: "content", content: { type: "text", text: "export class AgentManager {}" } }]
  });
  const patch = buildToolPresentation({
    title: "tool_call_update",
    toolKind: "other",
    toolStatus: "completed",
    locations: [{ path: "src/index.ts" }],
    rawInput: { toolName: "apply_patch" },
    content: []
  });
  const grep = buildToolPresentation({
    title: "Search",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    rawInput: { name: "grep", pattern: "buildToolPresentation" },
    content: []
  });

  assert.equal(bash.kind, "execute");
  assert.equal(bash.operationLabel, "Run");
  assert.equal(bash.riskLabel, "Command");
  assert.equal(read.kind, "read");
  assert.equal(read.operationLabel, "Read");
  assert.equal(read.riskLabel, "Read-only");
  assert.equal(patch.kind, "edit");
  assert.equal(patch.operationLabel, "Edit");
  assert.equal(patch.riskLabel, "Workspace change");
  assert.equal(grep.kind, "search");
  assert.equal(grep.operationLabel, "Search");
});

test("normalizes wrapped provider command arguments before rendering", () => {
  const model = buildToolPresentation({
    title: "tool_call",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    rawInput: {
      name: "call_tool",
      arguments: JSON.stringify({
        command: "npm test --token secret --workspace crabdb",
        cwd: "/repo"
      })
    },
    content: []
  });

  assert.equal(model.kind, "execute");
  assert.equal(model.operationLabel, "Run");
  assert.match(model.summary, /\[REDACTED\]/);
  assert.doesNotMatch(model.summary, /secret/);
  assert.equal(model.facts.find((fact) => fact.label === "Cwd")?.value, "/repo");
  assert.equal(model.actions.length, 0);
});

test("uses nested provider file arguments for summaries and open actions", () => {
  const model = buildToolPresentation({
    title: "tool_call",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    rawInput: {
      name: "call_tool",
      arguments: {
        toolName: "read_file",
        input: {
          path: "src/webview/toolModel.ts",
          line_number: "42"
        }
      }
    },
    content: []
  });

  const openAction = model.actions.find((action) => action.kind === "openLocation");
  assert.equal(model.kind, "read");
  assert.equal(model.operationLabel, "Read");
  assert.equal(model.summary, "src/webview/toolModel.ts");
  assert.equal(model.riskLabel, "Read-only");
  assert.deepEqual(
    model.stats.find((stat) => stat.label === "location"),
    { label: "location", value: "1", tone: "default" }
  );
  assert.equal(openAction?.path, "src/webview/toolModel.ts");
  assert.equal(openAction?.line, 42);
});

test("prefers nested concrete tool names over generic provider wrappers", () => {
  const model = buildToolPresentation({
    title: "tool_call_update",
    toolKind: "other",
    toolStatus: "completed",
    locations: [],
    rawInput: {
      name: "call_tool",
      arguments: {
        toolName: "apply_patch"
      }
    },
    content: []
  });

  assert.equal(model.kind, "edit");
  assert.equal(model.operationLabel, "Edit");
  assert.equal(model.riskLabel, "Workspace change");
});

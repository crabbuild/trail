import assert from "node:assert/strict";
import test from "node:test";
import { attachmentToContentBlock } from "../model/PromptAttachment";

test("turns a text selection attachment into an embedded ACP resource", () => {
  const block = attachmentToContentBlock({
    id: "att-1",
    kind: "selection",
    label: "src/main.ts:1-2",
    uri: "file:///repo/src/main.ts",
    mimeType: "text/x-typescript",
    text: "console.log('hi');"
  });

  assert.equal(block.type, "resource");
  assert.deepEqual(block.resource, {
    uri: "file:///repo/src/main.ts",
    mimeType: "text/x-typescript",
    text: "console.log('hi');"
  });
});

test("falls back to ACP text when embedded context is not supported", () => {
  const block = attachmentToContentBlock(
    {
      id: "att-1",
      kind: "selection",
      label: "src/main.ts:1-2",
      uri: "file:///repo/src/main.ts",
      mimeType: "text/x-typescript",
      text: "console.log('hi');"
    },
    {
      embeddedContext: false
    }
  );

  assert.equal(block.type, "text");
  if (block.type !== "text") {
    throw new Error("Expected text content block.");
  }
  assert.equal(typeof block.text, "string");
  const text = String(block.text);
  assert.match(text, /Context from src\/main\.ts:1-2/);
  assert.match(text, /console\.log\('hi'\);/);
});

test("turns a file attachment without text into a resource link", () => {
  const block = attachmentToContentBlock({
    id: "att-2",
    kind: "file",
    label: "README.md",
    uri: "file:///repo/README.md"
  });

  assert.equal(block.type, "resource_link");
  assert.equal(block.uri, "file:///repo/README.md");
  assert.equal(block.name, "README.md");
});

test("turns diagnostics attachment into embedded context", () => {
  const block = attachmentToContentBlock({
    id: "att-3",
    kind: "diagnostics",
    label: "Diagnostics for /repo/src/main.ts",
    uri: "file:///repo/src/main.ts",
    mimeType: "text/plain",
    text: "Error 12-12: Missing semicolon"
  });

  assert.equal(block.type, "resource");
  assert.deepEqual(block.resource, {
    uri: "file:///repo/src/main.ts",
    mimeType: "text/plain",
    text: "Error 12-12: Missing semicolon"
  });
});

test("turns terminal output attachment into text content", () => {
  const block = attachmentToContentBlock({
    id: "att-terminal",
    kind: "terminal-output",
    label: "Terminal output: npm test",
    mimeType: "text/plain",
    text: "Command: npm test\n\nStdout:\npass"
  });

  assert.equal(block.type, "text");
  if (block.type !== "text") {
    throw new Error("Expected text content block.");
  }
  assert.match(String(block.text), /npm test/);
  assert.match(String(block.text), /pass/);
});

test("turns changed-files attachment into text content", () => {
  const block = attachmentToContentBlock({
    id: "att-4",
    kind: "changed-files",
    label: "Changed files for lane-a",
    mimeType: "text/plain",
    text: "- README.md\n- src/main.ts"
  });

  assert.equal(block.type, "text");
  if (block.type !== "text") {
    throw new Error("Expected text content block.");
  }
  assert.match(String(block.text), /README\.md/);
});

test("turns history attachment into text when embedded context is unavailable", () => {
  const block = attachmentToContentBlock(
    {
      id: "att-5",
      kind: "history",
      label: "CrabDB history for README.md",
      uri: "file:///repo/README.md",
      mimeType: "application/json",
      text: "{\"history\":[]}"
    },
    {
      embeddedContext: false
    }
  );

  assert.equal(block.type, "text");
  if (block.type !== "text") {
    throw new Error("Expected text content block.");
  }
  assert.match(String(block.text), /CrabDB history for README\.md/);
});

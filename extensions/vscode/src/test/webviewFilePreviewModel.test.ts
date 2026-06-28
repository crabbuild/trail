import assert from "node:assert/strict";
import test from "node:test";
import { buildFilePreviewModel } from "../webview/filePreviewModel";

test("builds a Shiki-ready file preview summary", () => {
  const model = buildFilePreviewModel({
    path: "src/webview/main.ts",
    language: "typescript",
    text: "const label = '世界';\nconsole.log(label);\n",
    maxChars: 100
  });

  assert.equal(model.title, "src/webview/main.ts");
  assert.equal(model.language, "typescript");
  assert.equal(model.truncated, false);
  assert.equal(model.lineCount, 3);
  assert.equal(model.charCount, 40);
  assert.deepEqual(
    model.badges.map((badge) => badge.label),
    ["3 lines", "40 chars"]
  );
  assert.equal(model.metaLabel, "3 lines - 40 chars");
  assert.equal(model.highlightSupported, true);
  assert.equal(model.accessibilityLabel, "src/webview/main.ts, typescript, 3 lines");
});

test("marks long file previews as truncated without losing source counts", () => {
  const model = buildFilePreviewModel({
    path: "README.md",
    language: "markdown",
    text: "12345\n67890\nabcde",
    maxChars: 8
  });

  assert.equal(model.truncated, true);
  assert.equal(model.lineCount, 3);
  assert.equal(model.charCount, 17);
  assert.match(model.text, /\[truncated\]/);
  assert.equal(model.badges.at(-1)?.label, "Truncated at 8");
  assert.equal(model.badges.at(-1)?.tone, "warning");
});

test("marks unsupported preview languages as plain text", () => {
  const model = buildFilePreviewModel({
    path: "notes.txt",
    language: "plaintext",
    text: "hello",
    maxChars: 100
  });

  assert.equal(model.highlightSupported, false);
  assert.deepEqual(
    model.badges.map((badge) => badge.label),
    ["1 line", "5 chars"]
  );
});

import assert from "node:assert/strict";
import test from "node:test";
import { normalizeHighlightSource } from "../webview/highlightSourceModel";

test("strips read-tool line numbers before Shiki tokenization", () => {
  const source = normalizeHighlightSource(
    "20\t * Get the list of agent names available.\n21\t */\n22\texport function getAgentNames(): string[] {\n23\t  return Object.keys(getAgentConfigs());\n24\t}\n25"
  );

  assert.equal(source.lineStart, 20);
  assert.equal(
    source.text,
    " * Get the list of agent names available.\n */\nexport function getAgentNames(): string[] {\n  return Object.keys(getAgentConfigs());\n}\n"
  );
  assert.doesNotMatch(source.text, /^20\t/m);
});

test("keeps explicit source line starts when the text has no prefixed gutter", () => {
  const source = normalizeHighlightSource("export const answer = 42;", 12);

  assert.equal(source.lineStart, 12);
  assert.equal(source.text, "export const answer = 42;");
});

test("detects space-separated read-tool line numbers", () => {
  const source = normalizeHighlightSource("7 export const a = 1;\n8 export const b = 2;\n9 export const c = 3;");

  assert.equal(source.lineStart, 7);
  assert.equal(source.text, "export const a = 1;\nexport const b = 2;\nexport const c = 3;");
});

import assert from "node:assert/strict";
import test from "node:test";
import { textContentValue, textOnlyContent } from "../webview/contentTextModel";

test("extracts streaming text from aliased text content blocks", () => {
  assert.equal(
    textOnlyContent([
      { type: "text", content: "Reading " },
      { type: "text", value: "workspace " },
      { type: "text", text: "context" }
    ]),
    "Reading workspace context"
  );
});

test("prefers non-empty text aliases while preserving explicit empty text", () => {
  assert.equal(textContentValue({ type: "text", text: "", content: "fallback content" }), "fallback content");
  assert.equal(textContentValue({ type: "text", text: "", content: "", value: "" }), "");
  assert.equal(textOnlyContent([{ type: "text", text: "" }]), "");
});

test("declines streaming text for mixed or malformed content blocks", () => {
  assert.equal(textOnlyContent([]), undefined);
  assert.equal(textOnlyContent([{ type: "text", content: "partial" }, { type: "image", data: "abc" }]), undefined);
  assert.equal(textOnlyContent([{ type: "text", markdown: "missing text aliases" }]), undefined);
  assert.equal(textContentValue({ type: "resource", resource: { text: "not message text" } }), undefined);
});

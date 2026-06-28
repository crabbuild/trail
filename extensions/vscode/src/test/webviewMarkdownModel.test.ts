import assert from "node:assert/strict";
import test from "node:test";
import { renderMarkdown } from "../webview/markdownModel";

test("renders common agent markdown blocks into structured safe html", () => {
  const html = renderMarkdown(
    [
      "## Summary",
      "",
      "Done with **bold** text, `inline code`, and [docs](https://example.com/docs).",
      "",
      "| File | Status |",
      "| --- | ---: |",
      "| README.md | changed |",
      "",
      "- [x] Inspect",
      "- Patch",
      "",
      "> Keep this visible.",
      "",
      "```ts",
      "const ok = true;",
      "```",
      "",
      "<script>alert('x')</script>"
    ].join("\n"),
    {
      renderCodeBlock: (code, language) => `<pre data-language="${language}"><code>${code}</code></pre>`
    }
  );

  assert.match(html, /<h2>Summary<\/h2>/);
  assert.match(html, /<strong>bold<\/strong>/);
  assert.match(html, /<code>inline code<\/code>/);
  assert.match(html, /<a href="https:\/\/example\.com\/docs"/);
  assert.match(html, /<table>/);
  assert.match(html, /<th class="align-left">File<\/th>/);
  assert.match(html, /<td class="align-right">changed<\/td>/);
  assert.match(html, /class="task-list-checkbox" type="checkbox" disabled checked/);
  assert.match(html, /<blockquote>/);
  assert.match(html, /<pre data-language="ts"><code>const ok = true;<\/code><\/pre>/);
  assert.match(html, /&lt;script&gt;alert\(&#39;x&#39;\)&lt;\/script&gt;/);
  assert.doesNotMatch(html, /<script>/);
});

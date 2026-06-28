import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DiffCard, type DiffCardProps } from "../webview/DiffCard";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';

function renderDiff(props: DiffCardProps): string {
  return renderToStaticMarkup(React.createElement(DiffCard, { props }));
}

function baseProps(overrides: Partial<DiffCardProps> = {}): DiffCardProps {
  return {
    nodeId: "diff-1",
    path: "src/webview/main.ts",
    subtitle: "3 old - 4 new",
    iconHtml,
    stats: [{ label: "3 old" }, { label: "4 new" }],
    previewHtml:
      '<section class="diff-preview diff-preview-loading" data-diff-preview-id="diff-preview-1"><template class="diff-source"></template></section>',
    ...overrides
  };
}

test("renders diff previews with shadcn accordion card and badge primitives", () => {
  const html = renderDiff(baseProps());

  assert.match(html, /data-diff-card/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="card-content"/);
  assert.match(html, /data-slot="accordion"/);
  assert.match(html, /data-slot="accordion-item"/);
  assert.match(html, /data-slot="accordion-trigger"/);
  assert.match(html, /data-slot="accordion-content"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /class="[^"]*tool-summary[^"]*diff-summary/);
  assert.match(html, /class="[^"]*tool-summary-meta/);
  assert.match(html, /class="[^"]*diff-stat/);
  assert.match(html, /class="diff-preview diff-preview-loading"/);
});

test("keeps diff helper markup mounted without legacy details markup", () => {
  const html = renderDiff(baseProps());

  assert.match(html, /template class="diff-source"/);
  assert.doesNotMatch(html, /<details/);
  assert.doesNotMatch(html, /<summary/);
});

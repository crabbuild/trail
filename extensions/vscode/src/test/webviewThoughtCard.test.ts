import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ThoughtCard, type ThoughtCardProps } from "../webview/ThoughtCard";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';

function renderThought(props: ThoughtCardProps): string {
  return renderToStaticMarkup(React.createElement(ThoughtCard, { props }));
}

function baseProps(overrides: Partial<ThoughtCardProps> = {}): ThoughtCardProps {
  return {
    nodeId: "thought-1",
    title: "Thinking",
    detail: "Inspecting workspace files before editing.",
    statusLabel: "live",
    iconHtml,
    contentHtml: '<p>Inspecting workspace files before editing.</p>',
    emptyText: "No thought content reported.",
    ...overrides
  };
}

test("renders thinking updates with shadcn accordion card and badge primitives", () => {
  const html = renderThought(baseProps());

  assert.match(html, /data-thought-card/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="card-content"/);
  assert.match(html, /data-slot="accordion"/);
  assert.match(html, /data-slot="accordion-item"/);
  assert.match(html, /data-slot="accordion-trigger"/);
  assert.match(html, /data-slot="accordion-content"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /class="[^"]*event-summary[^"]*thought-summary/);
  assert.match(html, /class="[^"]*tool-status/);
  assert.match(html, /class="markdown event-content"/);
});

test("renders empty thought content without legacy details markup", () => {
  const html = renderThought(
    baseProps({
      contentHtml: "",
      statusLabel: "done"
    })
  );

  assert.match(html, /No thought content reported/);
  assert.match(html, /class="[^"]*thought-empty/);
  assert.doesNotMatch(html, /<details/);
  assert.doesNotMatch(html, /<summary/);
});

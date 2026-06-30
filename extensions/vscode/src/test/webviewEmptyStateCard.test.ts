import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { EmptyStateCard, type EmptyStateCardProps } from "../webview/EmptyStateCard";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';

function renderEmpty(props: EmptyStateCardProps): string {
  return renderToStaticMarkup(React.createElement(EmptyStateCard, { props }));
}

test("renders ready transcript empty states with shadcn Empty and Button parts", () => {
  const html = renderEmpty({
    id: "ready",
    variant: "ready",
    ariaLabel: "Empty transcript",
    iconHtml,
    roleLabel: "CrabDB workspace",
    title: "Ready for a CrabDB turn",
    description: "Message the agent.",
    actions: [
      { action: "focusComposer", label: "Write a message", iconHtml, tone: "primary", disabled: false },
      { action: "attachSelection", label: "Attach selection", iconHtml, tone: "secondary", disabled: false },
      { action: "attachFile", label: "Attach file", iconHtml, tone: "secondary", disabled: false }
    ]
  });

  assert.match(html, /data-slot="empty"/);
  assert.match(html, /data-empty-state-card/);
  assert.match(html, /empty-state-ready/);
  assert.match(html, /data-slot="empty-icon"/);
  assert.match(html, /empty-state-media/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /empty-state-role/);
  assert.match(html, /data-slot="empty-title"/);
  assert.match(html, /Ready for a CrabDB turn/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-action="focusComposer"/);
  assert.match(html, /Write a message/);
  assert.match(html, /empty-action-label/);
  assert.match(html, /data-action="attachSelection"/);
  assert.match(html, /data-action="attachFile"/);
  assert.match(html, /empty-action-primary/);
  assert.match(html, /data-icon="inline-start"/);
  assert.doesNotMatch(html, /data-empty-icon-only/);
  assert.doesNotMatch(html, /sr-only/);
  assert.doesNotMatch(html, /Start in composer/);
  assert.doesNotMatch(html, /data-action="openSettings"/);
  assert.doesNotMatch(html, /Settings/);
  assert.doesNotMatch(html, /card-chrome/);
  assert.doesNotMatch(html, /class="tool-icon"[\s\S]*data-action="focusComposer"/);
});

test("renders filtered transcript empty states with disabled-safe action markup", () => {
  const html = renderEmpty({
    id: "filtered",
    variant: "filtered",
    ariaLabel: "No transcript items match the active filters.",
    iconHtml,
    roleLabel: "Transcript filter",
    title: "No matching transcript items",
    description: "Clear filters to return to the full run.",
    actions: [
      { action: "clearTimelineSearch", label: "Clear filters", iconHtml, tone: "primary", disabled: true }
    ]
  });

  assert.match(html, /empty-state-filtered/);
  assert.match(html, /Transcript filter/);
  assert.match(html, /data-action="clearTimelineSearch"/);
  assert.match(html, /disabled=""/);
});

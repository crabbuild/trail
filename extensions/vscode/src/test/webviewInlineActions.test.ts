import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { InlineActions, type InlineActionsProps } from "../webview/InlineActions";

function renderInlineActions(props: InlineActionsProps): string {
  return renderToStaticMarkup(React.createElement(InlineActions, { props }));
}

test("renders helper action rails with shadcn button groups while preserving action selectors", () => {
  const html = renderInlineActions({
    id: "inline-actions-1",
    ariaLabel: "Conflict actions",
    className: "conflict-actions",
    actions: [
      { action: "compareTasks", label: "Compare tasks", tone: "provider" },
      {
        action: "showConflict",
        data: { "conflict-id": "task/conflict:1" },
        label: "Open conflict:1",
        tone: "review"
      },
      {
        action: "openResource",
        className: "resource-chip chip-button",
        data: { uri: "file:///tmp/report.json" },
        detail: "report.json",
        label: "Open report",
        tone: "provider"
      }
    ]
  });

  assert.match(html, /data-slot="button-group"/);
  assert.match(html, /class="[^"]*inline-actions[^"]*conflict-actions/);
  assert.match(html, /data-inline-actions=""/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-action="compareTasks"/);
  assert.match(html, /data-action="showConflict"/);
  assert.match(html, /data-conflict-id="task\/conflict:1"/);
  assert.match(html, /data-action="openResource"/);
  assert.match(html, /data-uri="file:\/\/\/tmp\/report\.json"/);
  assert.match(html, /class="[^"]*resource-chip[^"]*chip-button/);
  assert.match(html, /<small>report\.json<\/small>/);
});

test("renders icon-only media actions with accessible labels", () => {
  const html = renderInlineActions({
    id: "inline-actions-2",
    ariaLabel: "Image preview actions",
    actions: [
      {
        action: "openMediaPreview",
        ariaLabel: "Open image preview",
        iconHtml: '<svg class="icon" aria-hidden="true"></svg>',
        iconOnly: true,
        label: "Open image preview",
        tone: "provider"
      }
    ]
  });

  assert.match(html, /data-slot="button-group"/);
  assert.match(html, /aria-label="Image preview actions"/);
  assert.match(html, /data-action="openMediaPreview"/);
  assert.match(html, /aria-label="Open image preview"/);
  assert.match(html, /data-inline-icon-only="true"/);
  assert.match(html, /data-icon="inline-start"/);
  assert.match(html, /class="sr-only">Open image preview<\/span>/);
  assert.doesNotMatch(html, /icon-button/);
  assert.doesNotMatch(html, /tool-icon/);
  assert.doesNotMatch(html, /<small>/);
});

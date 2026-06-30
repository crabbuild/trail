import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { TimelineGroupCard, type TimelineGroupCardProps } from "../webview/TimelineGroup";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';
const timelineGroupSource = fs.readFileSync(path.join(process.cwd(), "src", "webview", "TimelineGroup.tsx"), "utf8");

function renderTimelineGroup(props: TimelineGroupCardProps): string {
  return renderToStaticMarkup(React.createElement(TimelineGroupCard, { props }));
}

function baseProps(overrides: Partial<TimelineGroupCardProps> = {}): TimelineGroupCardProps {
  return {
    id: "node-group-turn-1",
    label: "Turn 1",
    detail: "1 message / 1 tool",
    status: "completed",
    statusLabel: "Completed",
    laneId: "agent-claude-code-679549e33f6c",
    iconHtml,
    bodyItems: [
      {
        id: "message-1",
        className: "timeline-group-body-item timeline-group-body-item-message",
        html: '<article class="turn-card" data-node-id="message-1">Message</article>',
        preserveDom: true
      }
    ],
    open: true,
    ...overrides
  };
}

test("renders timeline groups with shadcn accordion and badge primitives", () => {
  const html = renderTimelineGroup(baseProps());

  assert.match(html, /data-slot="accordion"/);
  assert.match(html, /data-slot="accordion-item"/);
  assert.match(html, /data-slot="accordion-trigger"/);
  assert.match(html, /data-slot="accordion-content"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /class="[^"]*timeline-group-summary/);
  assert.match(html, /class="[^"]*timeline-group-copy-id/);
  assert.match(html, /data-action="copyTimelineGroupId"/);
  assert.match(html, /data-target="agent-claude-code-679549e33f6c"/);
  assert.match(html, /aria-label="Copy ID"/);
  assert.match(html, /class="[^"]*timeline-group-body/);
  assert.match(html, /data-timeline-group-body-item/);
  assert.match(html, /data-node-id="message-1"/);
  assert.match(html, /class="turn-card"/);
  assert.match(html, /aria-expanded="true"/);
  assert.doesNotMatch(html, />agent-claude-code-679549e33f6c</);
});

test("keeps helper-rendered body selectors without native details markup", () => {
  const html = renderTimelineGroup(baseProps({ open: false }));

  assert.match(html, /data-node-id="message-1"/);
  assert.match(html, /aria-expanded="false"/);
  assert.doesNotMatch(html, /<details/);
  assert.doesNotMatch(html, /<summary/);
});

test("renders group body as stable keyed node rows", () => {
  const html = renderTimelineGroup(
    baseProps({
      bodyItems: [
        {
          id: "message-1",
          className: "timeline-group-body-item timeline-group-body-item-message",
          html: '<article class="turn-card message" data-node-id="message-1">Message</article>',
          preserveDom: true
        },
        {
          id: "tool-1",
          className: "timeline-group-body-item timeline-group-body-item-tool",
          html: '<article class="turn-card tool" data-node-id="tool-1">Tool</article>',
          preserveDom: true
        }
      ]
    })
  );

  assert.match(html, /timeline-group-body-item-message/);
  assert.match(html, /timeline-group-body-item-tool/);
  assert.match(html, /data-node-id="tool-1"/);
});

test("preserves mounted node islands across group rerenders", () => {
  const html = renderTimelineGroup(baseProps());

  assert.match(html, /data-stable-html-slot="message-1"/);
  assert.match(timelineGroupSource, /React\.memo\([\s\S]*StableHtmlSlot/);
  assert.match(timelineGroupSource, /previous\.slotId === next\.slotId && previous\.shellSignature === next\.shellSignature/);
  assert.match(timelineGroupSource, /React\.useLayoutEffect\([\s\S]*syncStableHtmlShell/);
  assert.match(timelineGroupSource, /function syncElementAttributes/);
  assert.match(timelineGroupSource, /html: item\.preserveDom \? stableHtmlShellSignature\(item\.html\) : item\.html/);
});

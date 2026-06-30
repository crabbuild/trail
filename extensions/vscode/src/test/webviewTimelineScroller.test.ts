import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { TimelineScroller, type TimelineScrollerProps } from "../webview/TimelineScroller";

const timelineScrollerSource = fs.readFileSync(path.join(process.cwd(), "src", "webview", "TimelineScroller.tsx"), "utf8");
const messageScrollerSource = fs.readFileSync(path.join(process.cwd(), "src", "webview", "components", "ui", "message-scroller.tsx"), "utf8");

function renderTimelineScroller(props: TimelineScrollerProps): string {
  return renderToStaticMarkup(React.createElement(TimelineScroller, { props }));
}

test("renders transcript rows as shadcn message scroller items", () => {
  const html = renderTimelineScroller({
    items: [
      {
        id: "node-group-turn-1",
        className: "timeline-scroller-row-group",
        scrollAnchor: true,
        html: '<div id="node-group-turn-1" data-timeline-group-root>Turn</div>',
        preserveDom: true
      },
      {
        id: "timeline-empty",
        className: "timeline-scroller-row-empty",
        html: '<div data-empty-state-card-root>Empty</div>'
      }
    ]
  });

  assert.match(html, /data-slot="message-scroller"/);
  assert.match(html, /data-slot="message-scroller-viewport"/);
  assert.match(html, /data-slot="message-scroller-content"/);
  assert.match(html, /data-slot="message-scroller-item"/);
  assert.match(html, /class="[^"]*timeline-scroller-row-group/);
  assert.match(html, /class="[^"]*timeline-scroller-row-empty/);
  assert.match(html, /data-timeline-group-root/);
  assert.match(html, /data-stable-html-slot="node-group-turn-1"/);
  assert.match(html, /data-empty-state-card-root/);
  assert.doesNotMatch(html, /dangerouslySetInnerHTML/);
});

test("configures shadcn message scroller for AI chat transcripts", () => {
  assert.match(timelineScrollerSource, /<MessageScrollerProvider[\s\S]*autoScroll/);
  assert.match(timelineScrollerSource, /defaultScrollPosition="last-anchor"/);
  assert.match(timelineScrollerSource, /scrollPreviousItemPeek=\{TIMELINE_PREVIOUS_ITEM_PEEK\}/);
  assert.match(timelineScrollerSource, /React\.memo\([\s\S]*StableHtmlSlot/);
  assert.match(timelineScrollerSource, /previous\.slotId === next\.slotId && previous\.shellSignature === next\.shellSignature/);
  assert.match(timelineScrollerSource, /React\.useLayoutEffect\([\s\S]*syncStableHtmlShell/);
  assert.match(timelineScrollerSource, /function syncElementAttributes/);
  assert.match(timelineScrollerSource, /html: item\.preserveDom \? stableHtmlShellSignature\(item\.html\) : item\.html/);
  assert.match(messageScrollerSource, /cn-message-scroller-viewport/);
  assert.match(messageScrollerSource, /\[contain-intrinsic-size:auto_10rem\] \[content-visibility:auto\]/);
});

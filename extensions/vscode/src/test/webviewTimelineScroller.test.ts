import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { TimelineScroller, type TimelineScrollerProps } from "../webview/TimelineScroller";

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
        html: '<div id="node-group-turn-1" data-timeline-group-root>Turn</div>'
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
  assert.match(html, /data-empty-state-card-root/);
  assert.doesNotMatch(html, /dangerouslySetInnerHTML/);
});

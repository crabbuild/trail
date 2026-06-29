import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { TimelineNavigation, type TimelineNavigationProps } from "../webview/TimelineNavigation";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';

function renderTimelineNavigation(props: TimelineNavigationProps): string {
  return renderToStaticMarkup(React.createElement(TimelineNavigation, { props }));
}

function baseProps(overrides: Partial<TimelineNavigationProps> = {}): TimelineNavigationProps {
  return {
    id: "timeline",
    filters: [
      { id: "all", label: "All", count: 12, active: true },
      { id: "chat", label: "Chat", count: 4, active: false },
      { id: "tools", label: "Tools", count: 6, active: false },
      { id: "diffs", label: "Diffs", count: 1, active: false },
      { id: "approvals", label: "Approvals", count: 1, active: false },
      { id: "events", label: "Events", count: 0, active: false }
    ],
    query: "schema",
    queryDetail: " matching schema",
    filtered: true,
    visibleCount: 5,
    searchIconHtml: iconHtml,
    mapIconHtml: iconHtml,
    activityIconHtml: iconHtml,
    visibleGroups: 2,
    chips: [
      { id: "lane", label: "Lane agent-1", iconHtml, active: true },
      { id: "session", label: "Session session-1", iconHtml },
      { id: "turns", label: "2 turns", iconHtml }
    ],
    activity: {
      total: 3,
      label: "Tool activity",
      detail: "2 edits / 1 command",
      tone: "warning",
      metrics: [
        { label: "Changes", value: "2", tone: "warning" },
        { label: "Commands", value: "1", tone: "active" }
      ],
      paths: [
        {
          path: "src/webview/main.ts",
          count: 2,
          detail: "2 edit operations",
          tone: "warning"
        }
      ]
    },
    turnLinks: [
      { id: "turn:1", href: "#node-group-turn-1", label: "Turn 1", detail: "2 messages / 1 tool" },
      { id: "turn:2", href: "#node-group-turn-2", label: "Turn 2", detail: "2 messages / 2 tools" }
    ],
    ...overrides
  };
}

test("renders the transcript toolbar with shadcn primitives", () => {
  const html = renderTimelineNavigation(baseProps());

  assert.match(html, /class="[^"]*timeline-toolbar/);
  assert.match(html, /data-slot="collapsible"/);
  assert.match(html, /data-slot="collapsible-trigger"/);
  assert.match(html, /data-slot="collapsible-content"/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="button-group"/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /class="timeline-filter-trigger"/);
  assert.match(html, /class="timeline-filter-trigger-label"/);
  assert.match(html, /class="[^"]*timeline-filter-popover/);
  assert.match(html, /class="[^"]*timeline-filter[^"]*active/);
  assert.match(
    html,
    /class="timeline-search"[\s\S]*<span data-icon="inline-start"/
  );
  assert.doesNotMatch(html, /<span[^>]*class="icon"[^>]*data-icon="inline-start"/);
  assert.match(html, /data-action="setTimelineFilter"/);
  assert.match(html, /data-timeline-filter="all"/);
  assert.match(html, /class="timeline-search-input"/);
  assert.match(html, /defaultValue="schema"|value="schema"/);
  assert.match(html, /data-action="clearTimelineSearch"/);
  assert.doesNotMatch(html, /data-slot="dropdown-menu-trigger"/);
  assert.doesNotMatch(html, /Lane map/);
  assert.doesNotMatch(html, /session-map/);
});

test("keeps lane map content out of the main transcript navigation", () => {
  const html = renderTimelineNavigation(baseProps());

  assert.doesNotMatch(html, /class="[^"]*event-chip/);
  assert.doesNotMatch(html, /class="[^"]*tool-activity/);
  assert.doesNotMatch(html, /class="session-map-turn"/);
  assert.doesNotMatch(html, /href="#node-group-turn-1"/);
  assert.doesNotMatch(html, /<details class="session-map"/);
  assert.doesNotMatch(html, /<summary class="session-map-summary"/);
});

test("renders empty filtered activity without phantom turn links", () => {
  const html = renderTimelineNavigation(
    baseProps({
      activity: {
        total: 0,
        label: "No visible tool activity",
        detail: "Clear filters to inspect all tool activity.",
        tone: "empty",
        metrics: [],
        paths: []
      },
      turnLinks: []
    })
  );

  assert.doesNotMatch(html, /Clear the transcript filter to inspect all tool activity/);
  assert.doesNotMatch(html, /class="session-map-turn"/);
});

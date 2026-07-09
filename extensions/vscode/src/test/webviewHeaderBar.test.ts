import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { HeaderBar, type HeaderBarProps } from "../webview/HeaderBar";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';

function renderHeader(props: HeaderBarProps): string {
  return renderToStaticMarkup(React.createElement(HeaderBar, { props }));
}

function baseProps(overrides: Partial<HeaderBarProps> = {}): HeaderBarProps {
  return {
    id: "header",
    title: "Review shadcn migration",
    status: "blocked",
    showStatusPill: true,
    usage: { used: 70, size: 100 },
    detailsIconHtml: iconHtml,
    capabilitiesIconHtml: iconHtml,
    primaryActionIconHtml: iconHtml,
    laneMap: {
      id: "timeline",
      visibleCount: 5,
      visibleGroups: 2,
      mapIconHtml: iconHtml,
      activityIconHtml: iconHtml,
      chips: [
        { id: "lane", label: "Lane agent-1", iconHtml, active: true },
        { id: "session", label: "Session session-1", iconHtml }
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
        { id: "turn:1", href: "#node-group-turn-1", label: "Turn 1", detail: "2 messages / 1 tool" }
      ]
    },
    toolbar: {
      runState: {
        label: "Needs review",
        detail: "Trail has warnings for this lane.",
        tone: "warning"
      },
      primaryAction: {
        action: "focusReview",
        label: "Open review",
        detail: "Check readiness, conflicts, approvals, and gates.",
        tone: "primary"
      },
      statusChips: [
        {
          id: "provider",
          label: "Provider",
          value: "Claude Code via Trail",
          displayValue: "Claude Code via Trail",
          tone: "ok",
          accessibilityLabel: "Provider: Claude Code via Trail"
        },
        {
          id: "lane",
          label: "Lane",
          value: "agent-1",
          displayValue: "agent-1",
          tone: "active",
          accessibilityLabel: "Lane: agent-1"
        }
      ],
      capabilities: [
        {
          id: "durable-state",
          label: "Durable state",
          group: "workflow",
          enabled: true,
          detail: "Trail persists transcript, checkpoints, review, and queue state."
        },
        {
          id: "image",
          label: "Images",
          group: "input",
          enabled: false,
          detail: "Images are hidden for this provider."
        }
      ],
      capabilitySummary: "1/2 ready"
    },
    inspectActions: [
      {
        action: "toggleReview",
        label: "Open review",
        iconHtml,
        active: true,
        ariaPressed: true,
        ariaExpanded: true,
        ariaControls: "review"
      },
      { action: "openDiff", label: "Open diff", iconHtml },
      { action: "openSettings", label: "Open Trail settings", iconHtml }
    ],
    runActions: [
      { action: "refresh", label: "Refresh task", iconHtml },
      { action: "cancel", label: "Cancel current turn", iconHtml, disabled: true }
    ],
    ...overrides
  };
}

test("renders the header toolbar with shadcn badges button groups cards and buttons", () => {
  const html = renderHeader(baseProps());

  assert.match(html, /class="header-main"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /data-slot="button-group"/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="collapsible"/);
  assert.match(html, /data-slot="collapsible-trigger"/);
  assert.match(html, /data-slot="collapsible-content"/);
  assert.match(html, /toolbar-run-warning/);
  assert.match(html, /toolbar-chip-ok/);
  assert.match(html, /toolbar-capability-grid/);
  assert.match(html, /toolbar-capability-section-workflow/);
  assert.match(html, /toolbar-capability-section-input/);
  assert.match(html, /data-lane-map-trigger="true"/);
});

test("preserves header actions and floating details selectors", () => {
  const html = renderHeader(baseProps());

  assert.match(html, /class="header-details"/);
  assert.match(html, /class="toolbar-capabilities"/);
  assert.match(html, /class="header-details-trigger"/);
  assert.match(html, /class="toolbar-capabilities-trigger"/);
  assert.match(
    html,
    /class="header-details-trigger"[\s\S]*<span data-icon="inline-start"/
  );
  assert.match(
    html,
    /class="toolbar-capabilities-trigger"[\s\S]*<span data-icon="inline-start"/
  );
  assert.doesNotMatch(
    html,
    /<(?:span)[^>]*class="icon"[^>]*data-icon="inline-start"/
  );
  assert.doesNotMatch(html, /<details/);
  assert.doesNotMatch(html, /<summary/);
  assert.match(html, /data-action="focusReview"/);
  assert.match(html, /data-action="toggleReview"/);
  assert.match(html, /aria-pressed="true"/);
  assert.match(html, /aria-expanded="true"/);
  assert.match(html, /aria-controls="review"/);
  assert.match(html, /data-action="openDiff"/);
  assert.match(html, /data-action="openSettings"/);
  assert.match(html, /data-action="refresh"/);
  assert.match(html, /data-action="cancel"/);
  assert.match(html, /data-header-icon-only="true"/);
  assert.match(html, /aria-controls="lane-map-drawer"/);
  assert.match(html, /aria-label="Open lane map"/);
  assert.match(html, /data-icon="inline-start"/);
  assert.doesNotMatch(html, /icon-button/);
  assert.doesNotMatch(html, /class="icon" data-icon="inline-start"/);
  assert.match(html, /disabled=""/);
});

test("renders context usage and capability action details accessibly", () => {
  const html = renderHeader(baseProps());

  assert.match(html, /Context usage 70%/);
  assert.match(html, /Trail capabilities/);
  assert.match(html, /1\/2 ready/);
  assert.match(html, /data-capability="durable-state"/);
  assert.match(html, /aria-label="Durable state: ready"/);
  assert.match(html, /data-capability="image"/);
  assert.match(html, /aria-label="Images: unavailable"/);
});

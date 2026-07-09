import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { RecoveryBanner, type RecoveryBannerProps } from "../webview/RecoveryBanner";

function renderBanner(props: RecoveryBannerProps): string {
  return renderToStaticMarkup(React.createElement(RecoveryBanner, { props }));
}

function baseProps(overrides: Partial<RecoveryBannerProps> = {}): RecoveryBannerProps {
  return {
    id: "provider-failure",
    kind: "failure",
    role: "alert",
    ariaLive: "assertive",
    eyebrow: "Agent interrupted",
    title: "Provider stopped early",
    description: "Partial transcript and lane changes remain in Trail.",
    detail: "The provider exited before the turn finished.",
    badges: ["exit 1", "12:34"],
    actions: [
      { action: "focusReview", label: "Open review", tone: "review" },
      { action: "startFollowUp", label: "Start follow-up", tone: "primary" },
      { action: "showAcpLogs", label: "Show logs", tone: "provider" }
    ],
    paths: [],
    ...overrides
  };
}

test("renders provider failure recovery with shadcn alert badges and buttons", () => {
  const html = renderBanner(baseProps());

  assert.match(html, /data-recovery-banner/);
  assert.match(html, /data-slot="alert"/);
  assert.match(html, /class="[^"]*recovery-banner/);
  assert.match(html, /role="alert"/);
  assert.match(html, /aria-live="assertive"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /class="[^"]*recovery-banner-role/);
  assert.match(html, /class="[^"]*tool-status/);
  assert.match(html, /data-slot="button-group"/);
  assert.match(html, /data-inline-actions/);
  assert.match(html, /recovery-banner-badges/);
  assert.match(html, /class="[^"]*recovery-actions/);
  assert.match(html, /data-action="focusReview"/);
  assert.match(html, /data-action="startFollowUp"/);
  assert.match(html, /data-action="showAcpLogs"/);
  assert.match(html, /data-recovery-action-tone="review"/);
  assert.match(html, /inline-action-primary/);
  assert.doesNotMatch(html, /card-chrome/);
  assert.doesNotMatch(html, /class="role(?:\s|")|class="[^"]*\srole(?:\s|")/);
});

test("renders overlap warning paths and coordination actions", () => {
  const html = renderBanner(
    baseProps({
      id: "task-overlap",
      kind: "overlap",
      role: "status",
      ariaLive: "polite",
      eyebrow: "Parallel work overlap",
      title: "Schema task also changes src/db/schema.ts",
      description: "Compare tasks or refresh Trail state before applying this lane.",
      detail: undefined,
      badges: ["2 tasks", "3 shared paths"],
      actions: [
        { action: "compareTasks", label: "Compare tasks", tone: "provider" },
        { action: "refresh", label: "Refresh", tone: "lane" },
        { action: "queueMerge", label: "Queue merge", tone: "lane" }
      ],
      paths: [
        {
          id: "task-1",
          title: "Schema task",
          labels: "src/db/schema.ts, src/db/client.ts"
        }
      ]
    })
  );

  assert.match(html, /class="[^"]*overlap-banner/);
  assert.match(html, /role="status"/);
  assert.match(html, /aria-live="polite"/);
  assert.match(html, /data-slot="separator"/);
  assert.match(html, /class="overlap-paths"/);
  assert.match(html, /class="[^"]*overlap-path/);
  assert.match(html, /<b>Schema task<\/b>/);
  assert.match(html, /data-action="compareTasks"/);
  assert.match(html, /data-action="refresh"/);
  assert.match(html, /data-action="queueMerge"/);
});

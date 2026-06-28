import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { PlanCard, type PlanCardProps } from "../webview/PlanCard";

function renderPlan(props: PlanCardProps): string {
  return renderToStaticMarkup(React.createElement(PlanCard, { props }));
}

function baseProps(overrides: Partial<PlanCardProps> = {}): PlanCardProps {
  return {
    nodeId: "plan-1",
    title: "Plan",
    detail: "3 tracked steps",
    entries: [
      {
        id: "plan-1-0",
        title: "Inspect the current webview renderer",
        status: "completed",
        statusClass: "completed"
      },
      {
        id: "plan-1-1",
        title: "Migrate the plan surface",
        status: "in_progress",
        statusClass: "in-progress",
        priority: "high"
      },
      {
        id: "plan-1-2",
        title: "Run checks",
        status: "pending",
        statusClass: "pending"
      }
    ],
    emptyText: "No plan steps reported.",
    ...overrides
  };
}

test("renders plan updates with shadcn card badge checkbox and separator primitives", () => {
  const html = renderPlan(baseProps());

  assert.match(html, /data-plan-card/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="card-header"/);
  assert.match(html, /data-slot="card-action"/);
  assert.match(html, /data-slot="card-title"/);
  assert.match(html, /data-slot="card-description"/);
  assert.match(html, /data-slot="card-content"/);
  assert.match(html, /data-slot="checkbox"/);
  assert.match(html, /data-slot="checkbox-indicator"/);
  assert.match(html, /data-slot="separator"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /class="[^"]*plan-list/);
  assert.match(html, /class="[^"]*plan-card-title/);
  assert.match(html, /class="[^"]*plan-card-action/);
  assert.match(html, /class="[^"]*plan-item[^"]*plan-completed/);
  assert.match(html, /class="[^"]*plan-status-checkbox/);
  assert.match(html, /aria-label="Inspect the current webview renderer: completed"/);
  assert.match(html, /class="[^"]*plan-status/);
  assert.match(html, /class="[^"]*plan-priority/);
  assert.match(html, /Migrate the plan surface/);
  assert.doesNotMatch(html, /card-chrome/);
  assert.doesNotMatch(html, /class="role(?:\s|")|class="[^"]*\srole(?:\s|")/);
});

test("renders empty plans without the legacy inline card chrome", () => {
  const html = renderPlan(
    baseProps({
      detail: "No plan steps reported",
      entries: []
    })
  );

  assert.match(html, /No plan steps reported/);
  assert.match(html, /class="[^"]*plan-empty/);
  assert.doesNotMatch(html, /<ol class="plan-list"/);
  assert.doesNotMatch(html, /card-chrome/);
  assert.doesNotMatch(html, /class="role(?:\s|")|class="[^"]*\srole(?:\s|")/);
});

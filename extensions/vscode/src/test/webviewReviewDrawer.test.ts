import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ReviewDrawer, type ReviewDrawerProps } from "../webview/ReviewDrawer";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';

function renderReviewDrawer(props: ReviewDrawerProps): string {
  return renderToStaticMarkup(React.createElement(ReviewDrawer, { props }));
}

function baseProps(overrides: Partial<ReviewDrawerProps> = {}): ReviewDrawerProps {
  return {
    id: "review",
    readiness: {
      tone: "ready",
      statusLabel: "Ready",
      headline: "Ready for dry-run",
      description: "No blocking gates are reported.",
      primaryAction: {
        action: "dryRunApply",
        label: "Dry-run apply",
        description: "Preview workspace changes safely",
        tone: "primary"
      },
      gates: [
        {
          id: "changes",
          label: "Changes",
          value: "2 paths",
          detail: "Changed paths are available for diff review.",
          tone: "ok"
        },
        {
          id: "tests",
          label: "Tests",
          value: "Missing",
          detail: "Run a test gate before applying.",
          tone: "warning"
        }
      ],
      metrics: [
        { label: "Changed paths", value: "2", tone: "ok" },
        { label: "Open gates", value: "1", tone: "warning" }
      ],
      actionGroups: [
        {
          id: "next",
          label: "Next step",
          detail: "The safest action for the current review state.",
          actions: [
            {
              action: "dryRunApply",
              label: "Dry-run apply",
              description: "Preview workspace changes safely",
              tone: "primary"
            },
            {
              action: "refresh",
              label: "Refresh",
              description: "Fetch the latest Trail review state.",
              tone: "default"
            }
          ]
        },
        {
          id: "recover",
          label: "Recover",
          detail: "Explicit recovery actions for this lane.",
          actions: [
            {
              action: "removeTask",
              label: "Remove task",
              description: "Delete this task record from the Trail view.",
              tone: "danger"
            }
          ]
        }
      ]
    },
    sectionsHtml: '<section class="review-section"><h3>Diffs</h3><ul><li><button class="resource-chip chip-button" data-action="openLocation" data-path="src/app.ts">src/app.ts</button></li></ul></section>',
    refreshAction: {
      action: "refresh",
      label: "Refresh",
      description: "Fetch the latest review state.",
      tone: "default"
    },
    actionIcons: {
      dryRunApply: iconHtml,
      refresh: iconHtml,
      removeTask: iconHtml
    },
    ...overrides
  };
}

test("renders review drawer command center and action rail through shadcn primitives", () => {
  const html = renderReviewDrawer(baseProps());

  assert.match(html, /review-command-center/);
  assert.match(html, /data-review-command/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-slot="button-group"/);
  assert.match(html, /review-primary-action/);
  assert.match(html, /data-action="dryRunApply"/);
  assert.match(html, /data-action="refresh"/);
  assert.match(html, /data-icon="inline-start"/);
  assert.doesNotMatch(
    html,
    /data-action="(?:dryRunApply|refresh|removeTask)"[\s\S]{0,360}<span[^>]*class="icon"[^>]*data-icon="inline-start"/
  );
  assert.match(html, /review-action-group-recover/);
  assert.match(html, /data-action="removeTask"/);
});

test("preserves helper-rendered review section selectors and location actions", () => {
  const html = renderReviewDrawer(baseProps());

  assert.match(html, /review-section-stack/);
  assert.match(html, /class="review-section"/);
  assert.match(html, /class="resource-chip chip-button"/);
  assert.match(html, /data-action="openLocation"/);
  assert.match(html, /data-path="src\/app\.ts"/);
});

test("renders disabled and destructive review actions without changing data-action contracts", () => {
  const html = renderReviewDrawer(
    baseProps({
      readiness: {
        ...baseProps().readiness,
        tone: "blocked",
        statusLabel: "Blocked",
        primaryAction: {
          action: "dryRunApply",
          label: "Dry-run apply",
          description: "Resolve blockers first.",
          tone: "primary",
          disabled: true,
          disabledReason: "Resolve review blockers first."
        },
        actionGroups: [
          {
            id: "next",
            label: "Next step",
            detail: "The safest action for the current review state.",
            actions: [
              {
                action: "dryRunApply",
                label: "Dry-run apply",
                description: "Resolve blockers first.",
                tone: "primary",
                disabled: true,
                disabledReason: "Resolve review blockers first."
              }
            ]
          },
          baseProps().readiness.actionGroups[1]!
        ]
      }
    })
  );

  assert.match(html, /review-command-blocked/);
  assert.match(html, /disabled=""/);
  assert.match(html, /Resolve review blockers first\./);
  assert.match(html, /class="[^"]*danger/);
  assert.match(html, /data-action="removeTask"/);
});

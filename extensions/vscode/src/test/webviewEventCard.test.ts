import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { EventCard, type EventCardProps } from "../webview/EventCard";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';
const eventCardSource = fs.readFileSync(path.join(process.cwd(), "src", "webview", "EventCard.tsx"), "utf8");

function renderEvent(props: EventCardProps): string {
  return renderToStaticMarkup(React.createElement(EventCard, { props }));
}

function baseProps(overrides: Partial<EventCardProps> = {}): EventCardProps {
  return {
    nodeId: "checkpoint-1",
    tone: "success",
    iconHtml,
    title: "Checkpoint saved",
    detail: "ch_123 saved for follow-up or rewind.",
    statusLabel: "Saved",
    facts: [
      {
        label: "Checkpoint",
        value: "ch_123",
        shortValue: "ch_123",
        active: true
      }
    ],
    chipsHtml: '<span class="event-chip">Session s_123</span>',
    callout: {
      title: "Durable recovery point",
      detail: "Available for follow-up starts, rewind, and failed-attempt preservation.",
      tone: "success"
    },
    actions: [
      {
        action: "copyCheckpoint",
        label: "Copy checkpoint",
        tone: "default",
        target: "ch_123",
        iconHtml
      },
      {
        action: "startFollowUp",
        label: "Start follow-up",
        tone: "primary",
        iconHtml
      }
    ],
    meterHtml: "",
    contentHtml: "",
    rawDetails: undefined,
    ...overrides
  };
}

test("renders audit events with shadcn card alert badge and buttons", () => {
  const html = renderEvent(baseProps());

  assert.match(html, /data-event-card/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /data-slot="alert"/);
  assert.match(html, /data-slot="button-group"/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-inline-actions/);
  assert.match(html, /event-fact active/);
  assert.match(html, /event-chip/);
  assert.match(html, /data-action="copyCheckpoint"/);
  assert.match(html, /data-target="ch_123"/);
  assert.match(html, /data-action="startFollowUp"/);
  assert.match(html, /inline-action-primary/);
  assert.match(html, /data-icon="inline-start"/);
  assert.doesNotMatch(html, /class="icon" data-icon="inline-start"/);
  assert.doesNotMatch(html, /event-action-primary/);
  assert.doesNotMatch(html, /class="event-action(?:\s|")|class="[^"]*\sevent-action(?:\s|")/);
});

test("renders checkpoints as collapsed accordion separators", () => {
  const html = renderEvent(
    baseProps({
      variant: "checkpoint",
      callout: undefined,
      defaultOpen: false
    })
  );

  assert.match(html, /checkpoint-separator/);
  assert.match(eventCardSource, /import \{ useSyncedAccordionValue \} from "\.\/syncedAccordionState"/);
  assert.match(eventCardSource, /useSyncedAccordionValue\(eventCardOpenValue\(props\)\)/);
  assert.match(eventCardSource, /value=\{openValue\}[\s\S]*onValueChange=\{setOpenValue\}/);
  assert.doesNotMatch(eventCardSource, /defaultValue=\{props\.defaultOpen/);
  assert.match(html, /data-slot="accordion"/);
  assert.match(html, /data-slot="accordion-trigger"/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /checkpoint-separator-rule/);
  assert.match(html, /data-action="copyCheckpoint"/);
  assert.doesNotMatch(html, /data-slot="card"/);
  assert.doesNotMatch(html, /data-slot="alert"/);
});

test("renders usage meters and raw details without action chrome", () => {
  const html = renderEvent(
    baseProps({
      tone: "warning",
      statusLabel: "75%",
      actions: [],
      callout: undefined,
      chipsHtml: "",
      meterHtml: '<div class="event-meter"><progress class="meter review" value="75" max="100"></progress></div>',
      rawDetails: {
        id: "event-raw",
        label: "Details",
        contentHtml: '<pre class="code-frame">{}</pre>',
        defaultOpen: true
      }
    })
  );

  assert.match(html, /event-meter/);
  assert.match(html, /class="[^"]*raw raw-accordion/);
  assert.match(html, /data-slot="accordion"/);
  assert.match(html, /data-slot="accordion-trigger"/);
  assert.match(html, /data-slot="accordion-content"/);
  assert.match(html, /code-frame/);
  assert.doesNotMatch(html, /<details class="raw"/);
  assert.doesNotMatch(html, /event-action-row/);
});

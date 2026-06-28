import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ComposerCard, type ComposerCardProps } from "../webview/ComposerCard";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';

function renderComposer(props: ComposerCardProps): string {
  return renderToStaticMarkup(React.createElement(ComposerCard, { props }));
}

function baseProps(overrides: Partial<ComposerCardProps> = {}): ComposerCardProps {
  return {
    id: "composer",
    status: {
      tone: "ready",
      label: "Ready for the next turn",
      detail: "Ask for code, tests, review, or attach editor context before sending."
    },
    draft: {
      tone: "ready",
      label: "Draft ready",
      detail: "42 chars",
      maxChars: 120_000,
      meterValue: 42,
      meterPercent: 1
    },
    draftValue: "Review the current changes",
    placeholder: "Message agent",
    keyShortcuts: "Enter Control+Enter Meta+Enter",
    maxChars: 120_000,
    controlsDisabled: false,
    sendBlockedReason: undefined,
    metricsText: "0 attachments - 26 chars - 1 line - 119,974 left",
    attachments: [
      {
        id: "attachment-1",
        kind: "selection",
        label: "src/webview/main.ts",
        mode: "embedded",
        title: "selection: src/webview/main.ts"
      }
    ],
    attachmentSummary: "selection",
    railItems: [
      { id: "state", label: "State", value: "Ready", tone: "ready" },
      { id: "send", label: "Send", value: "Enter sends", tone: "active" }
    ],
    presets: [
      { id: "implement", label: "Implement", detail: "Start a focused code change", iconHtml },
      { id: "review", label: "Review", detail: "Look for bugs and gaps", iconHtml }
    ],
    sendMode: "fast",
    contextUsageHtml: '<span class="composer-context-gauge"><span>12%</span></span>',
    sessionControlsHtml: '<label class="select-control"><span>Provider</span><select data-action="switchProvider"></select></label>',
    contextActions: [
      { action: "attachSelection", label: "Attach selection", iconHtml, disabled: false },
      { action: "attachFile", label: "Attach file", iconHtml, disabled: false }
    ],
    rewindIconHtml: iconHtml,
    sendIconHtml: iconHtml,
    clearIconHtml: iconHtml,
    settingsIconHtml: iconHtml,
    ...overrides
  };
}

test("renders the composer through shadcn card, button groups, buttons, and badges", () => {
  const html = renderComposer(baseProps());

  assert.match(html, /data-composer-card/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="card-content"/);
  assert.match(html, /data-slot="card-footer"/);
  assert.match(html, /data-slot="button-group"/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /data-slot="collapsible"/);
  assert.match(html, /data-slot="collapsible-trigger"/);
  assert.match(html, /data-slot="collapsible-content"/);
  assert.match(html, /class="[^"]*composer-context-rail/);
  assert.match(html, /class="[^"]*composer-input/);
  assert.match(html, /class="composer-controls"/);
  assert.match(html, /class="composer-controls-summary"/);
  assert.match(
    html,
    /class="composer-controls-summary"[\s\S]*<span data-icon="inline-start"/
  );
  assert.doesNotMatch(html, /<details class="composer-controls"/);
  assert.doesNotMatch(html, /<summary class="composer-controls-summary"/);
  assert.match(html, /data-action="insertPromptPreset"/);
  assert.match(html, /data-action="attachSelection"/);
  assert.match(html, /data-action="removeAttachment"/);
  assert.match(html, /data-attachment-id="attachment-1"/);
  assert.match(html, /data-action="send"/);
  assert.match(html, /data-composer-icon-only="true"/);
  assert.match(html, /data-icon="inline-start"/);
  assert.doesNotMatch(html, /icon-button/);
  assert.doesNotMatch(html, /class="icon" data-icon="inline-start"/);
  assert.doesNotMatch(html, /<span[^>]*class="icon"[^>]*dangerouslySetInnerHTML/);
  assert.match(html, /data-action="switchProvider"/);
});

test("renders blocked composer state with alert semantics and disabled send", () => {
  const html = renderComposer(
    baseProps({
      status: {
        tone: "waiting",
        label: "Permission required",
        detail: "Approve or reject the pending tool request before sending another prompt."
      },
      draft: {
        tone: "limit",
        label: "Prompt too long",
        detail: "Shorten the prompt",
        maxChars: 120_000,
        meterValue: 120_000,
        meterPercent: 100
      },
      controlsDisabled: true,
      sendBlockedReason: "Resolve the permission request before sending."
    })
  );

  assert.match(html, /data-slot="alert"/);
  assert.match(html, /composer-run-waiting/);
  assert.match(html, /aria-invalid="true"/);
  assert.match(html, /disabled=""/);
  assert.match(html, /Resolve the permission request before sending\./);
});

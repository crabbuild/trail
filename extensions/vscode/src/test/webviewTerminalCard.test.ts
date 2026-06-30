import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { TerminalCard, type TerminalCardProps } from "../webview/TerminalCard";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';
const terminalCardSource = fs.readFileSync(path.join(process.cwd(), "src", "webview", "TerminalCard.tsx"), "utf8");

function renderTerminal(props: TerminalCardProps): string {
  return renderToStaticMarkup(React.createElement(TerminalCard, { props }));
}

function baseProps(overrides: Partial<TerminalCardProps> = {}): TerminalCardProps {
  return {
    nodeId: "terminal-1",
    status: "completed",
    tone: "ok",
    title: "Run tests",
    subtitle: "npm test",
    statusLabel: "passed",
    iconHtml,
    openIconHtml: iconHtml,
    rows: [
      {
        id: "command",
        kind: "in",
        label: "IN",
        title: "Command",
        detail: "/repo",
        textHtml: "npm test",
        language: "shellscript",
        meta: "/repo",
        tone: "muted",
        truncated: false,
        empty: false,
        openByDefault: true
      },
      {
        id: "section-stdout",
        kind: "out",
        label: "OUT",
        title: "Stdout",
        detail: "2 lines",
        textHtml: "ok\\npassed",
        tone: "ok",
        truncated: false,
        empty: false,
        openByDefault: true
      }
    ],
    ...overrides
  };
}

test("renders terminal cards with shadcn card accordion badges and open action", () => {
  const html = renderTerminal(baseProps());

  assert.match(html, /data-terminal-card/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-slot="accordion"/);
  assert.match(terminalCardSource, /import \{ useSyncedAccordionValue \} from "\.\/syncedAccordionState"/);
  assert.match(terminalCardSource, /useSyncedAccordionValue\(terminalOpenValues\(outputRows\)\)/);
  assert.match(terminalCardSource, /value=\{openValues\}[\s\S]*onValueChange=\{setOpenValues\}/);
  assert.doesNotMatch(terminalCardSource, /defaultValue=\{openValues\}/);
  assert.match(html, /data-slot="accordion-trigger"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /terminal-transcript/);
  assert.match(html, /data-highlight-language="shellscript"/);
  assert.match(html, /data-inline-actions=""/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-action="openTerminal"/);
  assert.match(html, /data-node-id="terminal-1"/);
  assert.match(html, /data-inline-icon-only="true"/);
  assert.match(html, /data-icon="inline-start"/);
  assert.doesNotMatch(html, /icon-button/);
  assert.doesNotMatch(html, /class="icon" data-icon="inline-start"/);
});

test("renders failed terminal sections with truncated output notes", () => {
  const html = renderTerminal(
    baseProps({
      status: "failed",
      tone: "risk",
      statusLabel: "exit 1",
      rows: [
        baseProps().rows[0]!,
        {
          id: "section-stderr",
          kind: "err",
          label: "ERR",
          title: "Stderr",
          detail: "1 line",
          textHtml: '<span class="ansi-fg-red">failed</span>',
          tone: "risk",
          truncated: true,
          empty: false,
          openByDefault: true
        }
      ]
    })
  );

  assert.match(html, /terminal-tone-risk/);
  assert.match(html, /terminal-transcript-err/);
  assert.match(html, /ansi-fg-red/);
  assert.match(html, /truncated at 24,000 chars/);
});

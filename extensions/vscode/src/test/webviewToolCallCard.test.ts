import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ToolCallCard, type ToolCallCardProps } from "../webview/ToolCallCard";
import { buildToolPresentation, type ToolPresentationInput } from "../webview/toolModel";

const noopCallbacks = {
  onOpenLocation() {
    // Render tests only verify markup; click behavior is wired by the webview island.
  }
};

function cardProps(
  input: ToolPresentationInput,
  overrides: Partial<ToolCallCardProps> = {}
): ToolCallCardProps {
  const presentation = buildToolPresentation(input);
  return {
    nodeId: "node-1",
    rawToolKind: input.toolKind,
    title: presentation.title,
    subtitle: presentation.summary,
    status: input.toolStatus,
    terminal: input.content.some((item) => item.type === "terminal"),
    readPreview: false,
    model: {
      icon: presentation.icon,
      kind: presentation.kind,
      operationLabel: presentation.operationLabel,
      openByDefault: true,
      riskLabel: presentation.riskLabel,
      riskTone: presentation.riskTone,
      statusLabel: presentation.statusLabel,
      tone: presentation.tone
    },
    stats: presentation.stats,
    facts: presentation.facts,
    actions: presentation.actions,
    locations: input.locations.map((location) => ({
      path: location.path,
      line: typeof location.line === "number" ? location.line : undefined
    })),
    contentHtml: "",
    rawDetails: undefined,
    ...overrides
  };
}

function renderCard(props: ToolCallCardProps): string {
  return renderToStaticMarkup(
    React.createElement(ToolCallCard, {
      props,
      callbacks: noopCallbacks
    })
  );
}

test("renders read previews without generic tool chrome", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Read package.json",
        toolKind: "read",
        toolStatus: "completed",
        locations: [{ path: "package.json" }],
        content: [{ type: "content", content: { type: "text", text: "{}" } }]
      },
      {
        readPreview: true,
        contentHtml: '<div class="file-document">package metadata</div>'
      }
    )
  );

  assert.match(html, /tool-kind-file/);
  assert.match(html, /tool-detail-read/);
  assert.match(html, /file-document/);
  assert.doesNotMatch(html, /tool-card-actions|tool-action-bar/);
  assert.doesNotMatch(html, /tool-evidence-strip/);
  assert.doesNotMatch(html, /tool-facts/);
});

test("renders edit tools with shadcn actions and diff affordances", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Edit README.md",
        toolKind: "edit",
        toolStatus: "completed",
        locations: [{ path: "README.md" }],
        rawInput: { path: "README.md", line: 12 },
        content: [{ type: "diff", path: "README.md", oldText: "old", newText: "new" }]
      },
      {
        contentHtml: '<div class="diff-preview" tabindex="0">diff</div>'
      }
    )
  );

  assert.match(html, /tool-kind-change/);
  assert.match(html, /Workspace change|change/);
  assert.match(html, /tool-disclosure-icon/);
  assert.match(
    html,
    /<span class="summary-icon tool-summary-icon[^"]*">[\s\S]*<svg[^>]*data-icon="inline-start"[^>]*aria-hidden="true"/
  );
  assert.doesNotMatch(
    html,
    /<span class="summary-icon tool-summary-icon[^"]*">[\s\S]*<svg[^>]*class="icon"/
  );
  assert.match(html, /data-slot="hover-card-trigger"/);
  assert.match(html, /data-slot="context-menu-trigger"/);
  assert.match(html, /data-inline-actions/);
  assert.match(html, /tool-card-actions/);
  assert.doesNotMatch(html, /tool-action-bar/);
  assert.match(html, /data-slot="tooltip-trigger"/);
  assert.match(html, /data-action="focusToolDiff"/);
  assert.match(html, /data-action="openLocation"/);
  assert.match(html, /data-tool-action-index="0"/);
  assert.match(html, /data-icon="inline-start"/);
  assert.doesNotMatch(
    html,
    /data-action="(?:focusToolDiff|openLocation|inspectToolDetails)"[\s\S]{0,260}<svg[^>]*class="icon"/
  );
  assert.doesNotMatch(html, /title="Show this diff preview\."/);
  assert.match(html, /data-slot="breadcrumb"/);
  assert.match(html, /tool-locations/);
  assert.doesNotMatch(html, /class="chips/);
  assert.match(html, /data-slot="breadcrumb-list"/);
  assert.match(html, /data-slot="breadcrumb-page"/);
  assert.match(html, /class="[^"]*tool-fact/);
  assert.match(html, /<span data-slot="badge"[^>]*class="[^"]*tool-fact/);
  assert.match(html, /data-slot="separator"/);
  assert.match(html, /data-orientation="vertical"/);
  assert.match(html, /tool-fact-separator/);
  assert.doesNotMatch(html, /<dl class="tool-facts|<dt>|<dd>/);
  assert.match(html, /diff-preview/);
});

test("renders execute tools as terminal-focused cards", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Run tests",
        toolKind: "execute",
        toolStatus: "completed",
        locations: [],
        rawInput: { command: "npm test" },
        content: [{ type: "terminal", terminalId: "term-1", command: "npm test", stdout: "ok" }]
      },
      {
        terminal: true,
        contentHtml: '<div class="terminal-preview">ok</div>'
      }
    )
  );

  assert.match(html, /tool-detail-terminal/);
  assert.match(html, /terminal-preview/);
  assert.doesNotMatch(html, /tool-card-actions|tool-action-bar/);
});

test("renders failed tools open with status and inspect actions", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Delete generated file",
        toolKind: "delete",
        toolStatus: "failed",
        locations: [{ path: "tmp/generated.txt" }],
        content: []
      },
      {
        rawDetails: {
          id: "node-1-raw",
          label: "Details",
          contentHtml: '<pre>{}</pre>',
          defaultOpen: true
        }
      }
    )
  );

  assert.match(html, /data-open=""/);
  assert.match(html, /tool-status-failed/);
  assert.match(html, /tool-risk-badge-risk/);
  assert.match(html, /data-slot="hover-card-trigger"/);
  assert.match(html, /Inspect card/);
  assert.match(html, /data-action="inspectToolDetails"/);
});

test("renders pending tools with pending status chrome", () => {
  const html = renderCard(
    cardProps({
      title: "Search files",
      toolKind: "search",
      toolStatus: "pending",
      locations: [],
      rawInput: { pattern: "ToolCallCard" },
      content: []
    })
  );

  assert.match(html, /tool-status-pending/);
  assert.match(html, />pending</);
  assert.match(html, /tool-kind-query/);
  assert.match(html, /data-slot="hover-card-trigger"/);
});

test("renders empty tool calls without phantom content wrappers", () => {
  const html = renderCard(
    cardProps({
      title: "Provider tool",
      toolKind: "other",
      toolStatus: "completed",
      locations: [],
      content: []
    })
  );

  assert.match(html, /data-tool-card/);
  assert.match(html, /Provider tool/);
  assert.doesNotMatch(html, /class="tool-content"/);
  assert.doesNotMatch(html, /tool-card-actions|tool-action-bar/);
});

test("renders raw details with shadcn accordion while preserving helper content selectors", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Fetch docs",
        toolKind: "fetch",
        toolStatus: "completed",
        locations: [],
        rawInput: { url: "https://example.com/docs" },
        content: []
      },
      {
        contentHtml: '<div class="tool-content"><a href="https://example.com/docs">docs</a></div>',
        rawDetails: {
          id: "node-1-raw",
          label: "Details",
          contentHtml: '<pre class="code-frame">{"ok":true}</pre>',
          defaultOpen: true
        }
      }
    )
  );

  assert.match(html, /class="tool-content"/);
  assert.match(html, /class="[^"]*raw raw-accordion/);
  assert.match(html, /data-slot="accordion"/);
  assert.match(html, /data-slot="accordion-trigger"/);
  assert.match(html, /data-slot="accordion-content"/);
  assert.match(html, /code-frame/);
  assert.match(html, /Details/);
  assert.doesNotMatch(html, /<details class="raw"/);
  assert.doesNotMatch(html, /<summary/);
});

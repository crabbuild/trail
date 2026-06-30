import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  ToolCallGroupCard,
  type ToolCallGroupCardProps
} from "../webview/ToolCallGroupCard";
import type { ToolCallCardProps } from "../webview/ToolCallCard";
import { summarizeToolCallGroup } from "../webview/toolCallGroupSummary";
import { buildToolPresentation, type ToolPresentationInput } from "../webview/toolModel";

const noopCallbacks = {
  onOpenLocation() {
    // Render tests only verify markup; click behavior is wired by the webview island.
  }
};

function cardProps(
  id: string,
  input: ToolPresentationInput,
  overrides: Partial<ToolCallCardProps> = {}
): ToolCallCardProps {
  const presentation = buildToolPresentation(input);
  return {
    nodeId: id,
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
      openByDefault: presentation.openByDefault,
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
    ...overrides
  };
}

function groupProps(overrides: Partial<ToolCallGroupCardProps> = {}): ToolCallGroupCardProps {
  const items = [
    cardProps("tool-read", {
      title: "Read package.json",
      toolKind: "read",
      toolStatus: "completed",
      locations: [{ path: "package.json" }],
      content: [{ type: "content", content: { type: "text", text: "{}" } }]
    }),
    cardProps("tool-search", {
      title: "Search source",
      toolKind: "search",
      toolStatus: "completed",
      locations: [],
      rawInput: { pattern: "ToolCallGroupCard" },
      content: []
    })
  ];
  return {
    id: "tool-group-tool-read",
    title: "Read a file and searched code",
    detail: "2 tool calls: Read 1 / Search 1",
    status: "completed",
    statusLabel: "done",
    items,
    ...overrides
  };
}

function renderGroup(props: ToolCallGroupCardProps): string {
  return renderToStaticMarkup(
    React.createElement(ToolCallGroupCard, {
      props,
      callbacks: noopCallbacks
    })
  );
}

test("renders completed tool calls as one minimal shadcn collapsible summary", () => {
  const html = renderGroup(groupProps());

  assert.match(html, /data-slot="collapsible"/);
  assert.match(html, /data-slot="collapsible-trigger"/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /data-tool-call-group=""/);
  assert.match(html, /Read a file and searched code/);
  assert.match(html, /aria-label="Expand Read a file and searched code\. 2 tool calls: Read 1 \/ Search 1"/);
  assert.match(html, /bg-muted\/10/);
  assert.match(html, /hover:bg-muted\/20/);
  assert.doesNotMatch(html, /Completed tool calls/);
  assert.doesNotMatch(html, /tool-group-preview-item/);
  assert.doesNotMatch(html, /data-slot="badge"/);
  assert.doesNotMatch(html, /tool-kind-file|tool-kind-query/);
});

test("keeps risk and individual tool details hidden while collapsed", () => {
  const html = renderGroup(
    groupProps({
      status: "failed",
      statusLabel: "failed",
      items: [
        cardProps("tool-delete", {
          title: "Delete generated file",
          toolKind: "delete",
          toolStatus: "failed",
          locations: [{ path: "tmp/generated.txt" }],
          content: []
        }),
        cardProps("tool-read", {
          title: "Read package.json",
          toolKind: "read",
          toolStatus: "completed",
          locations: [{ path: "package.json" }],
          content: []
        })
      ]
    })
  );

  assert.doesNotMatch(html, /tool-risk-badge-risk/);
  assert.doesNotMatch(html, /1 risk/);
  assert.doesNotMatch(html, /tool-status-failed/);
  assert.doesNotMatch(html, /data-tool-card/);
});

test("summarizes grouped tools by completed activity", () => {
  assert.equal(
    summarizeToolCallGroup([
      cardProps("tool-search", {
        title: "Search source",
        toolKind: "search",
        toolStatus: "completed",
        locations: [],
        rawInput: { pattern: "ToolCallGroupCard" },
        content: []
      }),
      cardProps("tool-test", {
        title: "Run tests",
        toolKind: "execute",
        toolStatus: "completed",
        locations: [],
        rawInput: { command: "npm test" },
        content: []
      }),
      cardProps("tool-log", {
        title: "Run git log",
        toolKind: "execute",
        toolStatus: "completed",
        locations: [],
        rawInput: { command: "git log --oneline -1" },
        content: []
      })
    ]).title,
    "Searched code, ran 2 commands"
  );

  assert.equal(
    summarizeToolCallGroup([
      cardProps("tool-read-a", {
        title: "Read package.json",
        toolKind: "read",
        toolStatus: "completed",
        locations: [{ path: "package.json" }],
        content: []
      }),
      cardProps("tool-read-b", {
        title: "Read tsconfig.json",
        toolKind: "read",
        toolStatus: "completed",
        locations: [{ path: "tsconfig.json" }],
        content: []
      }),
      cardProps("tool-read-c", {
        title: "Read README.md",
        toolKind: "read",
        toolStatus: "completed",
        locations: [{ path: "README.md" }],
        content: []
      })
    ]).title,
    "Read 3 files"
  );

  assert.equal(
    summarizeToolCallGroup([
      cardProps("tool-read", {
        title: "Read package.json",
        toolKind: "read",
        toolStatus: "completed",
        locations: [{ path: "package.json" }],
        content: []
      }),
      cardProps("tool-list", {
        title: "Bash",
        toolKind: "execute",
        toolStatus: "completed",
        locations: [],
        rawInput: { command: "ls -la src" },
        content: []
      }),
      cardProps("tool-run", {
        title: "Run npm test",
        toolKind: "execute",
        toolStatus: "completed",
        locations: [],
        rawInput: { command: "npm test" },
        content: []
      })
    ]).title,
    "Read a file and listed files, ran a command"
  );

  assert.equal(
    summarizeToolCallGroup([
      cardProps("tool-edit-a", {
        title: "Edit package.json",
        toolKind: "edit",
        toolStatus: "completed",
        locations: [{ path: "package.json" }],
        content: []
      }),
      cardProps("tool-edit-b", {
        title: "Edit README.md",
        toolKind: "edit",
        toolStatus: "completed",
        locations: [{ path: "README.md" }],
        content: []
      }),
      cardProps("tool-read", {
        title: "Read package.json",
        toolKind: "read",
        toolStatus: "completed",
        locations: [{ path: "package.json" }],
        content: []
      }),
      cardProps("tool-search", {
        title: "Search source",
        toolKind: "search",
        toolStatus: "completed",
        locations: [],
        rawInput: { pattern: "ToolCallGroupCard" },
        content: []
      })
    ]).title,
    "Edited 2 files, read a file and searched code"
  );

  assert.equal(
    summarizeToolCallGroup([
      cardProps("tool-read", {
        title: "Read package.json",
        toolKind: "read",
        toolStatus: "completed",
        locations: [{ path: "package.json" }],
        content: []
      }),
      cardProps("tool-web", {
        title: "Web search",
        toolKind: "search",
        toolStatus: "completed",
        locations: [],
        rawInput: { query: "shadcn collapsible" },
        content: []
      })
    ]).title,
    "Read a file, searched the web"
  );
});

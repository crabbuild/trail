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
        toolStatus: "in_progress",
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

test("renders read permissions as a clean approval prompt", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Read /Users/haipingfu/Github/vscode-acp/src/core/AgentManager.ts",
        toolKind: "read",
        toolStatus: "pending",
        locations: [{ path: "/Users/haipingfu/Github/vscode-acp/src/core/AgentManager.ts" }],
        rawInput: { path: "/Users/haipingfu/Github/vscode-acp/src/core/AgentManager.ts" },
        rawOutput: { ok: true },
        content: []
      },
      {
        model: {
          ...buildToolPresentation({
            title: "Read /Users/haipingfu/Github/vscode-acp/src/core/AgentManager.ts",
            toolKind: "read",
            toolStatus: "pending",
            locations: [{ path: "/Users/haipingfu/Github/vscode-acp/src/core/AgentManager.ts" }],
            rawInput: { path: "/Users/haipingfu/Github/vscode-acp/src/core/AgentManager.ts" },
            content: []
          }),
          statusLabel: "Needs decision",
          riskLabel: "Needs approval",
          riskTone: "warning"
        },
        approval: {
          requestId: "request-read",
          status: "pending",
          statusLabel: "Needs decision",
          tone: "info",
          resolved: false,
          title: "Permission required",
          resolvedNote: "",
          impactText: "Read 1 affected location.",
          actions: [
            {
              kind: "approve",
              optionId: "allow",
              label: "Allow",
              description: "Allow read-only context.",
              tone: "primary",
              disabled: false
            },
            {
              kind: "approve",
              optionId: "allow_always",
              label: "Always allow",
              description: "Always allow this read.",
              tone: "primary",
              disabled: false
            },
            {
              kind: "reject",
              label: "Reject",
              description: "Do not allow this action.",
              tone: "risk",
              disabled: false
            }
          ]
        }
      }
    )
  );

  assert.match(html, /tool-approval-read/);
  assert.match(html, /Read AgentManager\.ts/);
  assert.match(html, /tool-meta-icon-badge/);
  assert.match(html, /aria-label="Needs decision"/);
  assert.match(html, /Allow read\?/);
  assert.match(html, /AgentManager\.ts/);
  assert.match(html, /<div class="approval-decision"[\s\S]*Allow[\s\S]*Always allow[\s\S]*data-action="reject"[\s\S]*Reject/);
  assert.doesNotMatch(html, /Read 1 affected location|Permission required/);
  assert.doesNotMatch(html, /Open path|Inspect details|inspectToolDetails|tool-card-actions/);
  assert.doesNotMatch(html, /tool-evidence-strip|tool-facts|tool-locations|data-slot="breadcrumb"/);
  assert.doesNotMatch(html, />Details</);
  assert.doesNotMatch(html, /code-frame|data-slot="context-menu-trigger"/);
});

test("renders think tools as clean content notes", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Find extension API surface",
        toolKind: "think",
        toolStatus: "pending",
        locations: [],
        rawInput: { prompt: "Find exported API surface" },
        rawOutput: { ok: true },
        content: [{ type: "content", content: { type: "text", text: "Search extension entry points." } }]
      },
      {
        contentHtml: "<p>Search extension entry points.</p>"
      }
    )
  );

  assert.match(html, /Find extension API surface/);
  assert.match(html, /Thinking/);
  assert.match(html, /think-tool-content/);
  assert.match(html, /Search extension entry points/);
  assert.match(html, /tool-status-pending/);
  assert.doesNotMatch(html, /1 output|tool-evidence-strip/);
  assert.doesNotMatch(html, /tool-stat(?:\s|-)/);
  assert.doesNotMatch(html, /Inspect details|inspectToolDetails|tool-card-actions/);
  assert.doesNotMatch(html, />Details</);
  assert.doesNotMatch(html, /code-frame/);
  assert.doesNotMatch(html, /tool-facts|tool-locations|data-slot="context-menu-trigger"/);
  assert.doesNotMatch(html, /tool-kind/);
});

test("renders edit tools as a compact lifecycle plus inline diff", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Edit README.md",
        toolKind: "edit",
        toolStatus: "in_progress",
        locations: [{ path: "README.md" }],
        rawInput: { path: "README.md", line: 12 },
        content: [{ type: "diff", path: "README.md", oldText: "old", newText: "new" }]
      },
      {
        contentHtml: '<section class="diff-preview diff-preview-compact" tabindex="0"><div class="diff-grid">diff</div></section>'
      }
    )
  );

  assert.match(html, /tool-kind-change/);
  assert.match(html, /Workspace change|change/);
  assert.match(html, /edit-lifecycle-ready/);
  assert.match(html, /Review changes/);
  assert.match(html, /Diff preview for README.md is ready below/);
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
  assert.doesNotMatch(html, /data-slot="context-menu-trigger"/);
  assert.doesNotMatch(html, /data-inline-actions/);
  assert.doesNotMatch(html, /tool-card-actions/);
  assert.doesNotMatch(html, /tool-action-bar/);
  assert.match(html, /data-icon="inline-start"/);
  assert.doesNotMatch(
    html,
    /data-action="(?:focusToolDiff|openLocation|inspectToolDetails)"[\s\S]{0,260}<svg[^>]*class="icon"/
  );
  assert.doesNotMatch(html, /title="Show this diff preview\."/);
  assert.doesNotMatch(html, /data-slot="breadcrumb"/);
  assert.doesNotMatch(html, /tool-locations/);
  assert.doesNotMatch(html, /class="chips/);
  assert.doesNotMatch(html, /data-slot="breadcrumb-list"/);
  assert.doesNotMatch(html, /data-slot="breadcrumb-page"/);
  assert.doesNotMatch(html, /class="[^"]*tool-fact/);
  assert.doesNotMatch(html, /tool-fact-separator/);
  assert.doesNotMatch(html, /<dl class="tool-facts|<dt>|<dd>/);
  assert.doesNotMatch(html, />Details</);
  assert.match(html, /diff-preview-compact/);
  assert.match(html, /diff-grid/);
  assert.doesNotMatch(html, /diff-preview-toolbar|diff-preview-meta|diff-preview-actions|code-title|code-language|diff-stat/);
});

test("renders edit tools without content as an active lifecycle state", () => {
  const html = renderCard(
    cardProps({
      title: "Edit README.md",
      toolKind: "edit",
      toolStatus: "in_progress",
      locations: [{ path: "README.md" }],
      rawInput: { path: "README.md" },
      content: []
    })
  );

  assert.match(html, /edit-lifecycle-running/);
  assert.match(html, /edit-lifecycle-loading-icon/);
  assert.match(html, /Preparing edit/);
  assert.match(html, /Generating changes for README.md\./);
  assert.doesNotMatch(html, /No rendered output/);
  assert.doesNotMatch(html, /class="tool-content"/);
});

test("renders completed edit tools without preview as a finished receipt", () => {
  const html = renderCard(
    cardProps({
      title: "Edit README.md",
      toolKind: "edit",
      toolStatus: "completed",
      locations: [{ path: "README.md" }],
      rawInput: { path: "README.md" },
      content: []
    })
  );

  assert.match(html, /edit-lifecycle-recorded/);
  assert.match(html, /Edit complete/);
  assert.match(html, /No diff preview was returned for README.md/);
  assert.doesNotMatch(html, /No rendered output/);
  assert.doesNotMatch(html, />Details</);
  assert.doesNotMatch(html, /code-frame/);
});

test("renders edit tools waiting for approval as an approval lifecycle state", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Edit README.md",
        toolKind: "edit",
        toolStatus: "pending",
        locations: [{ path: "README.md" }],
        rawInput: { path: "README.md" },
        content: []
      },
      {
        model: {
          ...buildToolPresentation({
            title: "Edit README.md",
            toolKind: "edit",
            toolStatus: "pending",
            locations: [{ path: "README.md" }],
            rawInput: { path: "README.md" },
            content: []
          }),
          statusLabel: "Needs decision",
          riskLabel: "Needs approval",
          riskTone: "warning"
        },
        approval: {
          requestId: "request-edit",
          status: "pending",
          statusLabel: "Needs decision",
          tone: "warning",
          resolved: false,
          title: "Permission required",
          resolvedNote: "",
          impactText: "Edit 1 affected location.",
          actions: [
            {
              kind: "approve",
              optionId: "allow_always",
              label: "Always allow",
              description: "Always allow this edit.",
              tone: "warning",
              disabled: false
            },
            {
              kind: "approve",
              optionId: "allow",
              label: "Allow",
              description: "Allow edit after reviewing preview.",
              tone: "warning",
              disabled: false
            },
            {
              kind: "reject",
              label: "Reject",
              description: "Do not allow this action.",
              tone: "risk",
              disabled: false
            }
          ]
        }
      }
    )
  );

  assert.doesNotMatch(html, /edit-lifecycle-approval/);
  assert.match(html, /tool-approval-edit/);
  assert.match(html, /Approve edit\?/);
  assert.match(html, /README.md/);
  assert.match(html, /Always allow/);
  assert.match(html, /<div class="approval-decision"[\s\S]*class="[^"]*approval-option-list[\s\S]*Always allow[\s\S]*Allow[\s\S]*data-action="reject"[\s\S]*Reject/);
  assert.match(html, /tool-approval/);
  assert.doesNotMatch(html, /Permission required/);
  assert.doesNotMatch(html, /Edit 1 affected location/);
  assert.doesNotMatch(html, /tool-card-actions|tool-facts|tool-locations|approval-meta|approval-detail-list|approval-request-details/);
  assert.doesNotMatch(html, />Details</);
  assert.doesNotMatch(html, /No rendered output/);
});

test("renders execute tools as terminal-focused cards", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Run tests",
        toolKind: "execute",
        toolStatus: "in_progress",
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

test("renders KiloCode background process tools as structured process panels", () => {
  const html = renderCard(
    cardProps(
      {
        title: "background_process",
        toolKind: "other",
        toolStatus: "completed",
        locations: [],
        rawInput: {
          name: "background_process",
          input: {
            action: "start",
            command: "npm run dev",
            description: "Start local dev server"
          }
        },
        content: []
      },
      {
        details: {
          kind: "background_process",
          title: "Start background process",
          description: "Start local dev server",
          rows: [
            { label: "Command", value: "npm run dev" },
            { label: "Process id", value: "dev-server" },
            { label: "Status", value: "running" }
          ],
          output: "Listening on :5173",
          outputLabel: "Output"
        }
      }
    )
  );

  assert.match(html, /data-open=""/);
  assert.match(html, /tool-structured-background_process/);
  assert.match(html, /Start background process/);
  assert.match(html, /Start local dev server/);
  assert.match(html, /Command/);
  assert.match(html, /npm run dev/);
  assert.match(html, /Process id/);
  assert.match(html, /Listening on :5173/);
  assert.doesNotMatch(html, /raw-accordion|tool-facts|tool-card-actions/);
});

test("renders KiloCode task tools as delegated agent panels", () => {
  const html = renderCard(
    cardProps(
      {
        title: "task",
        toolKind: "other",
        toolStatus: "pending",
        locations: [],
        rawInput: {
          toolName: "task",
          input: {
            subagent_type: "reviewer",
            description: "Inspect the tool-call UI"
          }
        },
        content: []
      },
      {
        details: {
          kind: "task",
          title: "Agent task (reviewer)",
          description: "Inspect the tool-call UI",
          rows: [
            { label: "Agent", value: "reviewer" },
            { label: "Status", value: "pending" }
          ],
          output: "No issues found.",
          outputLabel: "Result"
        }
      }
    )
  );

  assert.match(html, /tool-structured-task/);
  assert.match(html, /Agent task \(reviewer\)/);
  assert.match(html, /Inspect the tool-call UI/);
  assert.match(html, /No issues found/);
  assert.match(html, /tool-kind-agent/);
  assert.match(html, /tool-risk-badge-warning/);
  assert.doesNotMatch(html, /raw-accordion|tool-facts|tool-card-actions/);
});

test("renders failed tools collapsed with status icons only", () => {
  const html = renderCard(
    cardProps({
      title: "Delete generated file",
      toolKind: "delete",
      toolStatus: "failed",
      locations: [{ path: "tmp/generated.txt" }],
      content: []
    })
  );

  assert.doesNotMatch(html, /data-open=""/);
  assert.match(html, /tool-status-failed/);
  assert.match(html, /tool-risk-badge-risk/);
  assert.match(html, /data-slot="hover-card-trigger"/);
  assert.doesNotMatch(html, /Inspect card/);
  assert.doesNotMatch(html, /data-action="inspectToolDetails"/);
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
  assert.match(html, /aria-label="pending"/);
  assert.doesNotMatch(html, /tool-status-pending[^>]*>pending<\/span>/);
  assert.match(html, /tool-kind-query/);
  assert.match(html, /data-slot="hover-card-trigger"/);
});

test("renders permission decisions without pre-approval tool output", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Run git log",
        toolKind: "execute",
        toolStatus: "pending",
        locations: [],
        rawInput: { command: "git log --oneline -10" },
        content: [{ type: "terminal", terminalId: "term-1", command: "git log --oneline -10" }]
      },
      {
        terminal: true,
        model: {
          ...buildToolPresentation({
            title: "Run git log",
            toolKind: "execute",
            toolStatus: "pending",
            locations: [],
            rawInput: { command: "git log --oneline -10" },
            content: [{ type: "terminal", terminalId: "term-1", command: "git log --oneline -10" }]
          }),
          statusLabel: "Needs decision",
          riskLabel: "Needs approval",
          riskTone: "warning"
        },
        contentHtml: '<div class="terminal-transcript">git log --oneline -10</div>',
        approval: {
          requestId: "request-1",
          status: "pending",
          statusLabel: "Needs decision",
          tone: "warning",
          resolved: false,
          title: "Permission required",
          resolvedNote: "",
          impactText: "The agent is asking to run a command that can inspect or change the current task.",
          actions: [
            {
              kind: "approve",
              optionId: "allow_always",
              label: "Always allow",
              description: "Always allow this command.",
              tone: "warning",
              disabled: false
            },
            {
              kind: "approve",
              optionId: "allow",
              label: "Allow",
              description: "Allow once.",
              tone: "warning",
              disabled: false
            },
            {
              kind: "reject",
              label: "Reject",
              description: "Do not allow this action.",
              tone: "risk",
              disabled: false
            }
          ]
        }
      }
    )
  );

  assert.match(html, /tool-approval/);
  assert.match(html, /Always allow/);
  assert.match(html, /data-action="approve"/);
  assert.match(html, /data-option-id="allow_always"/);
  assert.match(html, /data-option-id="allow"/);
  assert.match(html, /data-action="reject"/);
  assert.doesNotMatch(html, /terminal-transcript/);
  assert.doesNotMatch(html, /No terminal output preview/);
  assert.match(html, /lucide-shield-check/);
  assert.match(html, /lucide-check/);
  assert.match(html, /lucide-circle-x/);
  assert.doesNotMatch(html, /approval-meta/);
  assert.doesNotMatch(html, /claude-code/);
  assert.doesNotMatch(html, /No file scope reported/);
  assert.doesNotMatch(html, /approval-request-details/);
  assert.doesNotMatch(html, />Details</);
  assert.doesNotMatch(html, /data-approval-card/);
});

test("renders tool output after permission is resolved", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Run git log",
        toolKind: "execute",
        toolStatus: "in_progress",
        locations: [],
        rawInput: { command: "git log --oneline -10" },
        content: [{ type: "terminal", terminalId: "term-1", command: "git log --oneline -10" }]
      },
      {
        terminal: true,
        contentHtml: '<div class="terminal-transcript">git log --oneline -10</div>',
        approval: {
          requestId: "request-1",
          status: "completed",
          statusLabel: "Allowed",
          tone: "success",
          resolved: true,
          title: "Permission required",
          resolvedNote: "Allowed once.",
          impactText: "The agent is asking to run a command that can inspect or change the current task.",
          actions: [
            {
              kind: "approve",
              optionId: "allow",
              label: "Allow",
              description: "Allow once.",
              tone: "warning",
              disabled: false
            },
            {
              kind: "reject",
              label: "Reject",
              description: "Do not allow this action.",
              tone: "risk",
              disabled: false
            }
          ]
        }
      }
    )
  );

  assert.match(html, /tool-approval-resolved/);
  assert.match(html, /Allowed once/);
  assert.match(html, /terminal-transcript/);
  assert.doesNotMatch(html, /data-action="approve"/);
  assert.doesNotMatch(html, /data-action="reject"/);
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

test("omits raw details while preserving rendered tool content", () => {
  const html = renderCard(
    cardProps(
      {
        title: "Fetch docs",
        toolKind: "fetch",
        toolStatus: "in_progress",
        locations: [],
        rawInput: { url: "https://example.com/docs" },
        content: []
      },
      {
        contentHtml: '<div class="tool-content"><a href="https://example.com/docs">docs</a></div>'
      }
    )
  );

  assert.match(html, /class="tool-content"/);
  assert.doesNotMatch(html, /class="[^"]*raw raw-accordion/);
  assert.doesNotMatch(html, /data-slot="accordion-trigger"/);
  assert.doesNotMatch(html, /data-slot="accordion-content"/);
  assert.doesNotMatch(html, /code-frame/);
  assert.doesNotMatch(html, />Details</);
  assert.doesNotMatch(html, /<details class="raw"/);
  assert.doesNotMatch(html, /<summary/);
});

import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ApprovalCard, type ApprovalCardProps } from "../webview/ApprovalCard";

const iconHtml = '<svg class="icon" aria-hidden="true"></svg>';

function renderApproval(props: ApprovalCardProps): string {
  return renderToStaticMarkup(React.createElement(ApprovalCard, { props }));
}

function baseProps(overrides: Partial<ApprovalCardProps> = {}): ApprovalCardProps {
  return {
    nodeId: "approval-1",
    requestId: "request-1",
    status: "pending",
    statusLabel: "Needs decision",
    tone: "warning",
    resolved: false,
    summaryIconHtml: iconHtml,
    title: "Permission required",
    detail: "Edit README.md · 1 affected location",
    resolvedNote: "",
    impactText: "The agent is asking to edit 1 affected location.",
    meta: [
      { iconHtml, label: "Edit" },
      { label: "1 affected location" },
      { label: "provider" }
    ],
    locationsHtml: '<div class="chips approval-locations"><span>README.md</span></div>',
    preview: {
      id: "approval-preview",
      title: "Preview",
      meta: "1 block",
      className: "approval-preview approval-section",
      contentClassName: "tool-content approval-tool-content",
      contentHtml: '<div class="media-preview-body">Preview body</div>',
      defaultOpen: true
    },
    requestDetails: {
      id: "approval-request-details",
      title: "Details",
      className: "approval-request-details",
      contentClassName: "approval-request-content",
      contentHtml: '<dl class="approval-detail-list"><div><dt>Provider</dt><dd>provider</dd></div></dl>',
      defaultOpen: false
    },
    actions: [
      {
        kind: "approve",
        optionId: "allow",
        label: "Allow",
        description: "Allow edit after reviewing preview.",
        tone: "warning",
        iconHtml,
        disabled: false
      },
      {
        kind: "reject",
        label: "Reject",
        description: "Do not allow this action.",
        tone: "risk",
        iconHtml,
        disabled: false
      }
    ],
    ...overrides
  };
}

test("renders pending approvals with compact shadcn card badge and decision buttons", () => {
  const html = renderApproval(baseProps());

  assert.match(html, /data-approval-card/);
  assert.match(html, /data-slot="card"/);
  assert.match(html, /approval-impact/);
  assert.match(html, /data-slot="accordion"/);
  assert.match(html, /data-slot="accordion-item"/);
  assert.match(html, /data-slot="accordion-trigger"/);
  assert.match(html, /data-slot="accordion-content"/);
  assert.match(html, /data-slot="badge"/);
  assert.match(html, /tool-status-pending/);
  assert.match(html, /data-slot="button"/);
  assert.match(html, /data-action="approve"/);
  assert.match(html, /data-option-id="allow"/);
  assert.match(html, /data-action="reject"/);
  assert.match(html, /data-icon="inline-start"/);
  assert.match(html, /approval-preview/);
  assert.match(html, /approval-request-details/);
  assert.match(html, /approval-tool-content/);
  assert.match(html, /approval-request-content/);
  assert.doesNotMatch(html, /class="icon" data-icon="inline-start"/);
  assert.doesNotMatch(html, /data-slot="alert"/);
  assert.doesNotMatch(html, /<details class="approval-preview/);
  assert.doesNotMatch(html, /<summary>Preview/);
  assert.doesNotMatch(html, /<details class="approval-request-details/);
});

test("renders resolved approvals as compact receipts without decision buttons", () => {
  const html = renderApproval(
    baseProps({
      status: "completed",
      statusLabel: "Approved",
      tone: "success",
      resolved: true,
      title: "Permission approved",
      resolvedNote: "Edit allowed for 1 affected location from provider.",
      actions: []
    })
  );

  assert.match(html, /approval-card-resolved/);
  assert.match(html, /approval-resolved-note/);
  assert.match(html, /Edit allowed/);
  assert.doesNotMatch(html, /data-action="approve"/);
  assert.doesNotMatch(html, /data-action="reject"/);
  assert.doesNotMatch(html, /tool-status-completed/);
});

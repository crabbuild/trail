import assert from "node:assert/strict";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MessageCard, type MessageCardProps } from "../webview/MessageCard";

function renderMessage(props: MessageCardProps): string {
  return renderToStaticMarkup(React.createElement(MessageCard, { props }));
}

test("renders assistant messages with shadcn message structure", () => {
  const html = renderMessage({
    nodeId: "message-1",
    role: "assistant",
    streaming: false,
    contentHtml: '<p>Hello <strong>world</strong>.</p>'
  });

  assert.match(html, /data-message-card/);
  assert.match(html, /data-slot="message"/);
  assert.match(html, /data-slot="message-avatar"/);
  assert.match(html, /class="[^"]*message-avatar[^"]*message-avatar-assistant/);
  assert.match(html, /data-slot="marker"/);
  assert.match(html, /data-slot="marker-content"/);
  assert.match(html, /data-align="start"/);
  assert.match(html, /class="[^"]*message-header/);
  assert.match(html, /class="[^"]*message-role-marker/);
  assert.doesNotMatch(html, /class="role(?:\s|")|class="[^"]*\srole(?:\s|")/);
  assert.doesNotMatch(html, /card-chrome/);
  assert.match(html, />Agent</);
  assert.match(html, />AI</);
  assert.match(html, /class="markdown"/);
  assert.match(html, /<strong>world<\/strong>/);
});

test("renders user messages aligned to the end", () => {
  const html = renderMessage({
    nodeId: "message-2",
    role: "user",
    streaming: false,
    contentHtml: "<p>Run the checks.</p>"
  });

  assert.match(html, /data-align="end"/);
  assert.match(html, /class="[^"]*message-avatar[^"]*message-avatar-user/);
  assert.match(html, /data-slot="marker"/);
  assert.match(html, />You</);
  assert.doesNotMatch(html, /streaming/);
});

test("renders streaming state with shadcn badge affordance", () => {
  const html = renderMessage({
    nodeId: "message-3",
    role: "assistant",
    streaming: true,
    contentHtml: "<p>Working</p>"
  });

  assert.match(html, /data-slot="badge"/);
  assert.match(html, /data-slot="spinner"/);
  assert.match(html, /class="[^"]*message-streaming-badge/);
  assert.match(html, /data-slot="marker-icon"/);
  assert.match(html, /role="status"/);
  assert.match(html, /class="[^"]*message-streaming-status/);
  assert.match(html, />streaming</);
  assert.match(html, /Live response/);
});

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
  assert.match(html, /data-align="start"/);
  assert.match(html, /class="[^"]*transcript-message-assistant/);
  assert.doesNotMatch(html, /class="role(?:\s|")|class="[^"]*\srole(?:\s|")/);
  assert.doesNotMatch(html, /card-chrome/);
  assert.doesNotMatch(html, />Agent</);
  assert.doesNotMatch(html, />AI</);
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
  assert.match(html, /class="[^"]*transcript-message-user-bg/);
  assert.doesNotMatch(html, /data-slot="message-avatar"/);
  assert.doesNotMatch(html, />You</);
  assert.doesNotMatch(html, /streaming/);
});

test("renders streaming state without visible badge chrome", () => {
  const html = renderMessage({
    nodeId: "message-3",
    role: "assistant",
    streaming: true,
    contentHtml: "<p>Working</p>"
  });

  assert.match(html, /role="status"/);
  assert.match(html, /Streaming response/);
  assert.doesNotMatch(html, /data-slot="badge"/);
  assert.doesNotMatch(html, /data-slot="spinner"/);
  assert.doesNotMatch(html, />streaming</);
});

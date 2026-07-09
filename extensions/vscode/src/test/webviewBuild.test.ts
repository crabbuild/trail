import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const root = process.cwd();
const webviewDist = path.join(root, "dist", "webview");
const extensionScript = path.join(root, "dist", "extension.js");
const mainScript = path.join(webviewDist, "main.js");
const mainStyle = path.join(webviewDist, "main.css");
const chunksDir = path.join(webviewDist, "chunks");

function sourceWithSharedChunks(source: string): string {
  const chunkImports = [...source.matchAll(/(?:from|import)\s*"\.\/(chunk-[A-Z0-9]+\.js)"/g)];
  return chunkImports.reduce((combined, match) => {
    const chunkName = match[1];
    if (!chunkName) {
      return combined;
    }
    const chunkPath = path.join(chunksDir, chunkName);
    return fs.existsSync(chunkPath) ? `${combined}\n${fs.readFileSync(chunkPath, "utf8")}` : combined;
  }, source);
}

function sourceWithJsFunctionSignatures(source: string): string {
  return source.replace(/function\s+([A-Za-z0-9_]+)\(([^)]*)\)\s*(?::\s*[^{]+)?\s*\{/g, (_match, name, params) => {
    const jsParams = String(params)
      .split(",")
      .map((param) => param.trim().replace(/\?\s*:/, ":").replace(/:.*?(?==|$)/, "").replace(/\?$/, "").trim())
      .map((param) => param.replace(/\s*=\s*/g, " = "))
      .join(", ");
    return `function ${name}(${jsParams}) {`;
  });
}

test("webview build keeps chat startup bundle small and lazy-loads highlighting", () => {
  assert.ok(fs.existsSync(mainScript), "webview module entry should exist");
  assert.ok(fs.existsSync(mainStyle), "webview stylesheet should exist");
  assert.ok(fs.existsSync(extensionScript), "extension bundle should exist");
  assert.ok(fs.existsSync(chunksDir), "webview chunk directory should exist");
  assert.equal(fs.existsSync(path.join(root, "dist", "webview.js")), false, "legacy flat webview bundle should not be emitted");

  const mainBytes = fs.statSync(mainScript).size;
  assert.ok(mainBytes < 250 * 1024, `webview entry should keep startup headroom below 250 KiB, got ${mainBytes} bytes`);

  const chunks = fs.readdirSync(chunksDir);
  const approvalCardChunk = chunks.find((file) => /^ApprovalCard-[A-Z0-9]+\.js$/.test(file));
  assert.equal(approvalCardChunk, undefined, "permission approvals should render inside the tool call card instead of a separate lazy card");
  const composerCardChunk = chunks.find((file) => /^ComposerCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(composerCardChunk, "React shadcn composer card should be emitted as a lazy chunk");
  const highlightChunk = chunks.find((file) => /^highlight-[A-Z0-9]+\.js$/.test(file));
  assert.ok(highlightChunk, "Shiki highlighter should be emitted as a lazy chunk");
  const emptyStateCardChunk = chunks.find((file) => /^EmptyStateCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(emptyStateCardChunk, "React shadcn empty state should be emitted as a lazy chunk");
  const eventCardChunk = chunks.find((file) => /^EventCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(eventCardChunk, "React shadcn event card should be emitted as a lazy chunk");
  const headerBarChunk = chunks.find((file) => /^HeaderBar-[A-Z0-9]+\.js$/.test(file));
  assert.ok(headerBarChunk, "React shadcn header toolbar should be emitted as a lazy chunk");
  const laneMapDrawerChunk = chunks.find((file) => /^LaneMapDrawer-[A-Z0-9]+\.js$/.test(file));
  assert.ok(laneMapDrawerChunk, "Lane map drawer should be emitted as an on-demand drawer chunk");
  const diffCardChunk = chunks.find((file) => /^DiffCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(diffCardChunk, "React shadcn diff card should be emitted as a lazy chunk");
  const diffEnhancerChunk = chunks.find((file) => /^diffEnhancer-[A-Z0-9]+\.js$/.test(file));
  assert.ok(diffEnhancerChunk, "Diffs.com renderer should be emitted as a lazy chunk");
  const diffReviewChunk = chunks.find((file) => /^diffReviewDrawer-[A-Z0-9]+\.js$/.test(file));
  assert.ok(diffReviewChunk, "diff review drawer should be emitted as a lazy chunk");
  const markdownChunk = chunks.find((file) => /^markdownModel-[A-Z0-9]+\.js$/.test(file));
  assert.ok(markdownChunk, "rich markdown rendering should be emitted as a lazy chunk");
  const messageCardChunk = chunks.find((file) => /^MessageCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(messageCardChunk, "React shadcn message card should be emitted as a lazy chunk");
  const payloadDisclosureChunk = chunks.find((file) => /^PayloadDisclosure-[A-Z0-9]+\.js$/.test(file));
  assert.ok(payloadDisclosureChunk, "React shadcn payload disclosure should be emitted as a lazy chunk");
  const planCardChunk = chunks.find((file) => /^PlanCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(planCardChunk, "React shadcn plan card should be emitted as a lazy chunk");
  const recoveryBannerChunk = chunks.find((file) => /^RecoveryBanner-[A-Z0-9]+\.js$/.test(file));
  assert.ok(recoveryBannerChunk, "React shadcn recovery banner should be emitted as a lazy chunk");
  const resultDrawerChunk = chunks.find((file) => /^ResultDrawer-[A-Z0-9]+\.js$/.test(file));
  assert.ok(resultDrawerChunk, "React shadcn result drawer should be emitted as a lazy chunk");
  const reviewDrawerChunk = chunks.find((file) => /^ReviewDrawer-[A-Z0-9]+\.js$/.test(file));
  assert.ok(reviewDrawerChunk, "React shadcn review drawer should be emitted as a lazy chunk");
  const terminalCardChunk = chunks.find((file) => /^TerminalCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(terminalCardChunk, "React shadcn terminal card should be emitted as a lazy chunk");
  const thoughtCardChunk = chunks.find((file) => /^ThoughtCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(thoughtCardChunk, "React shadcn thought card should be emitted as a lazy chunk");
  const timelineGroupChunk = chunks.find((file) => /^TimelineGroup-[A-Z0-9]+\.js$/.test(file));
  assert.ok(timelineGroupChunk, "React shadcn timeline group should be emitted as a lazy chunk");
  const timelineNavigationChunk = chunks.find((file) => /^TimelineNavigation-[A-Z0-9]+\.js$/.test(file));
  assert.ok(timelineNavigationChunk, "React shadcn timeline navigation should be emitted as a lazy chunk");
  const inlineActionsChunk = chunks.find((file) => /^InlineActions-[A-Z0-9]+\.js$/.test(file));
  assert.ok(inlineActionsChunk, "React shadcn inline actions should be emitted as a lazy chunk");
  const timelineScrollerChunk = chunks.find((file) => /^TimelineScroller-[A-Z0-9]+\.js$/.test(file));
  assert.ok(timelineScrollerChunk, "React shadcn timeline scroller should be emitted as a lazy chunk");
  const toolCallCardChunk = chunks.find((file) => /^ToolCallCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(toolCallCardChunk, "React shadcn tool call card should be emitted as a lazy chunk");
  const toolCallGroupCardChunk = chunks.find((file) => /^ToolCallGroupCard-[A-Z0-9]+\.js$/.test(file));
  assert.ok(toolCallGroupCardChunk, "React shadcn tool call group should be emitted as a lazy chunk");

  const mainBundleSource = fs.readFileSync(mainScript, "utf8");
  const webviewSource = fs.readFileSync(path.join(root, "src", "webview", "main.ts"), "utf8");
  const mainSource = sourceWithJsFunctionSignatures(webviewSource);
  const approvalCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "ApprovalCard.tsx"), "utf8");
  const composerCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "ComposerCard.tsx"), "utf8");
  const composerModelSourceTs = fs.readFileSync(path.join(root, "src", "webview", "composerModel.ts"), "utf8");
  const diffCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "DiffCard.tsx"), "utf8");
  const diffEnhancerSourceTs = fs.readFileSync(path.join(root, "src", "webview", "diffEnhancer.ts"), "utf8");
  const emptyStateCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "EmptyStateCard.tsx"), "utf8");
  const eventCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "EventCard.tsx"), "utf8");
  const headerBarSourceTs = fs.readFileSync(path.join(root, "src", "webview", "HeaderBar.tsx"), "utf8");
  const laneMapDrawerSourceTs = fs.readFileSync(path.join(root, "src", "webview", "LaneMapDrawer.tsx"), "utf8");
  const messageCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "MessageCard.tsx"), "utf8");
  const streamdownMarkdownSourceTs = fs.readFileSync(path.join(root, "src", "webview", "StreamdownMarkdown.tsx"), "utf8");
  const messageScrollerSourceTs = fs.readFileSync(path.join(root, "src", "webview", "components", "ui", "message-scroller.tsx"), "utf8");
  const payloadDisclosureSourceTs = fs.readFileSync(path.join(root, "src", "webview", "PayloadDisclosure.tsx"), "utf8");
  const planCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "PlanCard.tsx"), "utf8");
  const rawDetailsSourceTs = fs.readFileSync(path.join(root, "src", "webview", "RawDetails.tsx"), "utf8");
  const recoveryBannerSourceTs = fs.readFileSync(path.join(root, "src", "webview", "RecoveryBanner.tsx"), "utf8");
  const diffReviewSourceTs = fs.readFileSync(path.join(root, "src", "webview", "diffReviewDrawer.ts"), "utf8");
  const resultDrawerSourceTs = fs.readFileSync(path.join(root, "src", "webview", "ResultDrawer.tsx"), "utf8");
  const reviewDrawerSourceTs = fs.readFileSync(path.join(root, "src", "webview", "ReviewDrawer.tsx"), "utf8");
  const terminalCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "TerminalCard.tsx"), "utf8");
  const thoughtCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "ThoughtCard.tsx"), "utf8");
  const timelineGroupSourceTs = fs.readFileSync(path.join(root, "src", "webview", "TimelineGroup.tsx"), "utf8");
  const timelineModelSourceTs = fs.readFileSync(path.join(root, "src", "webview", "timelineModel.ts"), "utf8");
  const timelineNavigationSourceTs = fs.readFileSync(path.join(root, "src", "webview", "TimelineNavigation.tsx"), "utf8");
  const inlineActionsSourceTs = fs.readFileSync(path.join(root, "src", "webview", "InlineActions.tsx"), "utf8");
  const timelineScrollerSourceTs = fs.readFileSync(path.join(root, "src", "webview", "TimelineScroller.tsx"), "utf8");
  const toolCallCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "ToolCallCard.tsx"), "utf8");
  const toolCallGroupCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "ToolCallGroupCard.tsx"), "utf8");
  const tailwindSource = fs.readFileSync(path.join(root, "src", "webview", "tailwind.css"), "utf8");
  assert.doesNotMatch(mainBundleSource, /import\("\.\/chunks\/ApprovalCard-[A-Z0-9]+\.js"\)/, "main bundle should not dynamically import a separate approval card island");
  assert.match(approvalCardSourceTs, /import \{ flushSync \} from "react-dom"/, "approval card island should commit permission rows synchronously when mounted");
  assert.match(approvalCardSourceTs, /flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<ApprovalCard/, "approval card island should avoid blank permission frames");
  assert.match(mainBundleSource, /import\("\.\/chunks\/ComposerCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React composer island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/DiffCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React diff card island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/highlight-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the highlighter chunk");
  assert.match(mainBundleSource, /import\("\.\/chunks\/EmptyStateCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React empty-state island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/EventCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React event card island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/HeaderBar-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React header toolbar island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/diffEnhancer-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the Diffs.com renderer chunk");
  assert.match(mainBundleSource, /import\("\.\/chunks\/diffReviewDrawer-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the diff review drawer chunk");
  assert.match(mainBundleSource, /import\("\.\/chunks\/markdownModel-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the markdown renderer chunk");
  assert.match(mainBundleSource, /import\("\.\/chunks\/MessageCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React message card island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/PayloadDisclosure-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React payload disclosure island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/PlanCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React plan card island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/RecoveryBanner-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React recovery banner island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/ResultDrawer-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React result drawer island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/ReviewDrawer-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React review drawer island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/TerminalCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React terminal card island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/ThoughtCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React thought card island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/TimelineGroup-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React timeline group island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/TimelineNavigation-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React timeline navigation island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/InlineActions-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React inline actions island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/TimelineScroller-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React timeline scroller island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/ToolCallCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React tool card island");
  assert.match(mainBundleSource, /import\("\.\/chunks\/ToolCallGroupCard-[A-Z0-9]+\.js"\)/, "main bundle should dynamically import the React grouped tool card island");
  assert.doesNotMatch(mainBundleSource, /@shikijs\/langs|createHighlighterCore/, "main bundle should not include Shiki language payloads");
  assert.doesNotMatch(mainBundleSource, /node_modules\/@pierre\/diffs/, "main bundle should not include Diffs.com renderer payloads");
  assert.doesNotMatch(mainBundleSource, /node_modules\/@pierre\/trees/, "main bundle should not include Trees.software renderer payloads");
  const composerCardSource = fs.readFileSync(path.join(chunksDir, composerCardChunk || ""), "utf8");
  const diffCardSource = fs.readFileSync(path.join(chunksDir, diffCardChunk || ""), "utf8");
  const diffEnhancerSource = fs.readFileSync(path.join(chunksDir, diffEnhancerChunk || ""), "utf8");
  const diffReviewSource = fs.readFileSync(path.join(chunksDir, diffReviewChunk || ""), "utf8");
  const emptyStateCardSource = fs.readFileSync(path.join(chunksDir, emptyStateCardChunk || ""), "utf8");
  const eventCardSource = fs.readFileSync(path.join(chunksDir, eventCardChunk || ""), "utf8");
  const headerBarSource = fs.readFileSync(path.join(chunksDir, headerBarChunk || ""), "utf8");
  const highlightSource = fs.readFileSync(path.join(chunksDir, highlightChunk || ""), "utf8");
  const messageCardSource = fs.readFileSync(path.join(chunksDir, messageCardChunk || ""), "utf8");
  const payloadDisclosureSource = fs.readFileSync(path.join(chunksDir, payloadDisclosureChunk || ""), "utf8");
  const planCardSource = fs.readFileSync(path.join(chunksDir, planCardChunk || ""), "utf8");
  const recoveryBannerSource = fs.readFileSync(path.join(chunksDir, recoveryBannerChunk || ""), "utf8");
  const resultDrawerSource = fs.readFileSync(path.join(chunksDir, resultDrawerChunk || ""), "utf8");
  const reviewDrawerSource = fs.readFileSync(path.join(chunksDir, reviewDrawerChunk || ""), "utf8");
  const terminalCardSource = fs.readFileSync(path.join(chunksDir, terminalCardChunk || ""), "utf8");
  const thoughtCardSource = fs.readFileSync(path.join(chunksDir, thoughtCardChunk || ""), "utf8");
  const timelineGroupSource = fs.readFileSync(path.join(chunksDir, timelineGroupChunk || ""), "utf8");
  const timelineNavigationSource = fs.readFileSync(path.join(chunksDir, timelineNavigationChunk || ""), "utf8");
  const inlineActionsSource = fs.readFileSync(path.join(chunksDir, inlineActionsChunk || ""), "utf8");
  const timelineScrollerSource = fs.readFileSync(path.join(chunksDir, timelineScrollerChunk || ""), "utf8");
  const toolCallCardSource = fs.readFileSync(path.join(chunksDir, toolCallCardChunk || ""), "utf8");
  const toolCallGroupCardSource = fs.readFileSync(path.join(chunksDir, toolCallGroupCardChunk || ""), "utf8");
  assert.match(toolCallCardSourceTs, /interface ToolCallApprovalProps/, "tool card should own permission approval props");
  assert.match(toolCallCardSourceTs, /function ToolApprovalPanel/, "tool card should render permission approval controls inline");
  assert.match(toolCallCardSourceTs, /const pendingApproval = Boolean\(props\.approval && !props\.approval\.resolved\)/, "tool card should detect unresolved permission prompts before rendering output");
  assert.match(toolCallCardSourceTs, /\{pendingApproval \? null : props\.terminal \? \(/, "tool card should hide terminal and tool content until permission is granted");
  assert.doesNotMatch(toolCallCardSourceTs, /Accordion|ToolApprovalDisclosures|ToolApprovalMeta|locationsHtml|requestDetails/, "tool card approval prompts should omit provider metadata, locations, and details accordions");
  assert.match(toolCallCardSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "tool card should render permission decisions with shadcn button components");
  assert.match(toolCallCardSourceTs, /ButtonGroup className="approval-option-list"/, "tool card should keep approval options grouped inside the tool call");
  assert.match(toolCallCardSourceTs, /ShieldCheck[\s\S]*CircleX|CircleX[\s\S]*ShieldCheck/, "tool card approval actions should use lucide React icons instead of injected SVG strings");
  assert.match(approvalCardSourceTs, /variant=\{variant\}[\s\S]*size="sm"[\s\S]*className="approval-decision-copy"/, "approval card decision buttons should keep visible short labels next to icons");
  assert.match(toolCallCardSourceTs, /variant=\{variant\}[\s\S]*size="sm"[\s\S]*className="approval-decision-copy"/, "tool approval buttons should keep visible short labels next to icons");
  assert.doesNotMatch(`${approvalCardSourceTs}\n${toolCallCardSourceTs}`, /data-approval-icon-only|approval-decision-copy sr-only/, "important approval decisions should not collapse into icon-only controls");
  assert.doesNotMatch(toolCallCardSourceTs, /from "@\/webview\/components\/ui\/alert"/, "tool card should not import alert chrome for compact permission prompts");
  assert.match(sourceWithSharedChunks(toolCallGroupCardSource), /createRoot/, "lazy grouped tool island should mount with React");
  assert.match(toolCallGroupCardSourceTs, /import \{[\s\S]*Collapsible,[\s\S]*CollapsibleContent,[\s\S]*CollapsibleTrigger[\s\S]*\} from "@\/webview\/components\/ui\/collapsible"/, "grouped tool island should render collapsed summaries with shadcn collapsible components");
  assert.match(toolCallGroupCardSourceTs, /<ToolCallCard[\s\S]*props=\{item\}[\s\S]*callbacks=\{callbacks\}/, "expanded grouped tools should reuse the single tool card renderer");
  assert.match(toolCallGroupCardSourceTs, /bg-card/, "collapsed grouped tool summaries should use the same light card baseline as tool calls");
  assert.match(toolCallGroupCardSourceTs, /hover:bg-muted\/60/, "collapsed grouped tool summaries should keep hover feedback aligned with tool calls");
  assert.doesNotMatch(toolCallGroupCardSourceTs, /bg-muted\/10|hover:bg-muted\/20/, "grouped tool summaries should not use the old gray strip treatment");
  assert.doesNotMatch(toolCallGroupCardSourceTs, /from "@\/webview\/components\/ui\/badge"|tool-group-preview|tool-group-meta/, "collapsed grouped tool summaries should avoid badges and per-tool previews");
  assert.match(sourceWithSharedChunks(toolCallGroupCardSource), /data-tool-call-group/, "lazy grouped tool island should preserve grouped tool DOM affordances");
  assert.match(webviewSource, /summarizeToolCallGroup\(items\)[\s\S]*title: summary\.title[\s\S]*detail: summary\.detail/, "grouped tool summaries should describe completed activity instead of showing only a tool-call count");
  assert.match(webviewSource, /function renderTimelineGroupBodyItems\(nodes: RenderNode\[\]\): TimelineGroupCardProps\["bodyItems"\][\s\S]*collectGroupableToolRun\(nodes, index\)[\s\S]*toolCallGroupBodyItem\(toolRun\)/, "timeline groups should coalesce adjacent finished tool calls into one body item");
  assert.match(webviewSource, /function shouldGroupCompletedToolCalls\(\): boolean \{[\s\S]*timelineFilter === "all" && timelineSearchTokens\(timelineQuery\)\.length === 0/s, "tool grouping should stay disabled during transcript filtering and search");
  assert.match(webviewSource, /function isGroupableToolNode\(node: RenderNode \| undefined\): node is ToolNodeView \{[\s\S]*node\.permission\?\.status === "pending"[\s\S]*return status !== "pending" && status !== "in_progress"/, "tool grouping should leave pending and running tool calls visible as individual cards");
  assert.match(webviewSource, /function timelineGroups\(nodes: RenderNode\[\]\): TimelineGroup\[\] \{[\s\S]*buildTimelineConversationGroups\(nodes\)\.map/, "timeline grouping should use the shared conversation grouping model");
  assert.match(timelineModelSourceTs, /export function buildTimelineConversationGroups\(nodes: RenderNode\[\]\): TimelineConversationGroup\[\] \{[\s\S]*const segmentCounts = new Map<string, number>\(\)[\s\S]*const groupByUserMessages = nodes\.some\(isTimelineUserMessageNode\)[\s\S]*currentConversationBaseKey = `turn:\$\{currentConversationTurnId\}`[\s\S]*timelineConversationGroupScope/, "timeline grouping should start visible turns from user prompt boundaries");
  assert.match(timelineModelSourceTs, /function timelineConversationGroupScope\([\s\S]*groupByUserMessages[\s\S]*currentConversationBaseKey[\s\S]*baseKey: "session"[\s\S]*node\.turnId \? \{ baseKey: `turn:\$\{node\.turnId\}`/, "timeline grouping should fall back to raw turn scopes when no user prompt is visible");
  assert.doesNotMatch(timelineModelSourceTs.match(/export function buildTimelineConversationGroups\(nodes: RenderNode\[\]\): TimelineConversationGroup\[\] \{[\s\S]*?\n}\n\nfunction timelineConversationGroupScope/)?.[0] || "", /byKey\.get|byKey\.set/, "timeline grouping should not bucket repeated turn ids into their first visual group");
  assert.match(
    webviewSource,
    /function renderTimeline\(nodes: RenderNode\[\]\): TimelineScrollerItemView\[\][\s\S]*renderTimelineGroup\(group, index, groups\.length, groups\)[\s\S]*function renderTimelineGroup\(group: TimelineGroup, index: number, total: number, groups: TimelineGroup\[\]\): TimelineScrollerItemView \{[\s\S]*shouldOpenTimelineGroup\(group, index, total, groups\)/,
    "full timeline renders should pass all groups into timeline open-state selection"
  );
  assert.match(
    webviewSource,
    /function refreshTimelineGroupsForPatchedNodes\(nodeIds: string\[\]\): PatchedTimelineHydrationTargets \{[\s\S]*const groups = timelineGroups\(visibleTimelineNodes\(\)\)[\s\S]*renderTimelineGroup\(group, index, groups\.length, groups\)/,
    "patched timeline group refreshes should preserve the full open-state context"
  );
  assert.match(
    webviewSource,
    /function shouldOpenTimelineGroup\(group: TimelineGroup, index: number, total: number, groups: TimelineGroup\[\]\): boolean \{[\s\S]*index === total - 1 \|\| isLatestTurnGroup\(group, index, groups\)[\s\S]*function isLatestTurnGroup\(group: TimelineGroup, index: number, groups: TimelineGroup\[\]\): boolean \{[\s\S]*!group\.turnId[\s\S]*for \(let nextIndex = index \+ 1; nextIndex < groups\.length; nextIndex \+= 1\)[\s\S]*groups\[nextIndex\]\?\.turnId[\s\S]*return true/,
    "latest completed turn groups should stay open even when later session task updates trail them"
  );
  assert.match(webviewSource, /function approvalNode\(node:[\s\S]*return toolNode\(approvalAsToolNode\(node\)\)/, "orphan approval nodes should render through the tool call card");
  assert.match(webviewSource, /function chatNodes\(nodes: RenderNode\[\]\): RenderNode\[\]\s*\{[\s\S]*return timelineDisplayNodes\(nodes\);[\s\S]*\}/, "chat transcript nodes should use the shared timeline display model");
  assert.match(timelineModelSourceTs, /function scopedToolCallKey\([\s\S]*node\.taskId[\s\S]*node\.lane[\s\S]*node\.turnId[\s\S]*node\.acpSessionId[\s\S]*node\.source[\s\S]*node\.toolCallId/, "matched approval nodes should scope reused tool ids before attaching to existing tool nodes");
  assert.match(timelineModelSourceTs, /function timelineDisplayNodes\(nodes: RenderNode\[\]\): RenderNode\[\] \{[\s\S]*flatMap\(scopedToolCallKeysForTool\)[\s\S]*approvalForTool\(approvalsByTool, node\)[\s\S]*toolWithApproval\(node, approval\)[\s\S]*scopedToolCallKeysForApproval\(node\)[\s\S]*visibleToolKeys\.has\(key\)/, "matched approval nodes should attach through scoped tool id aliases");
  assert.match(timelineModelSourceTs, /function toolWithApproval\([\s\S]*timelineOrder: earliestTimelineOrder\(tool\.timelineOrder, approval\.timelineOrder\)[\s\S]*createdAt: earliestTimestamp\(tool\.createdAt, approval\.createdAt\)[\s\S]*permission: permissionFromApproval\(approval\)/, "folded approval tool rows should keep the earliest lifecycle ordering metadata");
  assert.match(composerCardSource, /createRoot/, "lazy composer island should mount with React");
  assert.match(composerCardSource, /data-composer-card/, "lazy composer island should preserve composer DOM affordances");
  assert.doesNotMatch(composerCardSourceTs, /from "@\/webview\/components\/ui\/alert"/, "composer island should not import alert chrome for running state");
  assert.match(composerCardSourceTs, /function ComposerStatus\([\s\S]*className="sr-only"[\s\S]*role="status"/, "composer status should remain available to assistive technology without visible chrome");
  assert.match(composerCardSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "composer island should render attachments with shadcn badges");
  assert.match(composerCardSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "composer island should render composer commands with shadcn buttons");
  assert.match(composerCardSourceTs, /import \{ ButtonGroup \} from "@\/webview\/components\/ui\/button-group"/, "composer island should group prompt controls with shadcn button groups");
  assert.match(composerCardSourceTs, /import \{ Card, CardContent, CardFooter \} from "@\/webview\/components\/ui\/card"/, "composer island should render the prompt surface with shadcn cards");
  assert.match(composerCardSourceTs, /import \{ Collapsible, CollapsibleContent, CollapsibleTrigger \} from "@\/webview\/components\/ui\/collapsible"/, "composer island should render agent controls with shadcn collapsible components");
  assert.doesNotMatch(composerCardSourceTs, /<details|<summary/, "composer island should not render native details for agent controls");
  assert.match(composerCardSourceTs, /function ComposerAttachmentShelf\([\s\S]*<Badge[\s\S]*className="attachment-chip"[\s\S]*<Button[\s\S]*data-action="removeAttachment"[\s\S]*data-attachment-id=\{attachment\.id\}/, "composer attachments should render remove actions through shadcn badge and button primitives");
  assert.match(composerCardSourceTs, /data-composer-icon-only="true"/, "composer icon commands should expose a scoped shadcn layout hook");
  assert.doesNotMatch(composerCardSourceTs, /preset\.iconHtml|data-action="insertPromptPreset"/, "composer presets should not occupy the visible prompt surface");
  assert.match(composerCardSourceTs, /<span[\s\S]*data-icon="inline-start"[\s\S]*dangerouslySetInnerHTML=\{\{ __html: props\.settingsIconHtml \}\}/, "composer floating controls trigger should use shadcn data-icon hooks");
  assert.doesNotMatch(composerCardSourceTs, /icon-button icon-only/, "composer icon commands should not use retired icon-button styling");
  assert.doesNotMatch(composerCardSourceTs, /className="icon"[\s\S]{0,120}data-icon="inline-start"/, "composer button icons should not carry retired manual icon sizing classes");
  assert.doesNotMatch(composerCardSourceTs, /className="icon"[\s\S]{0,120}dangerouslySetInnerHTML=\{\{ __html: props\.settingsIconHtml \}\}/, "composer floating controls trigger should not carry retired manual icon sizing classes");
  assert.match(diffCardSource, /createRoot/, "lazy diff card island should mount with React");
  assert.match(diffCardSourceTs, /import \{ flushSync \} from "react-dom"/, "lazy diff card island should commit diff rows synchronously");
  assert.match(diffCardSourceTs, /flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<DiffCard/, "lazy diff card island should avoid blank diff frames");
  assert.match(diffCardSource, /data-diff-card/, "lazy diff card island should preserve diff-card DOM affordances");
  assert.match(diffCardSourceTs, /import \{[\s\S]*Accordion,[\s\S]*AccordionContent,[\s\S]*AccordionItem,[\s\S]*AccordionTrigger[\s\S]*\} from "@\/webview\/components\/ui\/accordion"/, "diff card island should render file diffs with shadcn accordion components");
  assert.match(diffCardSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "diff card island should render file diff stats with shadcn badges");
  assert.match(diffCardSourceTs, /import \{ Card, CardContent \} from "@\/webview\/components\/ui\/card"/, "diff card island should render the diff surface with shadcn cards");
  assert.match(diffEnhancerSource, /FileTree/, "lazy diff enhancer should mount Trees.software file trees");
  assert.match(diffEnhancerSourceTs, /import \{ FileDiff, parsePatchFiles/, "lazy diff enhancer should import Diffs.com patch parsing only in the lazy source");
  assert.match(diffEnhancerSourceTs, /function fileDiffFromPatch[\s\S]*parsePatchFiles/, "lazy diff enhancer should render patch-only Diffs.com previews");
  assert.match(diffReviewSourceTs, /export function renderDiffReviewDrawer\(result:\s*unknown,\s*renderHelpers:/, "diff review drawer should render from a lazy source chunk");
  assert.match(diffReviewSourceTs, /function splitPatchFiles\(patch:\s*string\)/, "diff review drawer should parse patch-only responses lazily");
  assert.match(diffReviewSourceTs, /host\.inlineActions\(\{[\s\S]*className: "diff-review-header-actions"[\s\S]*action: "closeDrawer"/, "diff review close affordance should render through shadcn inline actions");
  assert.match(diffReviewSourceTs, /host\.inlineActions\(\{[\s\S]*className: "diff-review-suggestion-actions"[\s\S]*action: "insertDiffSuggestion"[\s\S]*data: \{ command: suggestion\.command \}/, "diff review suggestion affordances should render through shadcn inline actions");
  assert.doesNotMatch(diffReviewSourceTs, /iconButton/, "diff review drawer should not depend on retired raw icon buttons");
  assert.match(emptyStateCardSource, /createRoot/, "lazy empty state island should mount with React");
  assert.match(emptyStateCardSource, /data-empty-state-card/, "lazy empty state island should preserve empty-state DOM affordances");
  assert.match(emptyStateCardSourceTs, /import \{[\s\S]*Empty,[\s\S]*EmptyContent,[\s\S]*EmptyDescription,[\s\S]*EmptyHeader,[\s\S]*EmptyMedia,[\s\S]*EmptyTitle[\s\S]*\} from "@\/webview\/components\/ui\/empty"/, "empty state island should compose shadcn Empty parts");
  assert.match(emptyStateCardSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "empty state island should render the role label with shadcn badges");
  assert.match(emptyStateCardSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "empty state island should render actions with shadcn buttons");
  assert.match(emptyStateCardSourceTs, /<EmptyMedia className="empty-state-media" variant="icon">/, "empty state island should use a scoped shadcn EmptyMedia hook instead of legacy card chrome");
  assert.match(emptyStateCardSourceTs, /<Badge className="empty-state-role" variant="outline">/, "empty state role labels should render as shadcn badges");
  assert.match(emptyStateCardSourceTs, /<span[\s\S]*data-icon="inline-start"[\s\S]*dangerouslySetInnerHTML=\{\{ __html: action\.iconHtml \}\}/, "empty state action icons should use shadcn data-icon hooks");
  assert.doesNotMatch(emptyStateCardSourceTs, /card-chrome/, "empty state island should not depend on retired card chrome");
  assert.doesNotMatch(emptyStateCardSourceTs, /className="tool-icon"[\s\S]{0,120}dangerouslySetInnerHTML=\{\{ __html: action\.iconHtml \}\}/, "empty state action icons should not carry retired manual icon sizing classes");
  assert.match(eventCardSource, /createRoot/, "lazy event card island should mount with React");
  assert.match(eventCardSourceTs, /import \{ flushSync \} from "react-dom"/, "lazy event card island should commit completion and checkpoint rows synchronously");
  assert.match(eventCardSourceTs, /flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<EventCard/, "lazy event card island should avoid blank final event frames");
  assert.match(eventCardSource, /data-event-card/, "lazy event card island should preserve event-card DOM affordances");
  assert.match(eventCardSourceTs, /import \{ Alert, AlertDescription, AlertTitle \} from "@\/webview\/components\/ui\/alert"/, "event card island should render callouts with shadcn alert components");
  assert.match(eventCardSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "event card island should render event facts and status with shadcn badges");
  assert.match(eventCardSourceTs, /import \{ Card, CardContent, CardHeader \} from "@\/webview\/components\/ui\/card"/, "event card island should render the event surface with shadcn cards");
  assert.match(eventCardSourceTs, /import \{ InlineActions, type InlineActionTone \} from "\.\/InlineActions"/, "event card island should render actions through the shared shadcn inline action rail");
  assert.match(eventCardSourceTs, /import \{ RawDetails, type RawDetailsView \} from "\.\/RawDetails"/, "event card island should render raw details through the shadcn raw-details component");
  assert.match(eventCardSourceTs, /<InlineActions[\s\S]*className: "event-action-row"[\s\S]*tone: eventActionTone\(action\.tone\)[\s\S]*iconHtml: action\.iconHtml[\s\S]*data: action\.target \? \{ target: action\.target \} : undefined/, "event actions should preserve action targets while delegating button markup to InlineActions");
  assert.doesNotMatch(eventCardSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "event card island should not render bespoke event buttons");
  assert.doesNotMatch(eventCardSourceTs, /event-action-primary|event-action-danger|className=\{cn\([\s\S]*"event-action"/, "event card island should not emit retired per-button event-action classes");
  assert.match(headerBarSource, /createRoot/, "lazy header toolbar island should mount with React");
  assert.match(headerBarSource, /toolbar-capability-grid/, "lazy header toolbar island should preserve capability affordances");
  assert.match(headerBarSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "header toolbar island should render status chips with shadcn badges");
  assert.match(headerBarSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "header toolbar island should render commands with shadcn buttons");
  assert.match(headerBarSourceTs, /import \{ ButtonGroup \} from "@\/webview\/components\/ui\/button-group"/, "header toolbar island should group commands with shadcn button groups");
  assert.match(headerBarSourceTs, /import \{ Card, CardContent \} from "@\/webview\/components\/ui\/card"/, "header toolbar island should render floating panels with shadcn cards");
  assert.match(headerBarSourceTs, /import \{ Collapsible, CollapsibleContent, CollapsibleTrigger \} from "@\/webview\/components\/ui\/collapsible"/, "header toolbar island should render floating panels with shadcn collapsible components");
  assert.match(headerBarSourceTs, /LaneMapToolbarButton/, "header toolbar should own the lane map trigger");
  assert.match(headerBarSourceTs, /data-lane-map-trigger="true"/, "lane map should be reachable from the top toolbar");
  assert.match(headerBarSourceTs, /React\.lazy\(async \(\) =>[\s\S]*import\("\.\/LaneMapDrawer\.js"\)/, "lane map drawer should load only after the toolbar trigger opens it");
  assert.doesNotMatch(headerBarSourceTs, /<details|<summary/, "header toolbar island should not render native details for floating panels");
  assert.match(headerBarSourceTs, /data-header-icon-only="true"/, "header icon commands should expose a scoped shadcn layout hook");
  assert.match(headerBarSourceTs, /<span[\s\S]*data-icon="inline-start"[\s\S]*dangerouslySetInnerHTML=\{\{ __html: iconHtml \}\}/, "primary header action icons should use shadcn data-icon hooks");
  assert.match(headerBarSourceTs, /<span[\s\S]*data-icon="inline-start"[\s\S]*dangerouslySetInnerHTML=\{\{ __html: action\.iconHtml \}\}/, "secondary header action icons should use shadcn data-icon hooks");
  assert.doesNotMatch(headerBarSourceTs, /icon-button icon-only/, "header icon commands should not use retired icon-button styling");
  assert.doesNotMatch(headerBarSourceTs, /className="icon"[\s\S]{0,120}data-icon="inline-start"/, "header button icons should not carry retired manual icon sizing classes");
  assert.doesNotMatch(headerBarSourceTs, /className="icon"[\s\S]{0,120}dangerouslySetInnerHTML=\{\{ __html: iconHtml \}\}/, "header floating trigger icons should not carry retired manual icon sizing classes");
  assert.match(messageCardSource, /createRoot/, "lazy message card island should mount with React");
  assert.match(messageCardSourceTs, /import \{ flushSync \} from "react-dom"/, "lazy message card island should commit streaming updates synchronously");
  assert.match(messageCardSourceTs, /flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<MessageCard/, "lazy message card island should avoid blank frames while streaming");
  assert.match(messageCardSource, /data-message-card/, "lazy message card island should preserve message-card DOM affordances");
  assert.match(messageCardSource, /data-slot["']?\s*[:,=]\s*["']message["']|data-slot="message"/, "lazy message card island should use shadcn message components");
  assert.match(messageCardSourceTs, /import \{[\s\S]*Message,[\s\S]*MessageContent,[\s\S]*MessageGroup[\s\S]*\} from "@\/webview\/components\/ui\/message"/, "lazy message card island should compose the minimal shadcn Message parts");
  assert.match(messageCardSourceTs, /import \{ StreamdownMarkdown \} from "\.\/StreamdownMarkdown"/, "streaming messages should render live markdown through the shared Streamdown island");
  assert.match(messageCardSourceTs, /props\.contentMode === "stream-text"[\s\S]*<StreamdownMarkdown[\s\S]*streaming=\{props\.streaming\}[\s\S]*text=\{props\.contentText \|\| ""\}/, "streaming messages should keep Streamdown mounted behind a stable wrapper");
  assert.doesNotMatch(streamdownMarkdownSourceTs, /@streamdown\/code|plugins=\{streamdownPlugins\}/, "streaming markdown should keep the optional Shiki code plugin out of the live hot path");
  assert.match(streamdownMarkdownSourceTs, /const streamdownControls: ControlsConfig = false/, "streaming markdown should disable controls during live rendering to avoid button churn");
  assert.match(streamdownMarkdownSourceTs, /React\.memo\(function StreamdownMarkdown[\s\S]*requestAnimationFrame[\s\S]*__trailQueueStreamdownText[\s\S]*<Streamdown[\s\S]*parseIncompleteMarkdown=\{streaming\}/, "streaming markdown should coalesce updates per frame while using Streamdown incomplete-block parsing");
  assert.match(streamdownMarkdownSourceTs, /data-streamdown-markdown=""[\s\S]*data-streaming-markdown=""/, "streaming markdown should preserve the patchable DOM affordance");
  assert.match(tailwindSource, /@source "\.\.\/\.\.\/node_modules\/streamdown\/dist\/\*\.js";/, "webview styles should scan Streamdown utilities");
  assert.doesNotMatch(tailwindSource, /@streamdown\/code/, "webview styles should not scan the optional code plugin when it is not mounted");
  assert.match(fs.readFileSync(path.join(root, "package.json"), "utf8"), /"streamdown"/, "webview package should keep Streamdown for live markdown");
  assert.doesNotMatch(fs.readFileSync(path.join(root, "package.json"), "utf8"), /"@streamdown\/code"/, "webview package should not keep the optional code plugin in the streaming hot path");
  assert.match(webviewSource, /function streamingTextForNode\([\s\S]*node\.kind === "message" && !node\.streaming[\s\S]*return textOnlyContent\(node\.content\)/, "webview should route live text-only messages through the streaming text renderer");
  assert.doesNotMatch(messageCardSourceTs, /from "@\/webview\/components\/ui\/marker"|from "@\/webview\/components\/ui\/spinner"|from "@\/webview\/components\/ui\/badge"/, "plain assistant messages should not import visible role or streaming badge primitives");
  assert.match(payloadDisclosureSource, /createRoot/, "lazy payload disclosure island should mount with React");
  assert.match(payloadDisclosureSourceTs, /import \{ flushSync \} from "react-dom"/, "payload disclosure helpers should commit synchronously to avoid blank helper frames");
  assert.match(payloadDisclosureSourceTs, /import \{[\s\S]*Accordion,[\s\S]*AccordionContent,[\s\S]*AccordionItem,[\s\S]*AccordionTrigger[\s\S]*\} from "@\/webview\/components\/ui\/accordion"/, "payload disclosure island should render helper disclosures with shadcn accordion components");
  assert.match(payloadDisclosureSourceTs, /data-payload-disclosure-root/, "payload disclosure island should preserve helper placeholder roots");
  assert.match(payloadDisclosureSourceTs, /ids\?: ReadonlySet<string> \| undefined[\s\S]*options\.ids && !options\.ids\.has\(id\)[\s\S]*activeIds\.add\(id\)[\s\S]*flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<PayloadDisclosure/, "payload disclosure hydration should be filterable and preserve untouched mounted roots");
  assert.doesNotMatch(payloadDisclosureSourceTs, /<details|<summary/, "payload disclosure island should not render native details markup");
  assert.match(planCardSource, /createRoot/, "lazy plan card island should mount with React");
  assert.match(planCardSourceTs, /import \{ flushSync \} from "react-dom"/, "lazy plan card island should commit plan updates synchronously");
  assert.match(planCardSourceTs, /flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<PlanCard/, "lazy plan card island should avoid blank plan frames");
  assert.match(planCardSource, /data-plan-card/, "lazy plan card island should preserve plan-card DOM affordances");
  assert.match(planCardSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "plan card island should render statuses with shadcn badges");
  assert.match(planCardSourceTs, /import \{[\s\S]*Card,[\s\S]*CardAction,[\s\S]*CardContent,[\s\S]*CardDescription,[\s\S]*CardHeader,[\s\S]*CardTitle[\s\S]*\} from "@\/webview\/components\/ui\/card"/, "plan card island should render the plan surface with full shadcn card composition");
  assert.match(planCardSourceTs, /import \{ Checkbox \} from "@\/webview\/components\/ui\/checkbox"/, "plan card island should render status markers with shadcn checkboxes");
  assert.match(planCardSourceTs, /import \{ Separator \} from "@\/webview\/components\/ui\/separator"/, "plan card island should separate header and steps with shadcn separators");
  assert.match(recoveryBannerSource, /createRoot/, "lazy recovery banner island should mount with React");
  assert.match(recoveryBannerSource, /data-recovery-banner/, "lazy recovery banner island should preserve recovery DOM affordances");
  assert.match(recoveryBannerSourceTs, /import \{ Alert, AlertDescription, AlertTitle \} from "@\/webview\/components\/ui\/alert"/, "recovery banner island should render callouts with shadcn alert components");
  assert.match(recoveryBannerSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "recovery banner island should render statuses with shadcn badges");
  assert.match(recoveryBannerSourceTs, /import \{ InlineActions, type InlineActionTone \} from "\.\/InlineActions"/, "recovery banner island should render actions through the shared shadcn inline action rail");
  assert.match(recoveryBannerSourceTs, /import \{ Separator \} from "@\/webview\/components\/ui\/separator"/, "recovery banner island should separate overlap evidence with shadcn separators");
  assert.match(recoveryBannerSourceTs, /<Badge className="recovery-banner-role" variant=\{destructive \? "destructive" : "outline"\}>/, "recovery banner eyebrow should use a scoped shadcn badge hook instead of generic role styling");
  assert.match(recoveryBannerSourceTs, /<div className="recovery-banner-badges">/, "recovery banner status chips should use a scoped badge row instead of retired card chrome");
  assert.match(recoveryBannerSourceTs, /<InlineActions[\s\S]*className: "recovery-actions"[\s\S]*data: \{ "recovery-action-tone": action\.tone \}/, "recovery actions should preserve recovery tone metadata through InlineActions");
  assert.doesNotMatch(recoveryBannerSourceTs, /card-chrome|className="role"|import \{ Button \} from "@\/webview\/components\/ui\/button"|import \{ ButtonGroup \} from "@\/webview\/components\/ui\/button-group"/, "recovery banner should not keep retired card chrome, role styling, or bespoke action buttons");
  assert.match(resultDrawerSource, /createRoot/, "lazy result drawer island should mount with React");
  assert.match(resultDrawerSource, /json-drawer/, "lazy result drawer island should preserve drawer DOM affordances");
  assert.match(resultDrawerSourceTs, /import \{[\s\S]*Accordion,[\s\S]*AccordionContent,[\s\S]*AccordionItem,[\s\S]*AccordionTrigger[\s\S]*\} from "@\/webview\/components\/ui\/accordion"/, "result drawer island should render helper disclosure widgets with shadcn accordion components");
  assert.match(resultDrawerSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "result drawer island should render status labels with shadcn badges");
  assert.match(resultDrawerSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "result drawer island should render the close affordance with shadcn buttons");
  assert.match(resultDrawerSourceTs, /import \{[\s\S]*Drawer,[\s\S]*DrawerContent,[\s\S]*DrawerDescription,[\s\S]*DrawerHeader,[\s\S]*DrawerTitle[\s\S]*\} from "@\/webview\/components\/ui\/drawer"/, "result drawer island should render inspector drawers with shadcn drawer components");
  assert.match(resultDrawerSourceTs, /<Button[\s\S]*data-action="closeDrawer"[\s\S]*size="icon-sm"[\s\S]*variant="ghost"/, "result drawer close affordance should use shadcn icon button sizing");
  assert.doesNotMatch(resultDrawerSourceTs, /className="icon-button"/, "result drawer close affordance should not use retired icon-button styling");
  assert.match(reviewDrawerSource, /createRoot/, "lazy review drawer island should mount with React");
  assert.match(reviewDrawerSource, /review-command-center/, "lazy review drawer island should preserve review readiness affordances");
  assert.match(reviewDrawerSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "review drawer island should render status and gates with shadcn badges");
  assert.match(reviewDrawerSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "review drawer island should render actions with shadcn buttons");
  assert.match(reviewDrawerSourceTs, /import \{ ButtonGroup \} from "@\/webview\/components\/ui\/button-group"/, "review drawer island should group review commands with shadcn button groups");
  assert.match(reviewDrawerSourceTs, /import \{[\s\S]*Card,[\s\S]*CardContent,[\s\S]*CardHeader,[\s\S]*CardTitle[\s\S]*\} from "@\/webview\/components\/ui\/card"/, "review drawer island should render review surfaces with shadcn cards");
  assert.match(reviewDrawerSourceTs, /<span[\s\S]*data-icon="inline-start"[\s\S]*dangerouslySetInnerHTML=\{\{ __html: iconHtml \}\}/, "review action icons should use shadcn data-icon hooks");
  assert.doesNotMatch(reviewDrawerSourceTs, /className="icon"[\s\S]{0,120}data-icon="inline-start"/, "review action icons should not carry retired manual icon sizing classes");
  assert.match(terminalCardSource, /createRoot/, "lazy terminal card island should mount with React");
  assert.match(terminalCardSourceTs, /import \{ flushSync \} from "react-dom"/, "lazy terminal card island should commit terminal updates synchronously");
  assert.match(terminalCardSourceTs, /flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<TerminalCard/, "lazy terminal card island should avoid blank terminal frames");
  assert.match(terminalCardSource, /data-terminal-card/, "lazy terminal card island should preserve terminal-card DOM affordances");
  assert.match(terminalCardSourceTs, /import \{[\s\S]*Accordion,[\s\S]*AccordionContent,[\s\S]*AccordionItem,[\s\S]*AccordionTrigger[\s\S]*\} from "@\/webview\/components\/ui\/accordion"/, "terminal card island should render output sections with shadcn accordion components");
  assert.match(terminalCardSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "terminal card island should render terminal status with shadcn badges");
  assert.match(terminalCardSourceTs, /import \{ InlineActions \} from "\.\/InlineActions"/, "terminal card island should render open actions through the shared shadcn inline action rail");
  assert.match(terminalCardSourceTs, /import \{ Card, CardContent, CardHeader \} from "@\/webview\/components\/ui\/card"/, "terminal card island should render the terminal surface with shadcn cards");
  assert.match(
    terminalCardSourceTs,
    /<InlineActions[\s\S]*className: "terminal-transcript-actions"[\s\S]*action: "openTerminal"[\s\S]*iconHtml: props\.openIconHtml[\s\S]*iconOnly: true[\s\S]*data: \{ "node-id": props\.nodeId \}/,
    "terminal open affordance should delegate button markup to InlineActions while preserving node metadata"
  );
  assert.doesNotMatch(terminalCardSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"|className="icon-button icon-only"/, "terminal open affordance should not keep bespoke button markup");
  assert.match(thoughtCardSource, /createRoot/, "lazy thought card island should mount with React");
  assert.match(thoughtCardSource, /data-thought-card/, "lazy thought card island should preserve thought-card DOM affordances");
  assert.match(thoughtCardSourceTs, /import \{[\s\S]*Accordion,[\s\S]*AccordionContent,[\s\S]*AccordionItem,[\s\S]*AccordionTrigger[\s\S]*\} from "@\/webview\/components\/ui\/accordion"/, "thought card island should render reasoning details with shadcn accordion components");
  assert.match(thoughtCardSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "thought card island should render reasoning status with shadcn badges");
  assert.match(thoughtCardSourceTs, /import \{ Card, CardContent \} from "@\/webview\/components\/ui\/card"/, "thought card island should render the reasoning surface with shadcn cards");
  assert.match(thoughtCardSourceTs, /import \{ flushSync \} from "react-dom"/, "lazy thought card island should commit streaming updates synchronously");
  assert.match(thoughtCardSourceTs, /flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<ThoughtCard/, "lazy thought card island should avoid blank reasoning frames while streaming");
  assert.match(thoughtCardSourceTs, /import \{ StreamdownMarkdown \} from "\.\/StreamdownMarkdown"/, "streaming thought cards should share the live Streamdown markdown island");
  assert.match(thoughtCardSourceTs, /props\.contentMode === "stream-text"[\s\S]*<StreamdownMarkdown[\s\S]*className="event-content"[\s\S]*text=\{props\.contentText \|\| ""\}/, "streaming thought cards should render live markdown without replacing the thought shell");
  assert.match(webviewSource, /function thoughtNode\(node:[\s\S]*const streamText = streamingTextForNode\(node\)[\s\S]*thoughtCardProps\.set/, "webview should route live text-only thoughts through the streaming text renderer");
  assert.match(timelineGroupSource, /createRoot/, "lazy timeline group island should mount with React");
  assert.match(timelineGroupSource, /timeline-group-summary/, "lazy timeline group island should preserve group summary affordances");
  assert.match(timelineGroupSourceTs, /import \{ flushSync \} from "react-dom"/, "timeline group shell should commit synchronously before nested message and tool islands hydrate");
  assert.match(timelineGroupSourceTs, /flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<TimelineGroupCard/, "timeline group roots should render inside flushSync so helper placeholders exist for nested hydration");
  assert.match(timelineGroupSourceTs, /import \{ Accordion as AccordionPrimitive \} from "@base-ui\/react\/accordion"/, "timeline group island should own the summary header layout so copy controls are not nested in a trigger button");
  assert.match(timelineGroupSourceTs, /import \{[\s\S]*Accordion,[\s\S]*AccordionContent,[\s\S]*AccordionItem[\s\S]*\} from "@\/webview\/components\/ui\/accordion"/, "timeline group island should render transcript groups with shadcn accordion components");
  assert.match(timelineGroupSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "timeline group island should render status metadata with shadcn badges");
  assert.match(timelineGroupSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "timeline group island should render hidden-id copy affordances with shadcn buttons");
  assert.match(timelineGroupSourceTs, /data-action="copyTimelineGroupId"[\s\S]*data-target=\{props\.laneId\}/, "timeline group id should be copyable without rendering the raw id as text");
  assert.doesNotMatch(timelineGroupSourceTs, /props\.laneLabel/, "timeline group summaries should not render raw lane ids directly");
  assert.match(timelineGroupSourceTs, /props\.bodyItems\.map\(\(item\) =>[\s\S]*key=\{item\.id\}[\s\S]*data-timeline-group-body-item/, "timeline group bodies should preserve existing transcript node DOM while appending streamed rows");
  assert.doesNotMatch(timelineGroupSourceTs, /props\.bodyHtml|bodyKeyRef/, "timeline group bodies should not replace the whole turn body when one node changes");
  assert.doesNotMatch(timelineGroupSourceTs, /<details|<summary/, "timeline group island should not render native details markup");
  assert.match(timelineNavigationSource, /createRoot/, "lazy timeline navigation island should mount with React");
  assert.match(timelineNavigationSource, /timeline-search-input/, "lazy timeline navigation should preserve search affordances");
  assert.match(timelineNavigationSourceTs, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/, "timeline navigation island should render counts and chips with shadcn badges");
  assert.match(timelineNavigationSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "timeline navigation island should render filters and clear actions with shadcn buttons");
  assert.match(timelineNavigationSourceTs, /import \{ ButtonGroup \} from "@\/webview\/components\/ui\/button-group"/, "timeline navigation island should group transcript filters with shadcn button groups");
  assert.match(timelineNavigationSourceTs, /import \{ Card, CardContent \} from "@\/webview\/components\/ui\/card"/, "timeline navigation island should render the toolbar filter panel with shadcn cards");
  assert.match(timelineNavigationSourceTs, /import \{ Collapsible, CollapsibleContent, CollapsibleTrigger \} from "@\/webview\/components\/ui\/collapsible"/, "timeline navigation island should open transcript filters from a toolbar collapsible");
  assert.match(timelineNavigationSourceTs, /<span[\s\S]*data-icon="inline-start"[\s\S]*dangerouslySetInnerHTML=\{\{ __html: props\.searchIconHtml \}\}/, "timeline search icon should use shadcn data-icon hooks");
  assert.doesNotMatch(timelineNavigationSourceTs, /className="icon"[\s\S]{0,120}dangerouslySetInnerHTML=\{\{ __html: props\.searchIconHtml \}\}/, "timeline search icon should not carry retired manual icon sizing classes");
  assert.doesNotMatch(timelineNavigationSourceTs, /session-map|Lane map|ToolActivityPanel/, "timeline navigation should not render the lane map inline");
  assert.match(laneMapDrawerSourceTs, /import \{[\s\S]*Drawer,[\s\S]*DrawerContent,[\s\S]*DrawerDescription,[\s\S]*DrawerHeader,[\s\S]*DrawerTitle[\s\S]*\} from "@\/webview\/components\/ui\/drawer"/, "lane map should render inside a shadcn drawer");
  assert.match(laneMapDrawerSourceTs, /import \{ Card, CardContent \} from "@\/webview\/components\/ui\/card"/, "lane map drawer should use shadcn cards for dense summaries");
  assert.match(laneMapDrawerSourceTs, /<span[\s\S]*data-icon="inline-start"[\s\S]*dangerouslySetInnerHTML=\{\{ __html: chip\.iconHtml \}\}/, "lane map badge icons should use shadcn data-icon hooks");
  assert.doesNotMatch(laneMapDrawerSourceTs, /className="icon"[\s\S]{0,120}data-icon="inline-start"/, "lane map badge icons should not carry retired manual icon sizing classes");
  assert.match(sourceWithSharedChunks(inlineActionsSource), /createRoot/, "lazy inline actions island should mount with React");
  assert.match(inlineActionsSourceTs, /import \{ flushSync \} from "react-dom"/, "inline action helpers should commit synchronously to avoid blank helper frames");
  assert.match(inlineActionsSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "inline actions island should render commands with shadcn buttons");
  assert.match(inlineActionsSourceTs, /import \{ ButtonGroup \} from "@\/webview\/components\/ui\/button-group"/, "inline actions island should group helper action rails with shadcn button groups");
  assert.match(inlineActionsSourceTs, /import \{[\s\S]*Tooltip,[\s\S]*TooltipContent,[\s\S]*TooltipProvider,[\s\S]*TooltipTrigger[\s\S]*\} from "@\/webview\/components\/ui\/tooltip"/, "inline actions island should provide shared shadcn tooltips for described actions");
  assert.match(inlineActionsSourceTs, /data-inline-actions-root/, "inline actions island should preserve helper placeholder roots");
  assert.match(inlineActionsSourceTs, /ids\?: ReadonlySet<string> \| undefined[\s\S]*options\.ids && !options\.ids\.has\(id\)[\s\S]*activeIds\.add\(id\)[\s\S]*flushSync\(\(\) => \{[\s\S]*mounted\.root\.render\(<InlineActions/, "inline actions hydration should be filterable and preserve untouched mounted roots");
  assert.match(inlineActionsSourceTs, /onAction\?: \(\(action: InlineAction, event: React\.MouseEvent<HTMLButtonElement>\) => void\)/, "inline actions should support local React callbacks for tool-card action rails");
  assert.match(inlineActionsSourceTs, /const iconOnly = action\.iconOnly \?\? Boolean\(action\.icon \|\| action\.iconHtml\)[\s\S]*data-inline-icon-only=\{iconOnly \? "true" : undefined\}/, "inline action icon-first buttons should expose a scoped shadcn layout hook");
  assert.match(inlineActionsSourceTs, /\{action\.icon \? action\.icon : null\}/, "inline actions should accept React-owned icons for lazy islands");
  assert.match(inlineActionsSourceTs, /<span[\s\S]*data-icon="inline-start"[\s\S]*dangerouslySetInnerHTML=\{\{ __html: action\.iconHtml \}\}/, "inline action icons should use shadcn data-icon hooks");
  assert.doesNotMatch(inlineActionsSourceTs, /icon-button icon-only/, "inline action icon buttons should not use retired icon-button styling");
  assert.doesNotMatch(inlineActionsSourceTs, /className="tool-icon"/, "inline action button icons should not carry retired manual icon wrapper styling");
  assert.match(timelineScrollerSource, /createRoot/, "lazy timeline scroller island should mount with React");
  assert.match(timelineScrollerSourceTs, /import \{ flushSync \} from "react-dom"/, "timeline scroller shell should commit synchronously before timeline groups hydrate");
  assert.match(timelineScrollerSourceTs, /const root = mountedRoot\.root[\s\S]*flushSync\(\(\) => \{[\s\S]*root\.render\(<TimelineScroller/, "timeline scroller root should render inside flushSync so group roots are queryable immediately");
  assert.match(timelineScrollerSource, /message-scroller-viewport/, "lazy timeline scroller island should use shadcn message scroller components");
  assert.match(timelineScrollerSource, /timeline-scroller-content/, "lazy timeline scroller island should preserve transcript content affordances");
  assert.match(timelineScrollerSourceTs, /MessageScrollerItem/, "timeline scroller source should compose transcript rows with shadcn message scroller items");
  assert.match(timelineScrollerSourceTs, /<MessageScrollerProvider[\s\S]*autoScroll[\s\S]*defaultScrollPosition="last-anchor"[\s\S]*scrollPreviousItemPeek=\{TIMELINE_PREVIOUS_ITEM_PEEK\}/, "timeline scroller should use shadcn chat scroll behavior for streamed AI transcripts");
  assert.match(messageScrollerSourceTs, /cn-message-scroller-viewport/, "message scroller component should align with the shadcn radix registry class hooks");
  assert.match(messageScrollerSourceTs, /\[contain-intrinsic-size:auto_10rem\] \[content-visibility:auto\]/, "message scroller items should keep long transcripts light while measuring rows");
  assert.match(
    timelineScrollerSourceTs,
    /props\.items\.map\([\s\S]*<MessageScrollerItem[\s\S]*messageId=\{item\.id\}[\s\S]*scrollAnchor=\{Boolean\(item\.scrollAnchor\)\}/,
    "timeline scroller rows should expose stable message ids and turn anchors to the shadcn scroller"
  );
  assert.doesNotMatch(
    timelineScrollerSourceTs,
    /<MessageScrollerContent[\s\S]{0,240}dangerouslySetInnerHTML/,
    "timeline scroller content should not bypass shadcn row measurement with one HTML blob"
  );
  assert.match(sourceWithSharedChunks(toolCallCardSource), /createRoot/, "lazy tool card island should mount with React");
  assert.match(toolCallCardSourceTs, /import \{ Button \} from "@\/webview\/components\/ui\/button"/, "lazy tool card island should render inline permission decisions with shadcn buttons");
  assert.match(toolCallCardSourceTs, /import \{ ButtonGroup \} from "@\/webview\/components\/ui\/button-group"/, "lazy tool card island should group inline permission decisions with shadcn button groups");
  assert.match(toolCallCardSourceTs, /import \{[\s\S]*HoverCard,[\s\S]*HoverCardContent,[\s\S]*HoverCardTrigger[\s\S]*\} from "@\/webview\/components\/ui\/hover-card"/, "lazy tool card island should describe status and risk chips with shadcn hover cards");
  assert.match(inlineActionsSourceTs, /import \{[\s\S]*Tooltip,[\s\S]*TooltipContent,[\s\S]*TooltipProvider,[\s\S]*TooltipTrigger[\s\S]*\} from "@\/webview\/components\/ui\/tooltip"/, "shared inline actions should describe action buttons with shadcn tooltip components");
  assert.doesNotMatch(toolCallCardSourceTs, /from "@\/webview\/components\/ui\/breadcrumb"|from "\.\/InlineActions"|from "@\/webview\/components\/ui\/context-menu"|from "@\/webview\/components\/ui\/separator"|from "\.\/RawDetails"/, "lazy tool card island should not import details-only chrome");
  assert.doesNotMatch(toolCallCardSourceTs, /<InlineActions|toolInlineAction|toolActionDomName|ToolActionBar|ToolContextMenu/, "lazy tool card island should not render generic action rails or context menus");
  assert.match(sourceWithSharedChunks(toolCallCardSource), /data-tool-card/, "lazy tool card island should preserve tool-card DOM affordances");
  assert.match(toolCallCardSourceTs, /import \{ flushSync \} from "react-dom"/, "lazy tool card island should commit tool updates synchronously");
  assert.match(toolCallCardSourceTs, /flushSync\(\(\) => \{[\s\S]*root\.render\([\s\S]*<ToolCallCard/, "lazy tool card island should avoid blank frames while tool status streams");
  assert.match(toolCallCardSourceTs, /className="summary-icon tool-summary-icon/, "lazy tool card summary icon should use a scoped shadcn data-icon wrapper");
  for (const [componentName, source] of Object.entries({
    diffCardSourceTs,
    eventCardSourceTs,
    terminalCardSourceTs,
    thoughtCardSourceTs,
    timelineGroupSourceTs,
    timelineNavigationSourceTs,
    toolCallGroupCardSourceTs,
    toolCallCardSourceTs
  })) {
    assert.doesNotMatch(
      source,
      /className="tool-icon"/,
      `${componentName} should not emit the retired generic tool-icon wrapper`
    );
  }
  assert.match(rawDetailsSourceTs, /import \{[\s\S]*Accordion,[\s\S]*AccordionContent,[\s\S]*AccordionItem,[\s\S]*AccordionTrigger[\s\S]*\} from "@\/webview\/components\/ui\/accordion"/, "shared raw-details component should compose shadcn accordion parts");
  assert.doesNotMatch(rawDetailsSourceTs, /<details|<summary/, "shared raw-details component should not render native details markup");
  assert.match(highlightSource, /codeToTokensWithThemes/, "lazy Shiki highlighter should emit dual-theme token variants");
  assert.match(highlightSource, /--shiki-dark/, "lazy Shiki highlighter should preserve dark theme colors on tokens");
  assert.match(highlightSource, /tokenizeMaxLineLength/, "lazy Shiki highlighter should bound expensive long-line tokenization");
  assert.doesNotMatch(highlightSource, /codeToTokensBase/, "lazy Shiki highlighter should not collapse tokens to a single active theme");
  assert.doesNotMatch(mainBundleSource, /function splitPatchFiles\(patch\)/, "main bundle should not include diff review patch parsing");
  assert.doesNotMatch(mainSource, /skip-links|Chat landmarks|Alt\+1\/2\/3/, "webview should not render the retired visible landmark strip in the toolbar");
  assert.match(mainSource, /event\.altKey[\s\S]*event\.key === "1"[\s\S]*focusTranscript\(\)[\s\S]*event\.key === "2"[\s\S]*focusComposer\(\)[\s\S]*event\.key === "3"[\s\S]*focusReview\(\)/, "webview should keep Alt+1/2/3 keyboard landmark navigation without visible toolbar links");
  assert.match(headerBarSourceTs, /<span className="sr-only">Trail capabilities<\/span>/, "top toolbar should keep Trail capabilities accessible");
  assert.match(headerBarSourceTs, /toolbar-capability-grid/, "capabilities should open as a structured toolbar menu");
  assert.match(headerBarSourceTs, /group="workflow" label="Workflow"/, "capability inspector should group workflow capability state");
  assert.match(headerBarSourceTs, /group="input" label="Input"/, "capability inspector should group prompt input capability state");
  assert.match(headerBarSourceTs, /className="toolbar-capability-actions"/, "capability inspector should expose review and settings actions");

  for (const component of [
    "marker",
    "breadcrumb",
    "checkbox",
    "context-menu",
    "drawer",
    "dropdown-menu",
    "empty",
    "tooltip"
  ]) {
    assert.ok(
      fs.existsSync(path.join(root, "src", "webview", "components", "ui", `${component}.tsx`)),
      `shadcn ${component} component should be available for webview migration`
    );
  }

  const mainStyleSource = fs.readFileSync(mainStyle, "utf8");
  assert.match(
    mainStyleSource,
    /body\.vscode-light\s*{[^}]*--surface-canvas:[^}]*--surface-composer:[^}]*--border-quiet:/s,
    "webview surfaces should tune themselves for VS Code light themes"
  );
  assert.match(
    mainStyleSource,
    /body\.vscode-dark\s*{[^}]*--surface-canvas:[^}]*--surface-composer:[^}]*--border-quiet:/s,
    "webview surfaces should tune themselves for VS Code dark themes"
  );
  assert.match(
    mainStyleSource,
    /body\.vscode-high-contrast,[\s\S]*body\.vscode-high-contrast-light\s*{[^}]*--surface-canvas: var\(--surface\)[^}]*--border-quiet: var\(--border\)/s,
    "webview surfaces should keep high-contrast themes border-driven"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.transcript-message-assistant \.transcript-message-content:has\(\[role="status"\]\):{1,2}after\s*{/,
    "streaming assistant messages should keep the accessible status marker without drawing a visible live bar"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /trail-stream-sheen|will-change: opacity, transform|animation: trail-stream-sheen/,
    "streaming assistant live affordance should not animate while tokens are arriving"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.card-chrome\b|\.role\s*\{/,
    "webview stylesheet should not keep retired generic card-chrome or role selectors"
  );
  assert.doesNotMatch(
    webviewSource,
    /class="card-chrome"|class="role(?:\s|")|class="[^"]*\srole(?:\s|")/,
    "webview helper markup should not emit retired generic card-chrome or role classes"
  );
  assert.match(
    webviewSource,
    /function activityNode\([\s\S]*<span class="activity-title">/,
    "helper-rendered activity nodes should use a scoped activity title hook"
  );
  assert.match(
    webviewSource,
    /function compareTaskCard\([\s\S]*<div class="compare-task-header">[\s\S]*<span class="compare-task-label">[\s\S]*<span class="compare-task-status status status-\$\{escapeClass\(status\)\}">/,
    "helper-rendered compare task cards should use scoped header hooks while preserving status semantics"
  );
  assert.match(
    webviewSource,
    /function coordinationPanel\([\s\S]*<span class="coordination-issue-tone coordination-\$\{escapeClass\(issue\.tone\)\}">/,
    "coordination issue helpers should use scoped status labels instead of tool badge fallbacks"
  );
  assert.match(
    webviewSource,
    /function testSummary\([\s\S]*<span class="test-run-status status-\$\{escapeClass\(status\)\}">/,
    "review test and eval summaries should use scoped run status labels instead of tool badge fallbacks"
  );
  assert.doesNotMatch(
    webviewSource,
    /<span class="tool-status">/,
    "helper-rendered HTML should not emit non-shadcn tool-status spans"
  );
  assert.match(
    mainStyleSource,
    /\.activity-title\s*{[^}]*font-weight:\s*650/s,
    "helper-rendered activity labels should carry title weight through a scoped hook"
  );
  assert.match(
    mainStyleSource,
    /\.compare-task-header\s*{[^}]*display: flex[^}]*align-items: center[^}]*gap: 8px[^}]*flex-wrap: wrap[\s\S]*\.compare-task-label\s*{[^}]*font-weight:\s*650[\s\S]*\.compare-task-status\s*{[^}]*max-width: min\(180px, 100%\)/s,
    "compare task cards should keep scoped header, label, and status layout hooks"
  );
  assert.match(
    mainStyleSource,
    /\.coordination-issue-tone,\s*\.test-run-status\s*{[^}]*display: inline-flex[^}]*max-width: min\(180px, 100%\)[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "helper status labels should stay bounded without reusing tool badge fallback classes"
  );
  assert.doesNotMatch(mainStyleSource, /\.skip-links/, "retired visible landmark strip styles should be removed from the toolbar");
  assert.match(
    mainStyleSource,
    /\.toolbar-capabilities-trigger\s*{[^}]*display: inline-grid[^}]*place-items: center[^}]*width: 32px[^}]*max-width: 32px/s,
    "toolbar capabilities should render as a compact icon-first trigger"
  );
  assert.match(
    mainStyleSource,
    /\.header-details-trigger \[data-icon="inline-start"\],[\s\S]*\.toolbar-capabilities-trigger \[data-icon="inline-start"\][\s\S]*width:\s*16px[\s\S]*height:\s*16px/s,
    "header floating trigger icons should size shadcn data-icon hooks instead of retired .icon wrappers"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capabilities-trigger:{1,2}after\s*{[^}]*content: none/s,
    "toolbar capabilities should not add a visible disclosure chevron next to the icon"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capabilities-trigger\[aria-expanded="true"\]:{1,2}after\s*{[^}]*content: none/s,
    "open toolbar capabilities should keep the icon trigger visually minimal"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability-grid\s*{[^}]*max-height: min\(360px, calc\(100vh - 84px\)\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable/s,
    "toolbar capability menu should stay bounded with contained scrolling"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability-list\s*{[^}]*display: grid[^}]*grid-template-columns: repeat\(auto-fit, minmax\(104px, 1fr\)\)[^}]*min-width: 0/s,
    "toolbar capability groups should use a compact responsive icon-era card grid"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability strong\s*{[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "toolbar capability labels should stay compact inside the capability inspector"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability small\s*{[^}]*max-height: min\(46px, 12vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin[^}]*user-select: text/s,
    "toolbar capability details should scroll inside each card instead of ballooning the toolbar menu"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability-actions\s*{[^}]*display: flex[^}]*flex-wrap: wrap[^}]*border-top: 1px solid var\(--border-subtle\)/s,
    "toolbar capability actions should remain grouped under the capability matrix"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability-actions button\s*{[^}]*flex: 0 0 30px[^}]*width: 30px[^}]*overflow: hidden[^}]*transition:[^}]*transform (?:80ms|\.08s) ease-out/s,
    "toolbar capability action buttons should render as fixed icon controls with pressed feedback"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability-actions button:{1,2}before\s*{[^}]*content: none/s,
    "toolbar capability action buttons should avoid extra meter chrome"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability-actions button span\s*{[^}]*min-width: 0[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "toolbar capability action labels should truncate before widening the popover"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability-actions button\[data-action="?toggleReview"?\]\s*{[^}]*border-color: var\(--state-warning-border\)[^}]*background: color-mix\(in srgb, var\(--trail-review\) 8%, var\(--surface-subtle\)\)[^}]*\}[\s\S]*\.toolbar-capability-actions button\[data-action="?toggleReview"?\] \[data-icon="inline-start"\]\s*{[^}]*color: var\(--trail-review\)/s,
    "toolbar review capability action should carry review-gate semantics"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability-actions button\[data-action="?openSettings"?\]\s*{[^}]*border-color: var\(--state-provider-border\)[^}]*background: color-mix\(in srgb, var\(--trail-provider\) 7%, var\(--surface-subtle\)\)[^}]*\}[\s\S]*\.toolbar-capability-actions button\[data-action="?openSettings"?\] \[data-icon="inline-start"\]\s*{[^}]*color: var\(--trail-provider\)/s,
    "toolbar settings capability action should carry provider/config semantics"
  );
  assert.match(
    mainStyleSource,
    /\.header-detail-body\s*{[^}]*max-height: min\(360px, calc\(100vh - 84px\)\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable/s,
    "header detail menu should stay bounded with contained scrolling"
  );
  assert.match(
    mainStyleSource,
    /\.header-detail-chips\s*{[^}]*align-items: stretch[^}]*min-width: 0/s,
    "header detail chips should align into a stable wrapped grid"
  );
  assert.match(
    mainStyleSource,
    /\.header-detail-chips \.toolbar-chip\s*{[^}]*flex: 1 1 min\(220px, 100%\)/s,
    "header detail chips should reserve readable width for long session facts"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group\s*{[^}]*max-width: 100%[^}]*border: 1px solid color-mix\(in srgb, var\(--border-subtle\) 74%, transparent\)[^}]*background: color-mix\(in srgb, var\(--surface-muted\) 38%, transparent\)[^}]*padding: 2px/s,
    "top toolbar actions should render as compact grouped controls"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group \.toolbar-action-button\s*{[^}]*max-width: 100%/s,
    "top toolbar primary actions should stay bounded as icon controls"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group button\[data-header-icon-only="true"\]\s*{[^}]*position: relative[^}]*border-color: color-mix\(in srgb, var\(--border-subtle\) 62%, transparent\)[^}]*background: color-mix\(in srgb, var\(--surface-muted\) 28%, transparent\)/s,
    "top toolbar icon buttons should share a refined command surface"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group button\[data-header-icon-only="true"\]:{1,2}after\s*{[^}]*content: ""[^}]*inset-inline: 8px[^}]*height: 2px[^}]*background: currentColor[^}]*transform: scaleX\((?:0)?\.38\)/s,
    "top toolbar icon buttons should expose compact typed meters without extra DOM"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group button\[data-header-icon-only="true"\]:hover:{1,2}after,[\s\S]*\.header-action-group button\[data-header-icon-only="true"\]\.active:{1,2}after\s*{[^}]*opacity: (?:0)?\.76[^}]*transform: scaleX\((?:0)?\.92\)/s,
    "top toolbar icon meters should become readable on hover, focus, and active states"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group button\[data-header-icon-only="true"\]\[data-action="?toggleReview"?\],[\s\S]*\.header-action-group button\[data-header-icon-only="true"\]\[data-action="?openDiff"?\]\s*{[^}]*color: var\(--trail-review\)/s,
    "review and diff header icons should carry review/change semantics"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group button\[data-header-icon-only="true"\]\[data-action="?openSettings"?\]\s*{[^}]*color: var\(--trail-provider\)[^}]*\}[\s\S]*\.header-action-group button\[data-header-icon-only="true"\]\[data-action="?refresh"?\]\s*{[^}]*color: var\(--trail-lane\)[^}]*\}[\s\S]*\.header-action-group button\[data-header-icon-only="true"\]\[data-action="?cancel"?\]\s*{[^}]*color: var\(--trail-risk\)/s,
    "settings, refresh, and cancel header icons should carry provider, lane, and risk semantics"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group button\[data-header-icon-only="true"\]:disabled\s*{[^}]*color: var\(--trail-muted\)[^}]*\}[\s\S]*\.header-action-group button\[data-header-icon-only="true"\]:disabled:{1,2}after\s*{[^}]*opacity: (?:0)?\.14[^}]*transform: scaleX\((?:0)?\.32\)/s,
    "disabled header icon commands should visibly damp both icon and meter"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-action-button\s*{[^}]*position: relative[^}]*overflow: hidden[^}]*transition:[^}]*transform (?:80ms|\.08s) ease-out/s,
    "top toolbar primary actions should keep stable geometry and pressed feedback"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-action-button:{1,2}before\s*{[^}]*content: none/s,
    "top toolbar primary actions should avoid extra meter chrome"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-action-button\[data-action="?dryRunApply"?\],[\s\S]*\.toolbar-action-button\[data-action="?startFollowUp"?\]\s*{[^}]*border-color: var\(--state-success-border\)[^}]*background: color-mix\(in srgb, var\(--trail-checkpoint\) 8%, var\(--surface-muted\)\)/s,
    "dry-run and follow-up toolbar actions should read as checkpoint-backed workflow controls"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-action-button\[data-action="?focusReview"?\],[\s\S]*\.toolbar-action-button\[data-action="?focusTranscript"?\]\s*{[^}]*border-color: var\(--state-warning-border\)[^}]*background: color-mix\(in srgb, var\(--trail-review\) 9%, var\(--surface-muted\)\)/s,
    "review and approval toolbar actions should carry a review-gate surface"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-action-button\[data-action="?focusComposer"?\] \[data-icon="inline-start"\],[\s\S]*\.toolbar-action-button\[data-action="?refresh"?\] \[data-icon="inline-start"\]\s*{[^}]*color: var\(--trail-lane\)/s,
    "compose and refresh toolbar actions should carry a lane icon accent"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-action-button\[data-action="?cancel"?\] \[data-icon="inline-start"\]\s*{[^}]*color: var\(--trail-risk\)/s,
    "cancel toolbar actions should keep a distinct risk accent"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-chip strong\s*{[^}]*font-variant-numeric:\s*tabular-nums/s,
    "toolbar numeric chip values should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-chip\s*{[^}]*overflow: hidden[^}]*user-select: text/s,
    "toolbar chips should contain and allow selecting long run identifiers"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-chip:{1,2}before\s*{[^}]*content: ""[^}]*flex: 0 0 3px[^}]*background: var\(--border-subtle\)/s,
    "toolbar chips should render compact semantic meters without extra DOM"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-chip-ok:{1,2}before\s*{[^}]*background: var\(--trail-checkpoint\)[^}]*\}[\s\S]*\.toolbar-chip-warning:{1,2}before\s*{[^}]*background: var\(--trail-review\)[^}]*\}[\s\S]*\.toolbar-chip-blocked:{1,2}before\s*{[^}]*background: var\(--trail-risk\)[^}]*\}[\s\S]*\.toolbar-chip-active:{1,2}before\s*{[^}]*background: var\(--trail-lane\)/s,
    "toolbar chip meters should map Trail states to distinct operational colors"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-chip span\s*{[^}]*flex: 0 1 auto[^}]*min-width: 0[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "toolbar chip labels should truncate before crowding values"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability\s*{[^}]*position: relative[^}]*padding: 7px 8px 7px 14px/s,
    "toolbar capability cards should reserve room for semantic status meters"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability:{1,2}before\s*{[^}]*content: ""[^}]*inset-inline-start: 7px[^}]*width: 3px[^}]*background: var\(--border-subtle\)/s,
    "toolbar capability cards should render a compact status meter without extra markup"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-capability\.on\s*{[^}]*background: color-mix\(in srgb, var\(--trail-checkpoint\) 7%, var\(--surface-subtle\)\)[^}]*\}[\s\S]*\.toolbar-capability\.on:{1,2}before\s*{[^}]*background: var\(--trail-checkpoint\)/s,
    "ready toolbar capabilities should carry a success surface and meter"
  );
  assert.match(
    mainStyleSource,
    /\.provider,\s*\.provider-chip\s*{[^}]*display: inline-flex[^}]*min-width: 0[^}]*max-width: 100%[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "provider chips should stay bounded for long provider names"
  );
  assert.match(
    mainStyleSource,
    /\.capabilities\s*{[^}]*min-width: 0[^}]*max-width: 100%/s,
    "capability chip groups should shrink inside narrow rows"
  );
  assert.match(
    mainStyleSource,
    /\.capability-chip\s*{[^}]*display: inline-flex[^}]*min-width: 0[^}]*max-width: 100%[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "capability chips should truncate instead of widening cards"
  );
  assert.match(
    mainStyleSource,
    /\.coordination-chip\s*{[^}]*min-width: 0[^}]*max-width: 100%[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "coordination chips should stay single-line and bounded"
  );
  assert.match(
    mainStyleSource,
    /@media \(pointer: coarse\)\s*{[\s\S]*\.toolbar-capabilities-trigger,[\s\S]*\.timeline-filter-trigger,[\s\S]*\.composer-controls-summary,[\s\S]*\.settings-nav a,[\s\S]*min-height:\s*44px/s,
    "toolbar, filter, and composer controls should keep coarse-pointer touch targets"
  );
  assert.match(
    mainStyleSource,
    /@media \(pointer: coarse\)\s*{[\s\S]*\.composer-icon-tools,\s*\.composer-action-group\s*{[^}]*max-height: min\(104px, 32vh\)/s,
    "composer toolbar containers should allow wrapped coarse-pointer controls without cramped clipping"
  );
  assert.match(
    mainStyleSource,
    /@media \(pointer: coarse\)\s*{[\s\S]*\.tool-summary,[\s\S]*\.timeline-group-summary,[\s\S]*\.thought-summary,[\s\S]*\.payload-summary,[\s\S]*min-height:\s*44px[\s\S]*touch-action:\s*manipulation/s,
    "collapsible inspector rows should keep comfortable coarse-pointer targets"
  );
  assert.match(
    mainStyleSource,
    /\.composer-input-frame\s*{[^}]*border: 1px solid color-mix\(in srgb, var\(--input-border\) 72%, var\(--text\)\)[^}]*box-shadow: none/s,
    "composer textarea frame should keep one quiet visible border"
  );
  assert.match(
    mainStyleSource,
    /\.composer-input-frame:focus-within\s*{[^}]*border-color: var\(--input-active-border\)[^}]*box-shadow: none/s,
    "composer textarea focus should use a single border instead of stacked rings"
  );
  assert.match(
    mainStyleSource,
    /\.composer-input:focus-visible\s*{[^}]*outline:\s*0/s,
    "composer textarea should not add a nested browser focus outline inside the focused frame"
  );
  assert.match(
    mainSource,
    /COMPOSER_PROMPT_PRESETS[\s\S]*=\s*\[[\s\S]*id: "implement"[\s\S]*id: "review"[\s\S]*id: "test"[\s\S]*id: "explain"/,
    "composer should expose focused prompt starter presets"
  );
  assert.match(
    mainSource,
    /composerSendMode[\s\S]*=\s*"fast"[\s\S]*isComposerSendMode\(restoredState\?\.composerSendMode\)[\s\S]*composerSendMode = restoredState\.composerSendMode/,
    "composer send mode should persist across webview refreshes"
  );
  assert.match(
    mainSource,
    /event\.key === "Enter"[\s\S]*composerSendMode === "fast"[\s\S]*sendPrompt\(\)/,
    "composer draft mode should allow plain Enter to create new lines"
  );
  assert.match(
    mainSource,
    /function insertPromptPreset\(presetId\)[\s\S]*insertComposerText\(input, preset\.text\)/,
    "composer prompt presets should insert at the current draft cursor"
  );
  assert.match(
    mainSource,
    /function clearComposerDraft\(\)[\s\S]*composerDraft = ""[\s\S]*syncComposerAffordances\(\)/,
    "composer should provide a clear-draft action without touching attachments"
  );
  assert.match(
    mainSource,
    /composerRailItems\(\{[\s\S]*statusTone: status\.tone[\s\S]*attachmentModes: attachments\.map\(attachmentMode\)[\s\S]*sendMode: composerSendMode/,
    "composer should keep data-driven prompt context metadata available"
  );
  assert.doesNotMatch(composerCardSourceTs, /className="composer-context-rail"/, "composer context rail should not occupy the prompt surface");
  assert.match(composerCardSourceTs, /aria-invalid=\{props\.draft\.tone === "limit" \? "true" : undefined\}/, "composer limit state should mark the prompt field invalid");
  assert.match(
    composerModelSourceTs,
    /Shorten the prompt or move bulky context into attachments before sending\./,
    "composer send gate should explain prompt-limit recovery"
  );
  assert.match(
    mainSource,
    /FLOATING_DETAILS_SELECTOR = "\.composer-controls,\.header-details,\.toolbar-capabilities,\.timeline-toolbar"/,
    "composer, header, capability, and transcript filter menus should share one floating-details registry"
  );
  assert.match(
    mainStyleSource,
    /\.composer-controls-summary \[data-icon="inline-start"\],[\s\S]*\.composer-controls-summary \[data-icon="inline-start"\] > svg\s*{[^}]*width:\s*var\(--composer-control-icon-size\)[^}]*height:\s*var\(--composer-control-icon-size\)/s,
    "composer floating controls trigger should size shadcn data-icon hooks instead of retired .icon wrappers"
  );
  assert.match(
    mainSource,
    /function closeFloatingDetails\(except, restoreFocus = (?:false|!1)\)[\s\S]*querySelectorAll(?:<[^>]+>)?\(`\$\{FLOATING_DETAILS_SELECTOR\}\[data-floating-open="true"\]`\)[\s\S]*dispatchFloatingMenuClose\(\{ except, restoreFocus \}\)/,
    "floating detail menus should light-dismiss and restore keyboard focus when requested"
  );
  assert.match(
    mainSource,
    /function closeComposerControls\(\)[\s\S]*closeFloatingDetails\(\)/,
    "composer command insertion should use the shared floating-details close helper"
  );
  assert.match(
    mainSource,
    /function insertSlashCommand\(commandName, hint\)[\s\S]*closeComposerControls\(\)[\s\S]*input\.focus\(\)/,
    "inserting a slash command should close the composer controls and return focus to the prompt"
  );
  assert.match(
    mainSource,
    /activeFloatingDetails = target\?\.closest(?:<[^>]+>)?\(FLOATING_DETAILS_SELECTOR\)[\s\S]*closeFloatingDetails\(activeFloatingDetails \|\| (?:undefined|void 0)\)/,
    "opening one floating detail menu should close any sibling menu"
  );
  assert.match(
    mainSource,
    /event\.key === "Escape" && closeFloatingDetails\((?:undefined|void 0), (?:true|!0)\)/,
    "floating detail menus should close from Escape before global shortcuts continue"
  );
  assert.match(mainSource, /data-live-announcement/, "webview should expose a stable live region for local action feedback");
  assert.match(mainSource, /function announceToast\(message, tone\)/, "local toasts should share an announcement helper");
  assert.match(mainSource, /liveRegion\.textContent = message/, "local action feedback should update the live region without rerendering");
  assert.match(
    mainSource,
    /node\.setAttribute\("role", tone === "error" \? "alert" : "status"\)/,
    "local toasts should expose assertive error and polite status semantics"
  );
  assert.match(
    mainSource,
    /function copyTextToClipboard\(text, label, successMessage\)/,
    "copy actions should share one clipboard feedback path"
  );
  assert.match(mainSource, /No \$\{label\} available to copy\./, "empty copy targets should explain why nothing happened");
  assert.match(mainSource, /Clipboard API unavailable/, "copy actions should handle missing clipboard API support");
  assert.match(mainSource, /document\.execCommand\("copy"\)/, "copy actions should keep a fallback copy path");
  assert.match(mainSource, /drawerRestoreFocus/, "drawers should remember the opener for focus restoration");
  assert.match(
    mainSource,
    /function configureJsonDrawer\(drawer, label\)[\s\S]*setAttribute\("aria-modal", "true"\)/,
    "webview drawers should expose modal dialog semantics"
  );
  assert.match(
    mainSource,
    /function prepareJsonDrawer\(\)[\s\S]*closeJsonDrawer\(\{ restoreFocus: (?:false|!1) \}\)[\s\S]*!active\.closest\("\.json-drawer"\)/,
    "opening a drawer should preserve the non-drawer trigger as the focus return target"
  );
  assert.match(
    mainSource,
    /function mountJsonDrawer\(drawer\)[\s\S]*\[data-action='closeDrawer'\][\s\S]*\.focus\(\)/,
    "mounted drawers should move keyboard focus to the close affordance"
  );
  assert.match(
    mainSource,
    /function mountJsonDrawer\(drawer\)[\s\S]*setAppModalInert\((?:true|!0)\)/,
    "mounted drawers should make the underlying webview inert"
  );
  assert.match(
    webviewSource,
    /function mountResultDrawer\(props: ResultDrawerProps\): void \{[\s\S]*setAppModalInert\(true\)[\s\S]*import\("\.\/ResultDrawer\.js"\)[\s\S]*module\.mountResultDrawer\(\{/,
    "generic result drawers should mount through the lazy shadcn drawer island"
  );
  assert.match(
    webviewSource,
    /function openJsonDrawer\([\s\S]*mountResultDrawer\(\{[\s\S]*bodyHtml: drawer\.innerHTML[\s\S]*function openCompareDrawer\([\s\S]*mountResultDrawer\(\{[\s\S]*className: "compare-drawer"[\s\S]*function openConflictDrawer\([\s\S]*mountResultDrawer\(\{[\s\S]*className: "conflict-drawer"/,
    "JSON, compare, and conflict drawers should share the shadcn result drawer shell"
  );
  assert.match(
    webviewSource,
    /function resultDrawerWidgetHost\(id: string\)[\s\S]*data-result-drawer-widget/,
    "result drawer helper disclosures should mount through React widget placeholders"
  );
  assert.match(
    webviewSource,
    /function comparePathAccordionWidget\([\s\S]*type: "accordion"[\s\S]*className: "compare-paths"[\s\S]*defaultOpenIds: shared\.length \? \["compare-paths-shared"\] : \[\]/,
    "compare path lists should be represented as shadcn accordion widgets"
  );
  assert.match(
    webviewSource,
    /function conflictItemDetails\([\s\S]*type: "accordion"[\s\S]*className: "conflict-details"[\s\S]*triggerClassName: "conflict-details-summary"/,
    "conflict nested details should be represented as shadcn accordion widgets"
  );
  assert.doesNotMatch(
    webviewSource,
    /<details class="(?:compare-paths|conflict-details)"/,
    "compare and conflict drawer disclosures should not use native details markup"
  );
  assert.match(
    webviewSource,
    /function payloadDisclosure\([\s\S]*data-payload-disclosure-root[\s\S]*function rawDetailsView/,
    "resource, media, raw, and unsupported helper disclosures should route through the lazy payload island"
  );
  assert.doesNotMatch(
    webviewSource,
    /<details class="(?:resource|media-preview|raw|unsupported)"/,
    "resource, media, raw, and unsupported helper disclosures should not use native details markup"
  );
  assert.doesNotMatch(
    webviewSource,
    /drawer\.className = "json-drawer(?: compare-drawer| conflict-drawer)?"[\s\S]*drawer\.innerHTML = `\s*<div class="drawer-header">/,
    "generic result drawers should not keep the retired imperative header frame"
  );
  assert.match(
    mainSource,
    /function setAppModalInert\(inert\)[\s\S]*toggleAttribute\("inert", inert\)[\s\S]*setAttribute\("aria-hidden", "true"\)[\s\S]*removeAttribute\("aria-hidden"\)/,
    "modal drawers should hide and restore the app root for assistive technology"
  );
  assert.match(
    mainSource,
    /function closeJsonDrawer\(options = \{\}\)[\s\S]*target\.focus\(\{ preventScroll: (?:true|!0) \}\)/,
    "closing a drawer should restore focus without scrolling the transcript"
  );
  assert.match(
    mainSource,
    /function closeJsonDrawer\(options = \{\}\)[\s\S]*setAppModalInert\((?:false|!1)\)/,
    "closing a drawer should release the underlying webview from modal inertness"
  );
  assert.match(
    mainSource,
    /if \(handleJsonDrawerKeydown\(event\)\)\s*(?:\{\s*return;\s*\}|return;)/,
    "open drawers should get first chance at keyboard input before global shortcuts"
  );
  assert.match(
    mainSource,
    /function activeJsonDrawer\(\)[\s\S]*querySelector(?:<[^>]+>)?\("\.json-drawer"\)/,
    "drawer keyboard handling should share a single active drawer lookup"
  );
  assert.match(
    mainSource,
    /function handleJsonDrawerKeydown\(event\)[\s\S]*activeJsonDrawer\(\)[\s\S]*event\.key === "Escape"[\s\S]*closeJsonDrawer\(\)[\s\S]*event\.key === "Tab"[\s\S]*trapJsonDrawerFocus\(event\)[\s\S]*(?:true|!0)/,
    "open drawers should close on Escape, trap Tab, and suppress global shortcuts"
  );
  assert.match(
    mainSource,
    /function trapJsonDrawerFocus\(event\)[\s\S]*drawerFocusableElements\(drawer\)[\s\S]*event\.shiftKey && active === first[\s\S]*last\.focus\(\)[\s\S]*!event\.shiftKey && active === last[\s\S]*first\.focus\(\)/,
    "drawer Tab handling should cycle between the first and last focusable controls"
  );
  assert.match(
    mainSource,
    /function drawerFocusableElements\(drawer\)[\s\S]*querySelectorAll(?:<[^>]+>)?\(DRAWER_FOCUSABLE_SELECTOR\)[\s\S]*filter\(isVisibleFocusable\)/,
    "drawer focus trap should collect a filtered list of focusable controls"
  );
  assert.doesNotMatch(mainSource, /DRAWER_FOCUSABLE_SELECTOR\s*=\s*\[[\s\S]*\.join\(","\)/, "drawer focus selector should not allocate during startup");
  assert.match(
    mainSource,
    /function isVisibleFocusable\(element\)[\s\S]*aria-hidden"\) === "true"[\s\S]*getClientRects\(\)\.length > 0/,
    "drawer focus trap should ignore hidden controls"
  );
  assert.match(mainStyleSource, /\.composer-input-frame:hover\s*{[^}]*border-color/s, "composer prompt frame should expose hover feedback");
  assert.match(
    mainStyleSource,
    /\.composer-utility-row\s*{[^}]*display:\s*none/s,
    "composer helper controls should not occupy the prompt surface"
  );
  assert.doesNotMatch(composerCardSourceTs, /data-action="insertPromptPreset"|data-action="setComposerSendMode"|data-composer-clear/, "composer prompt starters, mode toggle, and clear action should be removed from the visible prompt surface");
  assert.match(
    mainStyleSource,
    /\.composer-context-rail\s*{[^}]*display:\s*none/s,
    "composer context rail should not occupy vertical space above the input"
  );
  assert.match(
    mainStyleSource,
    /\.composer-context-chip\s*{[^}]*display: inline-flex[^}]*max-width: min\(220px, 100%\)[^}]*user-select: text/s,
    "composer context chips should stay bounded and selectable"
  );
  assert.match(mainStyleSource, /\.composer-icon-tools\s*{[^}]*max-width: 100%/s, "composer tool row should be width constrained");
  assert.match(
    mainStyleSource,
    /\.composer-actions\s*{[^}]*display:\s*flex[^}]*flex-wrap:\s*wrap[^}]*padding:\s*6px 0 0/s,
    "composer actions should use a flat wrapping command bar below the input"
  );
  assert.match(
    mainStyleSource,
    /\.composer button\[data-composer-icon-only="true"\]\s*{[^}]*width: var\(--composer-control-size\)[^}]*height: var\(--composer-control-size\)[^}]*background: transparent/s,
    "composer icon controls should share the standard Trail control size"
  );
  assert.match(
    mainStyleSource,
    /\.composer button\[data-composer-icon-only="true"\]:{1,2}after\s*{[^}]*content:\s*none/s,
    "composer icon controls should not add extra underline accents"
  );
  assert.match(
    mainStyleSource,
    /\.composer button\[data-composer-icon-only="true"\]\.send-button\s*{[^}]*color: var\(--button-primary-fg\)[^}]*background: var\(--button-primary-bg\)/s,
    "composer send control should keep primary button treatment"
  );
  assert.match(
    mainStyleSource,
    /\.composer \[data-action="?attachSelection"?\],[\s\S]*\.composer \[data-action="?attachChangedFiles"?\]\s*{[^}]*color: var\(--trail-lane\)[^}]*background: color-mix\(in srgb, var\(--trail-lane\) 7%, transparent\)/s,
    "selection and changed-file composer controls should carry a lane/workspace accent"
  );
  assert.match(
    mainStyleSource,
    /\.composer \[data-action="?attachFile"?\],[\s\S]*\.composer \[data-action="?attachHistory"?\]\s*{[^}]*color: var\(--trail-provider\)[^}]*background: color-mix\(in srgb, var\(--trail-provider\) 7%, transparent\)/s,
    "file and history composer controls should carry a provider/context accent"
  );
  assert.match(
    mainStyleSource,
    /\.composer button\[data-composer-icon-only="true"\]:disabled\s*{[^}]*color: var\(--trail-muted\)[^}]*background: transparent/s,
    "disabled composer icon controls should remain inert after action-specific accents"
  );
  assert.match(
    mainStyleSource,
    /\.composer-actions\s*{[^}]*border:\s*0[^}]*background:\s*transparent[^}]*padding:\s*6px 0 0/s,
    "composer actions should not add a nested divider below the input"
  );
  assert.match(mainSource, /contextUsageGauge\(usage\)/, "composer toolbar should render the current context usage gauge");
  assert.match(
    mainStyleSource,
    /\.composer-context-gauge\s*{[^}]*place-items: center[^}]*width: var\(--composer-control-size\)[^}]*height: var\(--composer-control-size\)[^}]*border-radius: 50%/s,
    "composer context gauge should reserve stable circular geometry"
  );
  assert.match(
    mainStyleSource,
    /\.composer-context-gauge\s*{[^}]*conic-gradient\(var\(--context-gauge-color\) var\(--context-pct\)[\s\S]*\.composer-context-gauge > span\s*{[^}]*border-radius: 50%/s,
    "composer context gauge should render as a circular progress indicator"
  );
  assert.match(
    mainStyleSource,
    /\.attachment-list\s*{[^}]*display: grid[^}]*grid-template-columns: repeat\(auto-fit, minmax\(min\(220px, 100%\), 1fr\)\)[^}]*max-height: min\(132px, 26vh\)[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "composer attachment shelf should stay bounded and scroll-stable with many attachments"
  );
  assert.match(
    mainStyleSource,
    /\.attachment-list \.attachment-chip > span\s*{[^}]*flex: 1 1 auto[^}]*\}[\s\S]*\.attachment-list \.attachment-chip button\s*{[^}]*margin-inline-start: auto/s,
    "composer attachment chips should reserve flexible label space and anchor remove controls"
  );
  assert.match(
    mainStyleSource,
    /\.composer-icon-tools,\s*\.composer-action-group\s*{[^}]*position: relative[^}]*border:\s*0[^}]*background:\s*transparent[^}]*padding:\s*0/s,
    "composer icon rows should render as flat compact command groups"
  );
  assert.match(
    mainStyleSource,
    /\.composer-icon-tools,\s*\.composer-action-group\s*{[^}]*max-height: none[^}]*overflow: visible/s,
    "composer toolbar groups should not create nested scroll boxes"
  );
  assert.match(
    mainStyleSource,
    /\.composer-icon-tools:{1,2}before,\s*\.composer-action-group:{1,2}before\s*{[^}]*content:\s*none/s,
    "composer command groups should not add side-meter decoration"
  );
  assert.match(
    mainStyleSource,
    /\.composer button\[data-composer-icon-only="true"\]:not\(:disabled\):hover,[\s\S]*\.composer button\[data-composer-icon-only="true"\]:focus-visible\s*{[^}]*border-color: var\(--border-subtle\)[^}]*background: var\(--surface-hover\)/s,
    "composer icon buttons should show button-level hover and focus feedback"
  );
  assert.match(
    mainStyleSource,
    /\.composer-session\s*{[^}]*max-height: min\(360px, calc\(100vh - 160px\)\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable/s,
    "composer control menu should stay bounded with contained scrolling"
  );
  assert.match(
    mainStyleSource,
    /\.select-control span\s*{[^}]*min-width: 0[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "selector labels should truncate instead of widening dense controls"
  );
  assert.match(
    mainStyleSource,
    /\.select-control select\s*{[^}]*min-width: 0[^}]*max-width: min\(180px, 100%\)[^}]*padding-block: 2px[^}]*padding-inline: 7px 24px[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "selector controls should contain long provider and command labels with logical spacing"
  );
  assert.match(
    mainStyleSource,
    /\.select-control select:hover\s*{[^}]*border-color: var\(--input-active-border\)[^}]*background:/s,
    "selector controls should expose hover feedback"
  );
  assert.match(
    mainStyleSource,
    /\.select-control select:focus-visible\s*{[^}]*border-color: var\(--input-active-border\)[^}]*outline: 0[^}]*box-shadow: var\(--focus-ring\)/s,
    "selector controls should use the shared strong focus language"
  );
  assert.match(
    mainStyleSource,
    /\.select-control select:disabled\s*{[^}]*cursor: not-allowed[^}]*opacity: (?:0)?\.66/s,
    "selector controls should communicate disabled state"
  );
  assert.match(
    mainStyleSource,
    /\.composer-session \.select-control\s*{[^}]*flex: 1 1 172px[^}]*\}[\s\S]*\.composer-session \.select-control select\s*{[^}]*flex: 1 1 auto[^}]*max-width: 100%/s,
    "composer agent-control selectors should flex inside the floating menu"
  );
  assert.match(
    mainStyleSource,
    /\.composer-input\s*{[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable/s,
    "composer textarea should keep nested scrolling contained"
  );
  assert.match(
    mainStyleSource,
    /\.composer-input-footer\s*{[^}]*font-variant-numeric: tabular-nums/s,
    "composer counters should keep stable numeric alignment"
  );
  assert.match(
    mainStyleSource,
    /\.composer-draft-copy strong\s*{[^}]*max-width: min\(48%, 160px\)[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "composer draft state labels should truncate before crowding the meter"
  );
  assert.match(
    mainStyleSource,
    /\.composer-meter\s*{[^}]*justify-self: end[^}]*width: min\(128px, 100%\)/s,
    "composer prompt meter should keep a bounded stable width"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 420px\)\s*{[\s\S]*\.composer-icon-tools,\s*\.composer-action-group\s*{[\s\S]*width: 100%/s,
    "phone composer toolbar groups should wrap cleanly"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 420px\)\s*{[\s\S]*\.composer-context-gauge\s*{[^}]*width: var\(--composer-control-size\)/s,
    "phone composer context gauge should keep the same compact control size"
  );
  assert.match(
    mainStyleSource,
    /--focus-ring:\s*0 0 0 2px color-mix\(in srgb, var\(--focus\) 24%, transparent\)/,
    "webview controls should share a visible theme-aware keyboard focus ring"
  );
  assert.match(
    mainStyleSource,
    /button\.icon-only\s*{[^}]*display: inline-grid[^}]*place-items: center[^}]*aspect-ratio: 1[^}]*overflow: hidden[^}]*line-height: 1[^}]*vertical-align: middle/s,
    "icon-only controls should keep stable square geometry and centered glyphs"
  );
  assert.doesNotMatch(mainStyleSource, /\.icon-button/, "retired icon-button CSS should stay out of the shadcn webview stylesheet");
  assert.match(
    mainStyleSource,
    /:is\([^}]*\.toolbar-action-button[^}]*button\.timeline-filter[^}]*button\.resource-chip[^}]*button\.chip-button[^}]*button\.empty-action[^}]*\.settings-action-button[^}]*\.settings-nav a[^}]*\):focus-visible\s*{[^}]*box-shadow:[^}]*var\(--focus-ring\)/s,
    "dense developer controls should keep a stronger focus-visible treatment"
  );
  assert.match(
    mainStyleSource,
    /\.toast,\s*\.json-drawer\s*{[^}]*inset-inline-end:\s*16px/s,
    "floating feedback and drawers should use logical inline placement"
  );
  assert.match(
    mainStyleSource,
    /\.json-drawer\s*{[^}]*display:\s*grid[^}]*align-content:\s*start[^}]*gap:\s*10px[^}]*overflow:\s*auto[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "result drawers should render as bounded scroll-stable inspector panels"
  );
  assert.match(
    mainStyleSource,
    /\.drawer-header\s*{[^}]*position:\s*sticky[^}]*top:\s*-14px[^}]*border-bottom:\s*1px solid var\(--border-subtle\)[^}]*padding:\s*10px 14px/s,
    "result drawer headers should stay reachable while inspecting long output"
  );
  assert.match(
    mainStyleSource,
    /\.result-drawer-title\s*{[^}]*display:\s*grid[^}]*min-width:\s*0[\s\S]*\.result-drawer-title \[data-slot="drawer-description"\]\s*{[^}]*color:\s*var\(--trail-muted\)[^}]*overflow-wrap:\s*anywhere/s,
    "shadcn result drawer titles and descriptions should stay readable for long provider labels"
  );
  assert.match(
    mainStyleSource,
    /\.result-drawer-actions\s*{[^}]*display:\s*flex[^}]*align-items:\s*center[^}]*gap:\s*6px[\s\S]*\.result-drawer-badge\s*{[^}]*max-width:\s*min\(142px, 28vw\)/s,
    "shadcn result drawer status badges should not crowd the close button"
  );
  assert.match(
    mainStyleSource,
    /\.result-drawer-body\s*{[^}]*display:\s*grid[^}]*gap:\s*10px[^}]*min-width:\s*0/s,
    "shadcn result drawer bodies should preserve bounded helper-rendered content flow"
  );
  assert.match(
    mainStyleSource,
    /\.json-drawer \.code-frame > \.code\s*{[^}]*max-height:\s*min\(460px, calc\(100vh - 220px\)\)/s,
    "drawer code previews should be bounded by the visible workbench height"
  );
  assert.match(
    mainStyleSource,
    /\.compare-paths-summary,\s*\.conflict-details-summary\s*{[^}]*color:\s*var\(--trail-muted\)[^}]*font-size:\s*12px/s,
    "compare and conflict drawer accordion triggers should inherit compact helper disclosure styling"
  );
  assert.match(
    mainStyleSource,
    /\.compare-paths-summary \[data-slot="accordion-trigger-icon"\],\s*\.conflict-details-summary \[data-slot="accordion-trigger-icon"\]\s*{[^}]*margin-left:\s*auto/s,
    "compare and conflict drawer accordion icons should align with trigger text"
  );
  assert.match(
    mainStyleSource,
    /\.compare-paths ul,\s*\.compare-suggestions,\s*\.conflict-list\s*{[^}]*display:\s*grid[^}]*gap:\s*6px[^}]*padding:\s*0[^}]*list-style:\s*none/s,
    "compare and conflict drawer lists should render as structured evidence rows"
  );
  assert.match(
    mainStyleSource,
    /\.compare-paths li,\s*\.compare-suggestions li,\s*\.conflict-list li\s*{[^}]*border:\s*1px solid var\(--border-subtle\)[^}]*overflow-wrap:\s*anywhere/s,
    "compare and conflict evidence rows should contain long paths and commands"
  );
  assert.match(
    mainStyleSource,
    /\.toast\s*{[^}]*grid-template-columns: auto minmax\(0, 1fr\)[^}]*max-width: min\(420px, calc\(100vw - 32px\)\)[^}]*max-height: min\(180px, calc\(100vh - 32px\)\)[^}]*overscroll-behavior: contain[^}]*overflow-wrap: anywhere/s,
    "toast feedback should stay readable for long messages without covering the workbench"
  );
  assert.match(
    mainStyleSource,
    /\.toast\s*{[^}]*user-select: text[^}]*white-space: pre-wrap/s,
    "toast feedback should preserve multiline messages and allow copying diagnostic text"
  );
  assert.match(
    mainStyleSource,
    /\.toast:{1,2}before\s*{[^}]*border-radius:\s*999px[^}]*background: var\(--state-lane-border\)/s,
    "toast feedback should include a non-text status marker"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 420px\)\s*{[\s\S]*\.toast\s*{[\s\S]*inset-inline:\s*12px[\s\S]*max-width:\s*none/s,
    "mobile toast feedback should fit within narrow webview panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 420px\)\s*{[\s\S]*\.json-drawer\s*{[^}]*inset-inline:\s*12px[^}]*top:\s*56px[^}]*bottom:\s*12px[^}]*width:\s*auto/s,
    "result drawers should fit within very narrow webview panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(prefers-reduced-motion: reduce\)\s*{[\s\S]*\*:{1,2}before,[\s\S]*\*:{1,2}after\s*{[\s\S]*scroll-behavior:\s*auto\s*!important[\s\S]*transition-duration:\s*0?\.01ms\s*!important/,
    "webview motion should honor reduced-motion preferences across elements and pseudo-elements"
  );
  assert.match(
    mainStyleSource,
    /\.diff-loading-bar:{1,2}after\s*{[^}]*animation:\s*none\s*!important/s,
    "diff loading shimmer should stop animating when reduced motion is requested"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*--surface:\s*Canvas;[\s\S]*--focus-ring:\s*0 0 0 2px Highlight/,
    "webview colors should resolve to system colors in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.toast,[\s\S]*border-color:\s*CanvasText/,
    "toast feedback should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.json-drawer,[\s\S]*\.drawer-header,[\s\S]*border-color:\s*CanvasText/,
    "result drawers should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.header-action-group,[\s\S]*border-color:\s*CanvasText/,
    "toolbar action groups should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /button:focus-visible,[\s\S]*\.code:focus-visible\s*{[\s\S]*outline:\s*2px solid Highlight/,
    "forced-colors mode should keep a system-color keyboard outline"
  );
  assert.match(
    mainStyleSource,
    /\.toolbar-run-dot,[\s\S]*\.meter:{1,2}-moz-progress-bar\s*{[\s\S]*forced-color-adjust:\s*none;[\s\S]*background:\s*Highlight/,
    "status dots and meters should stay visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /\.diff-row-removed \.diff-code-old,[\s\S]*\.code-line-removed\s*{[\s\S]*border-inline-start:\s*3px dashed CanvasText/,
    "removed diff content should keep a non-color cue in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /\.diff-code del\s*{[\s\S]*text-decoration:\s*line-through/,
    "inline diff deletions should keep a text-decoration cue in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /\.settings-search input:focus\s*{[^}]*outline:\s*2px solid Highlight/s,
    "settings search should keep a strong forced-colors focus outline"
  );
  assert.match(
    mainStyleSource,
    /\.settings-nav-filtered\s*{[^}]*opacity:\s*1[^}]*border-style:\s*dotted/s,
    "filtered settings navigation should stay legible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /\.settings-nav-active,[\s\S]*\.settings-nav a\[aria-current="?page"?\]\s*{[^}]*border-inline-start-color: var\(--state-lane-border\)/s,
    "settings navigation should expose the active section without relying on text alone"
  );
  assert.match(
    mainStyleSource,
    /\.settings-nav-warn,[\s\S]*\.provider-routing-fact-warn\s*{[\s\S]*border-style:\s*dashed/s,
    "settings warning states should use non-color cues in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /\.settings-health-attention\s*{[^}]*border-style:\s*double/s,
    "settings attention states should keep a stronger forced-colors cue"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.settings-nav-active,[\s\S]*\.settings-nav a\[aria-current="?page"?\]\s*{[^}]*outline:\s*1px solid Highlight[^}]*border-inline-start-color:\s*Highlight/s,
    "active settings navigation should keep a system-color cue in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.header-action-group button\[data-header-icon-only="true"\]:{1,2}after\s*{[^}]*background: CanvasText/s,
    "header icon command meters should remain visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.header-action-group button\[data-header-icon-only="true"\]\[data-action="?toggleReview"?\]\.active\s*{[^}]*outline: 1px solid Highlight[^}]*background: Canvas[^}]*box-shadow: none/s,
    "active review header icons should keep a system-color selected cue"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.header-action-group button\[data-header-icon-only="true"\]\[data-action="?refresh"?\]:{1,2}after\s*{[^}]*background: Highlight/s,
    "refresh header icon meters should keep a system highlight cue in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /\.capability-cell\.on\s*{[^}]*color:\s*Highlight[^}]*text-decoration:\s*underline/s,
    "settings capability cells should keep a non-color enabled cue"
  );
  assert.match(
    mainStyleSource,
    /\.settings-next-list\s*{[^}]*grid-template-columns: repeat\(4, minmax\(0, 1fr\)\)[^}]*max-height: min\(260px, 42vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "settings next steps should render as a compact bounded action rail"
  );
  assert.match(
    mainStyleSource,
    /\.settings-next-step-warn\s*{[^}]*border-inline-start-color: var\(--state-warning-border\)/s,
    "settings next steps should keep warning affordances"
  );
  assert.match(
    mainStyleSource,
    /\.status\s*{[^}]*display: inline-block[^}]*max-width: 100%[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "status badges should truncate instead of stretching dense developer surfaces"
  );
  assert.match(
    mainStyleSource,
    /\.review-command-center\s*{[^}]*position: sticky[^}]*top: -16px/s,
    "review readiness should stay visible while scrolling"
  );
  assert.match(
    mainStyleSource,
    /\.review-drawer\s*{[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "review drawer should keep nested scrolling contained and stable"
  );
  assert.match(
    mainStyleSource,
    /\.review-actions\s*{[^}]*position: sticky[^}]*max-height: min\(48vh, 420px\)[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "review action rail should remain reachable without swallowing long reviews"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-group-next\s*{[^}]*border-color: var\(--state-lane-border\)/s,
    "review next-step actions should stay visually prioritized"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-list\s*{[^}]*max-height: min\(160px, 30vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "review action groups should stay bounded when Trail exposes many commands"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-list button\s*{[^}]*flex: 0 0 32px[^}]*max-width: 32px[^}]*overflow: hidden/s,
    "review action buttons should not widen the review rail"
  );
  assert.match(
    mainStyleSource,
    /button\.review-primary-action\s*{[^}]*position: relative[^}]*width: 38px[^}]*height: 38px[^}]*overflow: hidden/s,
    "primary review actions should expose a stable icon-first command surface"
  );
  assert.match(
    mainSource,
    /REVIEW_ACTION_ICONS[\s\S]*openDiff: "diff"/,
    "review actions should map common commands to semantic Lucide icons"
  );
  assert.match(mainSource, /REVIEW_ACTION_ICONS[\s\S]*runTests: "check"/, "review test actions should use a check icon");
  assert.match(mainSource, /REVIEW_ACTION_ICONS[\s\S]*removeTask: "stop"/, "review destructive actions should use a stop icon");
  assert.match(
    mainSource,
    /iconSvg\(reviewActionIcon\(action\.action\)\)/,
    "review action buttons should render icon affordances from their command type"
  );
  assert.match(
    mainStyleSource,
    /button\.review-primary-action:{1,2}before\s*{[^}]*content: none/s,
    "primary review actions should avoid extra accent DOM"
  );
  assert.match(
    mainStyleSource,
    /button\[data-review-icon-only="true"\] \[data-icon="inline-start"\] > \.icon\s*{[^}]*width: 15px[^}]*height: 15px/s,
    "review action icons should render as compact command glyphs"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-list button\s*{[^}]*position: relative[^}]*width: 32px[^}]*padding: 0/s,
    "review action buttons should reserve stable icon geometry"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-list button:{1,2}before\s*{[^}]*content: none/s,
    "review action buttons should avoid extra accent chrome"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-list\s*{[^}]*display: flex[^}]*flex-wrap: wrap/s,
    "review inspect actions should stay in a compact wrapping icon rail"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-list button\s*{[^}]*display: inline-grid[^}]*place-items: center/s,
    "review validate actions should stay icon-first in the rail"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-list button:disabled\s*{[^}]*opacity: 0\.58/s,
    "review recovery actions should still expose disabled state"
  );
  assert.match(
    mainStyleSource,
    /\.review-action-copy\s*{[^}]*display: grid[^}]*min-width: 0[\s\S]*\.review-action-list \.review-action-copy > span\s*{[^}]*-webkit-line-clamp: 2[^}]*\}[\s\S]*\.review-action-list \.review-action-copy > small\s*{[^}]*-webkit-line-clamp: 2/s,
    "review action labels and descriptions should clamp before crowding dense review controls"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*button\[data-review-icon-only="true"\] \[data-icon="inline-start"\] > \.icon\s*{[^}]*color: CanvasText[^}]*background: Canvas[\s\S]*button\.review-primary-action\.primary \[data-icon="inline-start"\] > \.icon,[\s\S]*\.review-action-list button\.primary \[data-icon="inline-start"\] > \.icon\s*{[^}]*color: HighlightText[^}]*background: Highlight/s,
    "review action icon capsules should remain legible in forced-colors mode"
  );
  assert.doesNotMatch(mainStyleSource, /--state-checkpoint-border/, "review applied state should not use an undefined checkpoint border token");
  assert.match(
    mainStyleSource,
    /\.review-command-applied\s*{[^}]*border-inline-start-color: var\(--state-success-border\)/s,
    "applied review command state should use the defined success border token"
  );
  assert.match(
    mainStyleSource,
    /\.review-metric strong\s*{[^}]*font-variant-numeric:\s*tabular-nums/s,
    "review metric values should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.review-gate-value\s*{[^}]*font-variant-numeric:\s*tabular-nums/s,
    "review gate values should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.review-facts dd,\s*\.settings-facts dd,\s*\.provider-routing-fact dd\s*{[^}]*max-height:\s*min\(84px, 20vh\)[^}]*overflow:\s*auto[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin[^}]*user-select:\s*text/s,
    "review and settings fact values should stay bounded, scroll-stable, and selectable"
  );
  assert.match(
    mainStyleSource,
    /\.review-section > ul:not\(\.review-issue-list\):not\(\.overlap-list\)\s*{[^}]*display: grid[^}]*list-style: none/s,
    "generic review drawer lists should render as structured evidence rows"
  );
  assert.match(
    mainStyleSource,
    /\.review-section > ul:not\(\.review-issue-list\):not\(\.overlap-list\) li\s*{[^}]*border: 1px solid var\(--border-subtle\)[^}]*overflow-wrap: anywhere/s,
    "generic review evidence rows should handle long paths and transcript labels"
  );
  assert.match(
    mainStyleSource,
    /\.review-issue-list\s*{[^}]*max-height: min\(220px, 36vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "review issue lists should stay bounded when Trail reports many blockers and warnings"
  );
  assert.match(
    mainStyleSource,
    /\.overlap-list\s*{[^}]*padding-inline-start: 18px/s,
    "parallel-work review lists should keep logical indentation"
  );
  assert.doesNotMatch(mainStyleSource, /\.review-drawer ul\s*{[^}]*padding-left: 18px/s, "review drawer should not fall back to raw physical bullet indentation");
  assert.match(
    mainStyleSource,
    /\.recovery-banner,\s*\.overlap-banner\s*{[^}]*grid-template-columns: minmax\(0, 1fr\) minmax\(min\(188px, 100%\), auto\)[^}]*overflow: hidden[^}]*box-shadow: var\(--shadow-soft\)/s,
    "recovery and overlap banners should render as framed coordination gates"
  );
  assert.match(
    mainStyleSource,
    /\.overlap-paths\s*{[^}]*grid-template-columns: repeat\(auto-fit, minmax\(min\(210px, 100%\), 1fr\)\)[^}]*max-height: min\(116px, 26vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "overlap banner evidence should stay bounded and scroll-stable"
  );
  assert.match(
    mainStyleSource,
    /\.overlap-paths span\s*{[^}]*border: 1px solid var\(--border-subtle\)[^}]*overflow-wrap: anywhere/s,
    "overlap banner path rows should contain long task names and paths"
  );
  assert.match(
    mainStyleSource,
    /\.recovery-banner-badges\s*{[^}]*display: flex[^}]*flex-wrap: wrap[^}]*max-width: 100%/s,
    "recovery banner badges should wrap without the retired card-chrome wrapper"
  );
  assert.match(
    mainStyleSource,
    /\.recovery-banner-badges \.tool-status\s*{[^}]*max-width: min\(160px, 100%\)/s,
    "recovery banner status badges should stay bounded"
  );
  assert.match(
    mainStyleSource,
    /\.recovery-banner-role\s*{[^}]*font-weight:\s*650/s,
    "recovery banner eyebrow badge should carry scoped title emphasis instead of generic role styling"
  );
  assert.match(
    mainStyleSource,
    /\.recovery-actions\.inline-actions\s*{[^}]*justify-self: end[^}]*width: min\(220px, 100%\)[^}]*margin-top: 0/s,
    "recovery and overlap actions should render as a compact shared inline-action rail"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.recovery-actions button(?:\s|:|\[|,)/,
    "recovery and overlap actions should not keep bespoke recovery button CSS"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\[data-action="?startFollowUp"?\]\s*{[^}]*border-color: var\(--state-success-border\)[^}]*background: color-mix\(in srgb, var\(--trail-checkpoint\) 8%, var\(--surface-muted\)\)[^}]*\}[\s\S]*\.inline-actions button\[data-action="?startFollowUp"?\]:{1,2}before\s*{[^}]*background: var\(--trail-checkpoint\)/s,
    "start-follow-up recovery actions should read as checkpoint-backed workflow controls"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\[data-action="?focusReview"?\],[\s\S]*\.inline-actions button\[data-action="?showConflict"?\]\s*{[^}]*border-color: var\(--state-warning-border\)[^}]*background: color-mix\(in srgb, var\(--trail-review\) 7%, var\(--surface-muted\)\)[^}]*\}[\s\S]*\.inline-actions button\[data-action="?focusReview"?\]:{1,2}before,[\s\S]*\.inline-actions button\[data-action="?showConflict"?\]:{1,2}before\s*{[^}]*background: var\(--trail-review\)/s,
    "review recovery actions should carry review-gate semantics"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\[data-action="?compareTasks"?\],[\s\S]*\.inline-actions button\[data-action="?showAcpLogs"?\]\s*{[^}]*border-color: var\(--state-provider-border\)/s,
    "compare/log recovery actions should carry provider semantics"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\[data-action="?refresh"?\],[\s\S]*\.inline-actions button\[data-action="?queueMerge"?\]\s*{[^}]*border-color: var\(--state-lane-border\)/s,
    "refresh/queue recovery actions should carry lane semantics"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.event-action(?:\s|\.|:|\[|,)/,
    "checkpoint event actions should not keep retired per-button event-action CSS"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\s*{[^}]*position: relative[^}]*max-width: 100%[^}]*text-overflow: ellipsis[^}]*white-space: nowrap[^}]*transition:[^}]*transform (?:80ms|\.08s) ease-out/s,
    "inline review actions should truncate and keep polished press feedback"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button:{1,2}before\s*{[^}]*content: ""[^}]*inset-inline-start: 6px[^}]*width: 3px[^}]*background: var\(--border-subtle\)/s,
    "inline review actions should expose compact typed meters without extra markup"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\[data-action="?refresh"?\],[\s\S]*\.inline-actions button\[data-action="?queueMerge"?\]\s*{[^}]*border-color: var\(--state-lane-border\)[^}]*background: color-mix\(in srgb, var\(--trail-lane\) 7%, var\(--surface-muted\)\)/s,
    "inline refresh, test, eval, and queue actions should carry lane semantics"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\[data-action="?showConflict"?\]\s*{[^}]*border-color: var\(--state-warning-border\)[^}]*background: color-mix\(in srgb, var\(--trail-review\) 7%, var\(--surface-muted\)\)[^}]*\}[\s\S]*\.inline-actions button\[data-action="?showConflict"?\]:{1,2}before\s*{[^}]*background: var\(--trail-review\)/s,
    "inline conflict actions should read as review-gate controls"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\[data-action="?compareTasks"?\],[\s\S]*\.inline-actions button\[data-action="?openResource"?\],[\s\S]*\.inline-actions button\[data-action="?showAcpLogs"?\]\s*{[^}]*border-color: var\(--state-provider-border\)[^}]*background: color-mix\(in srgb, var\(--trail-provider\) 6%, var\(--surface-muted\)\)/s,
    "inline compare, resource, preview, and log actions should carry provider/file semantics"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.recovery-banner,\s*\.overlap-banner(?:,\s*\.empty-state)?\s*{[^}]*grid-template-columns: minmax\(0, 1fr\)[\s\S]*\.recovery-actions\s*{[^}]*justify-self: stretch[^}]*width: 100%/s,
    "recovery and overlap banners should stack actions in narrow panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.recovery-banner,[\s\S]*\.overlap-banner,[\s\S]*\.recovery-actions,[\s\S]*\.overlap-paths span,[\s\S]*border-color: CanvasText/s,
    "recovery and overlap banners should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.empty-state,[\s\S]*\.empty-actions,[\s\S]*border-color: CanvasText/s,
    "empty-state surfaces should keep visible borders in forced-colors mode without decorative meters"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.inline-actions button:{1,2}before,[\s\S]*button\.review-primary-action:{1,2}before[\s\S]*background: CanvasText/s,
    "inline action meters should remain visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.composer-icon-tools,[\s\S]*\.composer-action-group,[\s\S]*border-color: CanvasText/s,
    "composer command groups should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.composer-icon-tools:{1,2}before,[\s\S]*\.composer-action-group:{1,2}before,[\s\S]*\.inline-actions button:{1,2}before[\s\S]*background: CanvasText/s,
    "composer command group meters should remain visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*button\.review-primary-action:{1,2}before,[\s\S]*\.review-action-list button:{1,2}before\s*{[^}]*background: CanvasText/s,
    "review action accents should remain visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.toolbar-action-button:{1,2}before\s*{[^}]*background: CanvasText/s,
    "top toolbar action meters should remain visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.toolbar-capability-actions button:{1,2}before\s*{[^}]*background: CanvasText/s,
    "toolbar capability action meters should remain visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.toolbar-action-button\.primary\s*{[^}]*border-color: Highlight[^}]*color: HighlightText[^}]*background: Highlight/s,
    "primary toolbar actions should keep system-highlight contrast in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.toolbar-action-button\[data-action="?focusComposer"?\]:{1,2}before,[\s\S]*\.toolbar-action-button\[data-action="?startFollowUp"?\]:{1,2}before\s*{[^}]*background: Highlight/s,
    "positive toolbar action meters should keep a system highlight cue in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.toolbar-chip:{1,2}before,[\s\S]*\.toolbar-capability:{1,2}before\s*{[^}]*background: CanvasText/s,
    "toolbar semantic meters should remain visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.toolbar-chip-ok:{1,2}before,[\s\S]*\.toolbar-chip-active:{1,2}before,[\s\S]*\.toolbar-capability\.on:{1,2}before\s*{[^}]*background: Highlight/s,
    "positive and active toolbar meters should keep a system highlight cue in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.tool-card,[\s\S]*\.inline-actions,[\s\S]*border-color: CanvasText/s,
    "tool cards and shared action rails should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.diff-review-status-chip,[\s\S]*border-color: CanvasText/s,
    "diff review status chips should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.diff-review-status-chip:{1,2}before,[\s\S]*\.inline-actions button:{1,2}before[\s\S]*background: CanvasText/s,
    "diff review status chip meters should remain visible in forced-colors mode"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.tool-action(?:\s|:{1,2}before|,)/s,
    "forced-colors mode should not carry stale tool-action selectors after the shadcn migration"
  );
  assert.match(
    mainStyleSource,
    /\.tool-approval-heading\s*{[^}]*grid-template-columns: auto minmax\(0, 1fr\)[^}]*align-items: start/s,
    "approval prompts should collapse to a single icon plus copy row"
  );
  assert.match(
    mainStyleSource,
    /\.tool-approval-icon\s*{[^}]*width: 15px[^}]*height: 15px[^}]*color: var\(--approval-accent\)/s,
    "approval prompt icons should use one compact semantic accent"
  );
  assert.match(
    mainStyleSource,
    /\.approval-decision\s*{[^}]*display: flex[^}]*flex-wrap: wrap[^}]*justify-content: flex-start/s,
    "approval decisions should render as a compact action strip"
  );
  assert.match(
    mainStyleSource,
    /\.approval-tone-risk \.approval-impact\s*{[^}]*color: var\(--text\)/s,
    "risky approvals should keep readable compact impact copy"
  );
  assert.match(
    mainStyleSource,
    /\.approval-option-list\s*{[^}]*display: flex[^}]*flex: 0 1 auto[^}]*flex-wrap: wrap/s,
    "approval options should stay compact and wrap under long labels"
  );
  assert.match(
    mainStyleSource,
    /\.tool-approval-edit \.approval-decision,\s*\.tool-approval-read \.approval-decision\s*{[^}]*display: grid[^}]*grid-auto-flow: column[^}]*grid-auto-columns: max-content[^}]*overflow-x: auto/s,
    "edit and read approval decisions should stay on one compact row"
  );
  assert.match(
    mainStyleSource,
    /\.tool-approval-edit \.approval-option-list,\s*\.tool-approval-read \.approval-option-list\s*{[^}]*display: grid[^}]*grid-auto-flow: column[^}]*grid-auto-columns: max-content[^}]*min-width: max-content/s,
    "edit and read approval allow options should stay grouped horizontally"
  );
  assert.match(
    mainStyleSource,
    /\.tool-approval-edit button\.approval-option,[\s\S]*\.tool-approval-read button\.approval-reject\s*{[^}]*white-space: nowrap/s,
    "edit and read approval buttons should not wrap labels into stacked rows"
  );
  assert.match(
    webviewSource,
    /decisionOptions = resolved[\s\S]*permission\.options[\s\S]*\.filter\(\(option\) => !isRejectPermissionOption\(option\)\)[\s\S]*\.sort\(\(a, b\) => approvalOptionSafetyRank\(a\) - approvalOptionSafetyRank\(b\)\)/,
    "approval options should omit reject-like provider options and prefer safer allow-once choices"
  );
  assert.match(
    toolCallCardSourceTs,
    /const detail = isEdit[\s\S]*approval\.resolved[\s\S]*approval\.resolvedNote[\s\S]*<p className=\{approval\.resolved \? "approval-resolved-note" : "approval-impact"\}>\{detail\}<\/p>/,
    "resolved tool permissions should render a small decision receipt"
  );
  assert.match(
    toolCallCardSourceTs,
    /"Approve edit\?"[\s\S]*editTarget \|\| "Workspace edit"[\s\S]*isEdit \? "tool-approval-edit" : ""/,
    "edit tool permissions should collapse to a compact target and decision strip"
  );
  assert.match(
    toolCallCardSourceTs,
    /readPermissionTarget[\s\S]*displayTitle[\s\S]*`Read \$\{readPermissionTarget\}`[\s\S]*readTarget=\{readPermissionTarget \|\| undefined\}/,
    "read tool permissions should collapse long paths into a short title and target"
  );
  assert.match(
    toolCallCardSourceTs,
    /"Allow read\?"[\s\S]*isRead \? "tool-approval-read" : ""/,
    "read tool permissions should render as a compact read-specific approval card"
  );
  assert.match(
    toolCallCardSourceTs,
    /approval\.resolved \? "tool-approval-resolved" : ""/,
    "resolved tool permissions should collapse to a small decision receipt"
  );
  assert.match(
    toolCallCardSourceTs,
    /function approvalActionIcon[\s\S]*action\.kind === "reject"[\s\S]*return CircleX[\s\S]*includes\("always"\)[\s\S]*return ShieldCheck[\s\S]*return Check/,
    "approval actions should map Always allow, Allow, and Reject to clear lucide intents"
  );
  assert.match(
    toolCallCardSourceTs,
    /function isPrimaryApprovalAction[\s\S]*action\.kind !== "approve"[\s\S]*always\|forever\|persist/s,
    "approval buttons should reserve the primary fill for one-time approve actions"
  );
  assert.match(
    mainStyleSource,
    /button\.approval-option,\s*button\.approval-reject\s*{[^}]*display: inline-flex[^}]*align-items: center[^}]*justify-content: center/s,
    "approval decision buttons should use one simple inline action shape"
  );
  assert.match(
    mainStyleSource,
    /button\.approval-option,\s*button\.approval-reject\s*{[^}]*transition:/s,
    "approval decision buttons should keep polished state transitions"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /button\.approval-option,\s*button\.approval-reject\s*{[^}]*border-inline-start/s,
    "approval decision buttons should not reintroduce left accent rails"
  );
  assert.match(
    mainStyleSource,
    /button\.approval-option:not\(:disabled\):hover,[\s\S]*button\.approval-reject:not\(:disabled\):hover\s*{[^}]*border-color: var\(--input-active-border\)[^}]*background: color-mix/s,
    "approval decision buttons should provide hover feedback before committing"
  );
  assert.match(
    mainStyleSource,
    /button\.approval-option\.primary\s*{[^}]*color: var\(--button-primary-fg\)[^}]*background: var\(--button-primary-bg\)/s,
    "the one-time approve action should keep the Trail primary button fill"
  );
  assert.match(
    mainStyleSource,
    /\.approval-option > \[data-icon="inline-start"\],\s*\.approval-reject > \[data-icon="inline-start"\]\s*{[^}]*display: block[^}]*width: 14px[^}]*height: 14px[^}]*color: currentColor[^}]*background: transparent/s,
    "approval decision icons should inherit the button color instead of carrying separate color chips"
  );
  assert.match(
    mainStyleSource,
    /\.approval-decision-copy\s*{[^}]*display: block[^}]*min-width: 0[\s\S]*\.approval-decision-copy > span\s*{[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "approval decision copy should remain bounded for long provider labels"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.approval-option-(?:primary|warning|risk) > \[data-icon="inline-start"\]\s*{/,
    "approval decision icons should not reintroduce separate semantic color chips"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.approval-option > \[data-icon="inline-start"\],[\s\S]*\.approval-reject > \[data-icon="inline-start"\]\s*{[^}]*color: currentColor[^}]*background: transparent[\s\S]*button\.approval-option\.primary > \[data-icon="inline-start"\]\s*{[^}]*color: HighlightText[^}]*background: transparent/s,
    "approval decision icons should remain legible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /button\.approval-reject\s*{[^}]*border-color: color-mix\(in srgb, var\(--state-danger-border\) 58%, var\(--border-subtle\)\)[^}]*color: var\(--trail-risk\)/s,
    "reject actions should keep a quiet but explicit danger border"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.approval-decision,\s*\.approval-option-list\s*{[^}]*align-items: center[^}]*flex-direction: row/s,
    "static narrow approval gates should keep compact decision icons reachable"
  );
  assert.match(
    mainStyleSource,
    /\.approval-detail-list\s*{[^}]*grid-template-columns: repeat\(auto-fit, minmax\(min\(132px, 100%\), 1fr\)\)/s,
    "approval request details should reflow before long provider values get squeezed"
  );
  assert.match(
    mainStyleSource,
    /\.approval-detail-list dd\s*{[^}]*font-variant-numeric:\s*tabular-nums/s,
    "approval detail values should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.approval-disclosure-summary\s*{[^}]*width: fit-content[^}]*max-width: 100%/s,
    "approval disclosure labels should stay compact but bounded"
  );
  assert.doesNotMatch(
    webviewSource,
    /<details class="approval-preview|<details class="approval-request-details"/,
    "approval preview and request wrappers should render through shadcn accordion props"
  );
  assert.match(
    mainStyleSource,
    /\.approval-locations\s*{[^}]*max-height: min\(52px, 14vh\)[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "approval affected-file chips should stay bounded inside permission gates"
  );
  assert.match(
    mainStyleSource,
    /\.approval-locations \.resource-chip,[\s\S]*\.approval-locations > span\s*{[^}]*text-overflow:\s*ellipsis[^}]*white-space:\s*nowrap/s,
    "approval location chips should truncate long paths without widening cards"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary\s*{[^}]*grid-template-columns: auto minmax\(0, 1fr\) auto auto/s,
    "tool summaries should reserve space for metadata and a disclosure affordance"
  );
  assert.match(
    mainStyleSource,
    /\.tool-card\s*{[^}]*position: relative[^}]*overflow: hidden/s,
    "tool cards should keep stable card geometry for the React island"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-card:{1,2}before\s*{[^}]*content:\s*""|\.tool-tone-file:{1,2}before|\.tool-tone-risk:{1,2}before/s,
    "tool cards should not carry the retired decorative operation band CSS"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-icon\b/,
    "webview stylesheet should not keep the retired generic tool icon selector"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-tone-(?:file|change|query|terminal|risk) \.summary-icon\s*{[^}]*border-color:/s,
    "tool cards should not keep stale pre-shadcn operation-tone icon color blocks"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary \.tool-summary-icon \[data-icon="inline-start"\]\s*{[^}]*width: 14px[^}]*height: 14px/s,
    "tool summary icons should use shadcn-style data-icon hooks instead of retired manual icon classes"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-summary \.tool-summary-icon \.icon\s*{/s,
    "tool summary icons should not depend on retired .icon styling"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.summary-icon \.icon\s*{/s,
    "shared summary icons should size SVGs without depending on retired .icon styling"
  );
  assert.match(
    mainStyleSource,
    /\.tool-card\s*{[^}]*border-color:\s*transparent[^}]*background:\s*transparent[^}]*overflow:\s*visible/s,
    "tool calls should render borderless outer surfaces that prioritize content over chrome"
  );
  assert.match(
    mainStyleSource,
    /\.turn-card\.tool:hover > \.tool-card,[\s\S]*\.turn-card\.tool:focus-within > \.tool-card,[\s\S]*\.turn-card\.tool:target > \.tool-card\s*{[^}]*border-color:\s*transparent/s,
    "tool call hover and focus states should not reintroduce card borders"
  );
  assert.match(
    mainStyleSource,
    /\.tool-card \.inline-actions button:{1,2}before\s*{[^}]*content:\s*none[^}]*display:\s*none/s,
    "borderless tool calls should suppress nested inline action meters from helper-rendered content"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-status:not\(\[data-slot="badge"\]\)|\.tool-kind:not\(\[data-slot="badge"\]\)|\.tool-risk-badge:not\(\[data-slot="badge"\]\)/,
    "tool metadata CSS should not keep non-shadcn badge fallback selectors after helper statuses are scoped"
  );
  assert.match(
    mainStyleSource,
    /\.tool-status\[data-slot="badge"\],[\s\S]*\.tool-kind\[data-slot="badge"\],[\s\S]*\.tool-risk-badge\[data-slot="badge"\]\s*{[^}]*min-width: 0[^}]*max-width: 100%[^}]*vertical-align: middle/s,
    "React tool metadata badges should let shadcn Badge own pill chrome while local CSS only bounds layout"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary-meta\s*{[^}]*gap: 4px[^}]*max-width: min\(92px, 26vw\)/s,
    "tool summary metadata should reserve only icon-width space"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary-meta \.tool-meta-icon-badge\[data-slot="badge"\]\s*{[^}]*width: 20px[^}]*height: 20px[^}]*padding: 0/s,
    "tool summary metadata badges should render as compact icon-only targets"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary-meta \.tool-meta-icon-badge \[data-icon="inline-start"\]\s*{[^}]*width: 12px[^}]*height: 12px/s,
    "tool summary metadata icons should stay visually quiet and consistent"
  );
  assert.match(
    mainStyleSource,
    /\.tool-kind-file\s*{[^}]*border-color: var\(--state-provider-border\)[^}]*color: var\(--trail-provider\)/s,
    "read tool kind chips should use the provider/file tone"
  );
  assert.match(
    mainStyleSource,
    /\.tool-kind-change\s*{[^}]*border-color: var\(--state-lane-border\)[^}]*color: var\(--trail-lane\)/s,
    "change tool kind chips should use the lane tone"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary-meta \.tool-status,[\s\S]*\.tool-summary-meta \.tool-risk-badge\s*{[^}]*flex: 0 0 20px/s,
    "tool summary metadata chips should reserve only icon space for the tool title"
  );
  assert.match(
    toolCallCardSourceTs,
    /<ChevronDown[\s\S]*className=\{cn\([\s\S]*tool-disclosure-icon[\s\S]*open \? "rotate-180"/,
    "tool summaries should expose a React-owned disclosure chevron from the shadcn collapsible state"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-summary:{1,2}after|\.tool-card\[data-open\] > \.tool-summary:{1,2}after/s,
    "tool summaries should not rely on retired CSS pseudo chevrons"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary,\s*\.timeline-group-summary,\s*\.payload-summary,\s*\.raw-summary\s*{[^}]*border-radius:\s*var\(--radius-control\)[^}]*transition:[^}]*background-color (?:120ms|\.12s) ease-out[^}]*box-shadow (?:120ms|\.12s) ease-out/s,
    "collapsible inspector summaries should share polished radius and transition treatment"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary:hover,\s*\.payload-summary:hover,\s*\.raw-summary:hover\s*{[^}]*background:\s*color-mix\(in srgb, var\(--surface-hover\) 64%, transparent\)/s,
    "collapsible inspector summaries should expose consistent hover feedback"
  );
  assert.match(
    mainStyleSource,
    /\.tool-summary:focus-visible,\s*\.timeline-group-summary:focus-visible,\s*\.payload-summary:focus-visible,\s*\.raw-summary:focus-visible\s*{[^}]*outline:\s*0[^}]*box-shadow:\s*var\(--focus-ring\)/s,
    "collapsible inspector summaries should share a strong keyboard focus treatment"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /(^|[,{]\s*)summary(?::|,|\s*\{)|summary::-webkit-details-marker|\.card-body > summary|\.terminal-output summary/s,
    "webview stylesheet should not keep native summary/details fallback styling after shadcn accordion migration"
  );
  assert.doesNotMatch(
    webviewSource,
    /details\.tool-card|details\.raw|HTMLDetailsElement|const DRAWER_FOCUSABLE_SELECTOR =\s*\n\s*'[^']*summary/s,
    "webview helper actions should not keep native details/summary fallbacks after shadcn accordion migration"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-evidence-strip|\.tool-detail > \.tool-locations|\.tool-card-actions|\.tool-facts|\.tool-fact|\.tool-context-/s,
    "tool call CSS should not reserve chrome for details, action rails, facts, or location strips"
  );
  assert.doesNotMatch(
    toolCallCardSourceTs,
    /ToolActionBar|ToolEvidenceStrip|ToolFacts|ToolLocations|ToolLocationBreadcrumb|ToolContextMenu|RawDetails|rawDetails|inspectToolDetails/s,
    "tool call cards should not render details, action rails, facts, location strips, or raw payload accordions"
  );
  assert.doesNotMatch(
    toolCallCardSourceTs,
    /from "@\/webview\/components\/ui\/breadcrumb"|from "\.\/InlineActions"|from "@\/webview\/components\/ui\/context-menu"|from "@\/webview\/components\/ui\/separator"/,
    "tool call cards should not import UI primitives solely for removed details chrome"
  );
  assert.doesNotMatch(
    toolCallCardSourceTs,
    /className="chips/,
    "tool card island should not emit the generic legacy chips wrapper"
  );
  assert.match(
    mainStyleSource,
    /\.tool-meta-hover-trigger\s*{[^}]*display: inline-flex[^}]*min-width: 0[^}]*max-width: 100%[^}]*\}[\s\S]*\.tool-meta-hover-card\s*{[^}]*max-width: min\(280px, calc\(100vw - 24px\)\)/s,
    "tool summary hover-card affordances should stay compact and viewport bounded"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-action(?:-bar)?(?:\s|\.|:|\[|,)|\.tool-action:{1,2}before\s*{[^}]*content:\s*""/s,
    "tool action buttons should rely on shared inline actions instead of retired tool-action selectors"
  );
  assert.match(
    toolCallCardSourceTs,
    /<Icon data-icon="inline-start" aria-hidden="true" \/>/,
    "tool summary and metadata icons should pass through shadcn's data-icon contract"
  );
  assert.doesNotMatch(
    toolCallCardSourceTs,
    /<Icon aria-hidden="true" className="icon" \/>/,
    "tool summary icons should not carry retired manual icon sizing classes"
  );
  assert.doesNotMatch(
    toolCallCardSourceTs,
    /<Icon data-icon="inline-start" className="icon"/,
    "tool action icons should not carry retired manual icon sizing classes"
  );
  assert.doesNotMatch(
    toolCallCardSourceTs,
    /tool-action-\$\{action\.tone\}|tool-action-primary|tool-action-danger/,
    "tool action buttons should not emit retired tone classes"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-action:hover|\.tool-action:active|\.tool-action-primary|\.tool-action-danger|\.tool-action-bar/s,
    "tool action buttons should rely on shadcn button variants for hover, active, and tone chrome"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-facts (?:dt|dd|div)\b|\.tool-fact\s*{[^}]*display:\s*grid|\.tool-fact\s*{[^}]*padding-block:/s,
    "tool facts should not keep retired definition-list mini-card styling"
  );
  assert.match(
    mainStyleSource,
    /\.diffs-mount\s*{[^}]*max-height: min\(460px, 52vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "enhanced Diffs.com mount should keep viewport-aware contained scrolling"
  );
  assert.match(
    mainStyleSource,
    /\.diffs-mount:focus-within\s*{[^}]*box-shadow: inset 0 0 0 1px var\(--input-active-border\)/s,
    "enhanced Diffs.com mount should expose focus when nested controls receive keyboard focus"
  );
  assert.match(
    mainSource,
    /message\.type === "diff"[\s\S]*openDiffReviewDrawer\(message\.result\)/,
    "task-level Open diff should render a structured review drawer instead of raw JSON"
  );
  assert.match(
    mainSource,
    /function getDiffReviewDrawerModule\(\)[\s\S]*import\("\.\/diffReviewDrawer\.js"\)/,
    "diff review parsing and drawer markup should stay behind an on-demand chunk"
  );
  assert.doesNotMatch(
    mainSource,
    /function diffReviewDrawerContent|function splitPatchFiles/,
    "main startup bundle should not inline diff review parser and drawer markup"
  );
  assert.match(
    diffReviewSourceTs,
    /export function renderDiffReviewDrawer\(result:\s*unknown,\s*renderHelpers:[\s\S]*html: diffReviewDrawerContent\(review\)[\s\S]*firstPath: review\.files\[0\]\?\.path \|\| ""/,
    "diff review chunk should expose rendered drawer content and the initial file selection"
  );
  assert.match(
    webviewSource,
    /renderDiffReviewDrawer\(result,[\s\S]*inlineActions: \(\{ actions, ariaLabel, className \}\) =>[\s\S]*actions: actions\.map\(\(\{ icon, \.\.\.action \}\) => \(\{[\s\S]*iconHtml: iconSvg\(icon as IconName\)/,
    "diff review drawer should register shadcn inline action props from the extension-hosted helper"
  );
  assert.match(
    webviewSource,
    /mountJsonDrawer\(drawer\)[\s\S]*selectDiffReviewFile\(rendered\.firstPath\)[\s\S]*hydratePayloadDisclosures\(\)[\s\S]*\.then\(\(\) => hydrateInlineActions\(\)\)[\s\S]*querySelector<HTMLElement>\("\[data-action='closeDrawer'\]"\)\?\.focus\(\)[\s\S]*hydrateDiffPreviews\(\+\+diffRenderEpoch\)\.then\(\(\) => hydrateInlineActions\(\)\)/,
    "diff review drawer should hydrate shadcn inline actions before focus and after diff previews replace loading markup"
  );
  assert.match(
    diffReviewSourceTs,
    /function diffReviewDrawerContent\(review: DiffReviewModel[\s\S]*data-diff-review-tree[\s\S]*diffReviewSuggestionList\(review\.suggestions, host\)/,
    "diff review drawer should include a Trees.software file rail and suggestion panel"
  );
  assert.match(
    diffReviewSourceTs,
    /function diffReviewDrawerContent\(review: DiffReviewModel[\s\S]*diffReviewStatusLegend\(review\.files, host\)[\s\S]*function diffReviewStatusChip\(status: DiffReviewStatus, host: DiffReviewDrawerHelpers, count\?: number \| undefined\)[\s\S]*function diffReviewStatusLabel\(status: DiffReviewStatus\)/,
    "diff review drawer should render semantic changed-file status chips from the lazy chunk"
  );
  assert.match(
    diffReviewSourceTs,
    /function splitPatchFiles\(patch: string\)[\s\S]*matchAll\(\/\^diff --git[\s\S]*patchLineStats\(section\)/,
    "diff review drawer should split patch-only responses into file sections"
  );
  assert.match(
    mainSource,
    /function renderPatchDiffPreview\([\s\S]*preview\.compact\s*\? "" : diffPreviewToolbar[\s\S]*preview\.compact\s*\? "" : `<div class="diff-preview-meta">[\s\S]*data-diffs-mode="patch"[\s\S]*data-diffs-compact="\$\{preview\.compact \? "true" : "false"\}"[\s\S]*template class="diff-patch-source"/,
    "patch-only files should mount Diffs.com with raw patch data"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-drawer\s*{[^}]*width: min\(1080px, calc\(100vw - 32px\)\)/s,
    "diff review drawer should use a wide review workspace instead of the narrow raw JSON drawer"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-layout\s*{[^}]*grid-template-columns: minmax\(190px, 0?\.26fr\) minmax\(0, 1fr\) minmax\(190px, 0?\.28fr\)/s,
    "diff review workspace should allocate file tree, diff, and action columns"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-header-actions\.inline-actions,\s*\.diff-review-suggestion-actions\.inline-actions\s*{[^}]*display: inline-flex[^}]*justify-content: flex-end[^}]*max-width: min\(156px, 42%\)[^}]*border-color: transparent[^}]*background: transparent/s,
    "diff review shadcn action rails should stay compact instead of inheriting framed helper chrome"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-header-actions\.inline-actions button::before,\s*\.diff-review-suggestion-actions\.inline-actions button::before\s*{[^}]*content: none[^}]*display: none/s,
    "diff review shadcn icon buttons should suppress helper action meter pseudo-elements"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-file-tree\s*{[^}]*max-height: min\(520px, calc\(100vh - 252px\)\)[^}]*overflow: auto[^}]*scrollbar-gutter: stable/s,
    "Trees.software file rail should stay bounded inside the drawer"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-status-legend\s*{[^}]*display: flex[^}]*flex-wrap: wrap[^}]*max-height: min\(64px, 12vh\)[^}]*overflow: auto[^}]*scrollbar-gutter: stable/s,
    "diff review status legend should stay compact and scroll-stable with many change types"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-status-chip\s*{[^}]*position: relative[^}]*display: inline-flex[^}]*border: 1px solid var\(--border-subtle\)[\s\S]*\.diff-review-status-chip:{1,2}before\s*{[^}]*inset-inline-start: 3px[^}]*width: 2px[^}]*background: var\(--border-subtle\)/s,
    "diff review status chips should expose compact type meters without extra markup"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-status-added\s*{[^}]*border-color: color-mix\(in srgb, var\(--state-success-border\) 54%, var\(--border-subtle\)\)[\s\S]*\.diff-review-status-deleted\s*{[^}]*border-color: color-mix\(in srgb, var\(--state-danger-border\) 58%, var\(--border-subtle\)\)[\s\S]*\.diff-review-status-renamed\s*{[^}]*border-color: color-mix\(in srgb, var\(--state-provider-border\) 52%, var\(--border-subtle\)\)[\s\S]*\.diff-review-status-untracked\s*{[^}]*border-color: color-mix\(in srgb, var\(--state-warning-border\) 54%, var\(--border-subtle\)\)/s,
    "diff review status chips should distinguish added, deleted, renamed, and untracked files"
  );
  assert.match(
    mainStyleSource,
    /\.diff-review-file-main\s*{[^}]*display: grid[^}]*gap: 3px[^}]*min-width: 0[\s\S]*button\.diff-review-file-button \.diff-review-file-label,[\s\S]*button\.diff-review-file-button small\s*{[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "diff review fallback rows should reserve status space while truncating long file names"
  );
  assert.match(
    mainSource,
    /type === "diff"[\s\S]*(?:const|let) path = String\(record\.path \|\| primaryToolPath\(tool\) \|\| "Tool diff"\)[\s\S]*return diffPreview\(\{[\s\S]*path,[\s\S]*compact: effectiveKind === "edit"/,
    "tool diff previews should preserve wrapped provider file paths before using generic labels"
  );
  assert.match(
    mainSource,
    /interface PendingDiffPreview[\s\S]*compact\?: boolean \| undefined[\s\S]*diff-preview-loading\$\{compact \? " diff-preview-compact" : ""\}[\s\S]*preview\.compact \? "" : diffPreviewToolbar[\s\S]*data-diffs-compact="\$\{preview\.compact \? "true" : "false"\}"/s,
    "edit tool diff previews should support a compact mode without path, metadata, or toolbar chrome"
  );
  assert.match(
    mainStyleSource,
    /\.tool-card \.diff-preview-compact \.diff-fallback\s*{[^}]*margin-top: 0[\s\S]*\.tool-card \.diff-preview-compact \.diff-compact-code\s*{[^}]*max-height: min\(520px, 55vh\)[^}]*overflow: auto/s,
    "compact edit patch fallbacks should avoid code toolbar chrome while staying scrollable"
  );
  assert.match(
    diffEnhancerSourceTs,
    /disableFileHeader: true/,
    "enhanced Diffs.com previews should not add a second file header above compact edit diffs"
  );
  assert.match(
    mainStyleSource,
    /\.diffs-mount\[data-diffs-state="?loading"?\]\s*{[^}]*min-height: 84px/s,
    "enhanced Diffs.com mount should reserve a stable loading height"
  );
  assert.match(
    mainStyleSource,
    /\.diff-grid\s*{[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "structured diff fallback should keep nested review scrolling stable"
  );
  assert.match(
    mainStyleSource,
    /\.diff-stat\s*{[^}]*display:\s*inline-flex[^}]*font-variant-numeric:\s*tabular-nums/s,
    "structured diff stat chips should stay compact with stable numeric alignment"
  );
  assert.match(
    mainStyleSource,
    /\.diff-preview-actions\.inline-actions\s*{[^}]*display: inline-flex[^}]*flex: 0 1 auto[^}]*justify-content: flex-end[^}]*max-width: min\(156px, 42%\)[^}]*max-height: min\(72px, 18vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "diff preview action clusters should stay bounded without stealing title space"
  );
  assert.match(
    mainStyleSource,
    /\.diff-preview-actions\.inline-actions button\[data-inline-icon-only="true"\]\s*{[^}]*flex: 0 0 auto/s,
    "diff preview icon actions should target the shadcn data hook instead of retired icon-button classes"
  );
  assert.match(
    webviewSource,
    /function diffPreviewToolbar\([\s\S]*const actions = inlineActions\(\{[\s\S]*className: "diff-preview-actions"[\s\S]*action: "openNodeDiff"[\s\S]*action: "copyDiff"[\s\S]*action: "openDiffPreview"[\s\S]*\$\{actions\}/,
    "diff preview toolbar actions should render through the shadcn inline actions island"
  );
  assert.doesNotMatch(
    webviewSource,
    /function diffPreviewToolbar\([\s\S]*iconButton\("openNodeDiff"[\s\S]*iconButton\("copyDiff"[\s\S]*iconButton\("openDiffPreview"/,
    "diff preview toolbar should not use retired raw icon buttons"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.diff-preview-toolbar\s*{[^}]*flex-direction: column[\s\S]*\.diff-preview-actions\s*{[^}]*justify-content: flex-start[^}]*max-width: 100%/s,
    "stacked diff preview toolbars should let actions use the full narrow pane width"
  );
  assert.match(
    mainStyleSource,
    /\.diff-preview-meta span\s*{[^}]*overflow:\s*hidden[^}]*text-overflow:\s*ellipsis[^}]*white-space:\s*nowrap/s,
    "structured diff metadata chips should truncate instead of overflowing"
  );
  assert.match(
    mainStyleSource,
    /\.diff-line-number\s*{[^}]*font-variant-numeric:\s*tabular-nums/s,
    "structured diff line numbers should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.diff-code-old\s*{[^}]*border-inline-end:\s*1px solid/s,
    "structured diff fallback should keep a visible before/after column divider"
  );
  assert.match(
    mainStyleSource,
    /\.diff-row:not\(\.diff-row-header\):not\(\.diff-row-gap\):hover\s*{[^}]*background:\s*color-mix/s,
    "structured diff fallback should expose a subtle row-hover orientation cue"
  );
  assert.match(
    mainStyleSource,
    /\.terminal-transcript\s*{[^}]*display:\s*grid[^}]*overflow:\s*hidden[^}]*border: 1px solid color-mix\(in srgb, var\(--terminal-stream-color\) 28%, var\(--border-subtle\)\)/s,
    "terminal tools should render as a single compact transcript surface"
  );
  assert.match(
    mainStyleSource,
    /\.terminal-transcript-row\s*{[^}]*grid-template-columns:\s*44px minmax\(0, 1fr\)[^}]*border-top:\s*1px solid var\(--border-subtle\)/s,
    "terminal transcript rows should reserve a stable IN/OUT/ERR label gutter"
  );
  assert.match(
    mainStyleSource,
    /\.terminal-transcript-code\s*{[^}]*max-height:\s*min\(440px, 58vh\)[^}]*font-family: var\(--vscode-editor-font-family[^}]*white-space:\s*pre-wrap/s,
    "terminal transcript code should stay bounded while preserving CLI whitespace"
  );
  assert.match(
    mainSource,
    /function terminalPreviewFromModel\(model, nodeId\)[\s\S]*terminalTranscriptRow\("in", "IN", command, \{ language: "shellscript"[\s\S]*terminalTranscriptRow\(section\.id === "stderr" \? "err" : "out"/,
    "terminal previews should render shell input and output as explicit transcript rows"
  );
  assert.match(
    webviewSource,
    /function terminalToolData\(\s*tool:\s*ToolNodeView,\s*presentation:\s*ToolPresentation\s*\)[\s\S]*const texts = terminalContentTexts\(tool\.content\)[\s\S]*const parsedTexts = texts\.map\(terminalCommandFromText\)[\s\S]*terminalCommand\(rawInput\)[\s\S]*terminalCommand\(rawOutput\)[\s\S]*textIntent[\s\S]*contentOutput: outputParts\.join\(/,
    "terminal tool previews should prefer real raw commands and parse generic content text without duplication"
  );
  assert.match(
    webviewSource,
    /function terminalBlockPreviewInput\([\s\S]*base:\s*Record<string,\s*unknown>[\s\S]*block:\s*Record<string,\s*unknown>[\s\S]*data:\s*Pick<ReturnType<typeof terminalToolData>,\s*"command"\s*\|\s*"contentOutput">[\s\S]*!command \|\| isCountSummary\(command\)[\s\S]*input\.command = data\.command[\s\S]*!terminalHasOutput\(input\)[\s\S]*input\.output = data\.contentOutput/,
    "terminal block previews should not let count summaries replace the shell input or hide text output fallbacks"
  );
  assert.match(
    mainSource,
    /function terminalCommandFromText\(text\)[\s\S]*terminalFenceBlock\(text\)[\s\S]*terminalCommandFromLines\(fenced\.body\.split\([\s\S]*looksLikeShellCommand\(first\)/,
    "terminal text fallbacks should split shell-looking and fenced console content into command and output"
  );
  assert.match(
    mainSource,
    /function looksLikeTerminalOutput\(lines\)[\s\S]*lines\.length > 2[\s\S]*https\?:\\\/\\\//,
    "terminal text fallbacks should keep plain descriptions out of output rows"
  );
  assert.match(
    webviewSource,
    /const terminal = model\.kind === "execute"[\s\S]*const contentHtml = terminal[\s\S]*terminalToolPreview\(node, model\)[\s\S]*contentHtml,[\s\S]*approval/s,
    "tool calls should send rendered content to the React card without generic raw-details props"
  );
  assert.doesNotMatch(
    webviewSource,
    /const rawDetails = !terminal[\s\S]*rawDetails,/s,
    "tool call props should not include generic raw-details rendering"
  );
  assert.match(
    webviewSource,
    /function terminalPreviewFromModel\([\s\S]*const openTerminal = nodeId[\s\S]*inlineActions\(\{[\s\S]*className: "terminal-transcript-actions"[\s\S]*action: "openTerminal"[\s\S]*data: \{ "node-id": nodeId \}[\s\S]*iconHtml: iconSvg\("terminal"\)[\s\S]*\$\{openTerminal\}/,
    "terminal tool preview actions should render through the shadcn inline action island"
  );
  assert.doesNotMatch(
    webviewSource,
    /function terminalPreviewFromModel\([\s\S]*iconButton\("openTerminal"/,
    "terminal tool preview should not use retired raw icon buttons"
  );
  assert.match(
    mainSource,
    /title = terminal \? "Bash" : model\.title[\s\S]*subtitle = terminal \? terminalToolIntent\(node, model\)/s,
    "collapsed terminal tools should summarize intent as Bash plus a provider or shell-comment subtitle"
  );
  assert.match(
    mainSource,
    /function terminalCommandParts\(value\)[\s\S]*shellCommentText\(lines\[0\]\)[\s\S]*intentLines\.push\(shellCommentText\(lines\.shift\(\)\)[\s\S]*value\.trim\(\)/s,
    "terminal commands should lift leading shell comments into the collapsed intent without losing the executable input"
  );
  assert.match(
    mainStyleSource,
    /\.terminal-transcript-out\s*{[^}]*--terminal-stream-color: var\(--trail-checkpoint\)/s,
    "stdout rows should use a success-colored stream accent"
  );
  assert.match(
    mainStyleSource,
    /\.terminal-transcript-err\s*{[^}]*--terminal-stream-color: var\(--trail-risk\)/s,
    "stderr rows should use a risk-colored stream accent"
  );
  assert.match(
    mainSource,
    /function renderAnsiText\(value\)[\s\S]*ansi-fg-\$\{color\}[\s\S]*function ansiColor\(value\)/s,
    "terminal output should convert ANSI foreground codes into safe color spans"
  );
  assert.match(
    mainStyleSource,
    /\.ansi-fg-blue\s*{[^}]*color:\s*var\(--syntax-blue\)/s,
    "terminal ANSI colors should map to the local syntax palette"
  );
  assert.match(
    mainStyleSource,
    /\.terminal-transcript-code\.highlighted\s*{[^}]*padding:\s*9px 12px[^}]*white-space:\s*pre-wrap/s,
    "highlighted terminal rows should keep transcript padding and wrapping"
  );
  assert.match(
    mainStyleSource,
    /\.terminal-transcript-code \.code-line:{1,2}before\s*{[^}]*content:\s*none[^}]*display:\s*none/s,
    "terminal transcript highlighting should suppress generic code line numbers"
  );
  assert.match(
    mainStyleSource,
    /\.tool-card \.terminal-transcript\s*{[^}]*gap:\s*3px[^}]*border-color:\s*transparent[^}]*background:\s*transparent[^}]*overflow:\s*visible/s,
    "terminal tool transcripts should keep IN/OUT content complete without drawing a framed component box"
  );
  assert.match(
    mainStyleSource,
    /\.tool-card \.code-frame,[\s\S]*\.tool-card \.file-preview \.code-frame\s*{[^}]*border-color:\s*transparent[^}]*background:\s*transparent[^}]*overflow:\s*visible[\s\S]*\.tool-card :is\(\.resource, \.media-preview, \.unsupported, \.raw\)\s*{[^}]*border-color:\s*transparent[^}]*background:\s*transparent[^}]*overflow:\s*visible/s,
    "tool call code previews and raw details should not reintroduce nested boxed panels"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.terminal-transcript-row\s*{[^}]*grid-template-columns:\s*36px minmax\(0, 1fr\)[\s\S]*\.terminal-transcript-code\s*{[^}]*max-height:\s*min\(360px, 52vh\)/s,
    "terminal transcripts should keep their IN/OUT gutter usable in narrow panes"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-filter-popover\s*{[^}]*max-height: min\(440px, calc\(100vh - 88px\)\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "timeline filter popover should stay bounded for long transcript sessions"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-filter-trigger\s*{[^}]*grid-template-columns: auto auto auto[^}]*justify-content: center[^}]*max-width: 100%/s,
    "timeline filter toolbar trigger should keep icon count and disclosure bounded"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-filter-popover\s*{[^}]*width: min\(390px, calc\(100vw - 28px\)\)[^}]*max-height: min\(440px, calc\(100vh - 88px\)\)/s,
    "timeline filter popover should stay inside narrow webview panes"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-filter-group\s*{[^}]*display: grid[^}]*grid-template-columns: repeat\(2, minmax\(0, 1fr\)\)[^}]*gap: 6px/s,
    "timeline filter choices should render as a compact bounded grid"
  );
  assert.match(
    mainStyleSource,
    /button\.timeline-filter\s*{[^}]*min-width: 0[^}]*max-width: 100%[^}]*overflow: hidden/s,
    "timeline filter buttons should shrink without widening the toolbar"
  );
  assert.match(
    mainStyleSource,
    /button\.timeline-filter span\s*{[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "timeline filter labels should truncate under long localized names"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-search:focus-within\s*{[^}]*border-color: var\(--input-active-border\)[^}]*box-shadow: var\(--focus-ring\)/s,
    "timeline transcript search should keep a visible focus treatment"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-search \[data-icon="inline-start"\],[\s\S]*\.timeline-search \[data-icon="inline-start"\] > svg\s*{[^}]*width:\s*14px[^}]*height:\s*14px/s,
    "timeline search icon should size shadcn data-icon hooks instead of retired .icon wrappers"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-shell\s*{[^}]*grid-template-rows:\s*minmax\(0, 1fr\)/s,
    "timeline shell should give the transcript the full main view after moving filters to the toolbar"
  );
  assert.match(
    mainStyleSource,
    /\.lane-map-drawer\[data-slot="drawer-content"\]\s*{[^}]*width: min\(430px, calc\(100vw - 18px\)\)[^}]*max-width: min\(430px, calc\(100vw - 18px\)\)/s,
    "lane map drawer should keep a compact right-side width"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-group-summary\s*{[^}]*grid-template-columns: auto minmax\(0, 1fr\) auto[^}]*background: color-mix\(in srgb, var\(--surface-muted\) 18%, var\(--surface\)\)[^}]*padding: 6px 62px 6px 8px/s,
    "timeline group summaries should render as light rows with reserved room for copy and disclosure controls"
  );
  assert.match(
    mainStyleSource,
    /\.lane-map-body\s*{[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "lane map drawer body should stay scroll-stable during long transcript runs"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-group-disclosure\s*{[^}]*inset-inline-end: 8px[^}]*pointer-events: none[\s\S]*\.timeline-group-disclosure \[data-slot="accordion-trigger-icon"\]\s*{[^}]*width: 14px[^}]*height: 14px[^}]*color: var\(--trail-muted\)/s,
    "timeline group summaries should use a quiet icon-only disclosure affordance"
  );
  assert.match(
    mainStyleSource,
    /\.header-action-group button\[data-header-icon-only="true"\]\[data-lane-map-trigger="true"\]\.active\s*{[^}]*border-color: var\(--state-lane-border\)[^}]*background:\s*color-mix\(in srgb, var\(--trail-lane\) 10%, transparent\)/s,
    "open lane map trigger should keep a clear active state in the toolbar"
  );
  assert.doesNotMatch(
    timelineNavigationSourceTs,
    /<details className="session-map"|<summary className="session-map-summary"/,
    "session map should not keep native details markup after the shadcn accordion migration"
  );
  assert.doesNotMatch(
    webviewSource,
    /<details id="\$\{timelineGroupDomId\(group\)\}"|<summary class="timeline-group-summary"/,
    "timeline groups should render through the lazy shadcn accordion island"
  );
  assert.match(
    mainStyleSource,
    /\.lane-map-section-heading\s*{[^}]*justify-content: space-between[^}]*font-size: 11px[^}]*font-weight: 650/s,
    "lane map drawer section headings should stay compact and scannable"
  );
  assert.match(
    mainStyleSource,
    /\.lane-map-drawer \.event-chip \[data-icon="inline-start"\],[\s\S]*\.lane-map-drawer \.event-chip \[data-icon="inline-start"\] > svg\s*{[^}]*width: 12px[^}]*height: 12px/s,
    "lane map chip icons should style shadcn Badge data-icon hooks"
  );
  assert.match(
    mainStyleSource,
    /\.tool-activity-metric b\s*{[^}]*font-variant-numeric:\s*tabular-nums/s,
    "session map metric values should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.tool-activity-metrics\s*{[^}]*display: grid[^}]*grid-template-columns: repeat\(auto-fit, minmax\(min\(118px, 100%\), 1fr\)\)[^}]*align-items: stretch/s,
    "session map metrics should render as a resilient dashboard grid"
  );
  assert.match(
    mainStyleSource,
    /\.tool-activity-metric\s*{[^}]*grid-template-columns: auto minmax\(0, 1fr\)[^}]*border: 1px solid var\(--border-subtle\)[^}]*border-radius: var\(--radius-control\)/s,
    "session map metric cards should stay framed without left accent rails"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-activity-metric\s*{[^}]*border-inline-start/s,
    "session map metric cards should not use left accent rails"
  );
  assert.match(
    mainStyleSource,
    /\.tool-activity-paths\s*{[^}]*max-height: min\(112px, 24vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "session map changed-path chips should stay bounded inside activity summaries"
  );
  assert.match(
    mainStyleSource,
    /\.tool-activity-paths\s*{[^}]*display: grid[^}]*grid-template-columns: repeat\(auto-fit, minmax\(min\(180px, 100%\), 1fr\)\)/s,
    "session map changed paths should wrap in a stable grid"
  );
  assert.match(
    mainStyleSource,
    /\.tool-activity-path\s*{[^}]*max-width: 100%[^}]*border: 1px solid var\(--border-subtle\)[^}]*border-radius: var\(--radius-control\)/s,
    "session map changed-path chips should stay framed without left accent rails"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.tool-activity-path\s*{[^}]*border-inline-start/s,
    "session map changed-path chips should not use left accent rails"
  );
  assert.match(
    mainStyleSource,
    /\.session-map-turns\s*{[^}]*max-height: min\(128px, 24vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "session map turn links should stay bounded and scroll-stable"
  );
  assert.match(
    mainStyleSource,
    /\.session-map-turn\s*{[^}]*color: var\(--text\)[^}]*text-decoration: none[^}]*transition:/s,
    "session map turn links should read as structured navigation cards"
  );
  assert.match(
    mainStyleSource,
    /\.session-map-turn:focus-visible\s*{[^}]*outline: 0[^}]*border-color: var\(--input-active-border\)[^}]*box-shadow: var\(--focus-ring\)/s,
    "session map turn links should keep a strong keyboard focus treatment"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-group-meta\s*{[^}]*font-variant-numeric:\s*tabular-nums/s,
    "timeline group metadata should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-group-copy-id\[data-slot="button"\]\s*{[^}]*inset-inline-end: 31px[^}]*width: 22px[^}]*height: 22px[^}]*background: transparent[\s\S]*\.timeline-group-copy-id \[data-icon="inline-start"\]\s*{[^}]*width: 12px[^}]*height: 12px/s,
    "timeline group copy-id affordance should replace visible raw ids with a compact icon target"
  );
  assert.match(
    mainSource,
    /name === "copyTimelineGroupId"[\s\S]*copyTimelineGroupId\(action\)[\s\S]*async function copyTimelineGroupId\(action\)[\s\S]*copyTextToClipboard\(action\.dataset\.target \|\| "", "turn id", "Copied turn ID\."\)/,
    "timeline group id copy should use the shared clipboard feedback path"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.timeline-group-summary\s*{[^}]*grid-template-columns: auto minmax\(0, 1fr\) auto[\s\S]*\.timeline-group-meta\s*{[^}]*grid-column: 2 \/ 3/s,
    "timeline group metadata should wrap under the title without colliding with the chevron in narrow panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 900px\)\s*{[\s\S]*\.tool-activity-metrics,\s*\.tool-activity-paths,\s*\.session-map-turns\s*{[^}]*grid-template-columns: minmax\(0, 1fr\)/s,
    "session map activity grids should stack cleanly in responsive panes"
  );
  assert.match(
    mainStyleSource,
    /\.timeline-group\s*{[^}]*scroll-margin-block-start:\s*72px/s,
    "timeline groups should leave room for sticky chrome when linked from the session map"
  );
  assert.match(
    mainStyleSource,
    /\.timeline\s*{[^}]*scroll-behavior:\s*auto/s,
    "timeline scrolling should stay immediate so streaming scrollTop restores do not queue smooth animations"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.timeline\s*{[^}]*scroll-behavior:\s*smooth/s,
    "timeline scrolling should not animate token-driven scroll position updates"
  );
	  assert.match(
	    mainStyleSource,
	    /\.timeline \[data-slot="accordion-content"\],\s*\.timeline \[data-slot="collapsible-content"\]\s*{[^}]*animation:\s*none !important[^}]*transition:\s*none !important/s,
	    "timeline disclosure panels should not run open/close animations during streaming hydration"
	  );
	  assert.match(
	    mainStyleSource,
	    /\.timeline \[data-slot="accordion-content"\] > \*,\s*\.timeline \[data-slot="collapsible-content"\] > \*,\s*\.timeline \[data-starting-style\],\s*\.timeline \[data-ending-style\]\s*{[^}]*height:\s*auto !important[^}]*animation:\s*none !important[^}]*transition:\s*none !important/s,
	    "timeline disclosure measured-height styles should not collapse hydrated transcript content"
	  );
	  assert.match(
	    mainStyleSource,
	    /\.timeline-group \[data-slot="accordion-content"\]\s*{[^}]*height:\s*auto[^}]*overflow:\s*visible/s,
	    "expanded timeline group panels should not keep a stale measured accordion height before nested islands hydrate"
	  );
  assert.match(
    mainStyleSource,
    /\.timeline-group-body\s*{[^}]*height:\s*auto[^}]*max-height:\s*none[^}]*overflow:\s*visible/s,
    "timeline group bodies should use natural height after lazy message and tool cards mount"
  );
  assert.match(
    mainStyleSource,
    /\.turn-card\s*{[^}]*grid-template-columns: minmax\(0, 1fr\)[^}]*scroll-margin-block-start:\s*72px/s,
    "transcript cards should leave room for sticky chrome without a rail column"
  );
  assert.match(
    mainStyleSource,
    /\.rail\s*{[^}]*display:\s*none/s,
    "transcript cards should not render a left rail column"
  );
  assert.match(
    mainStyleSource,
    /\.card-body\s*{[^}]*border: 1px solid var\(--border-subtle\)[^}]*border-radius: var\(--radius-card\)/s,
    "transcript cards should use a single quiet frame"
  );
  assert.match(
    mainStyleSource,
    /\.tool-call-group \.tool-group-card\.card-body\s*{[^}]*border-color: transparent[^}]*background: transparent[^}]*box-shadow: none[^}]*overflow: visible[^}]*\}[\s\S]*\.tool-call-group \.tool-group-summary\s*{[^}]*min-height: 36px[^}]*border: 1px solid color-mix\(in srgb, var\(--border-subtle\) 48%, transparent\)[^}]*background: color-mix\(in srgb, var\(--surface\) 94%, var\(--surface-raised\)\)/s,
    "grouped tool-call surfaces should use the same light summary treatment as individual tool calls"
  );
  assert.match(
    mainStyleSource,
    /\.tool-call-group \.tool-group-card\.card-body\.is-open > \.tool-group-summary,[\s\S]*\.tool-call-group \.tool-group-card\.card-body\[data-open\] > \.tool-group-summary\s*{[^}]*border-end-start-radius: 0[^}]*background: color-mix\(in srgb, var\(--surface\) 92%, var\(--surface-raised\)\)[^}]*\}[\s\S]*\.tool-call-group \.tool-group-content\s*{[^}]*border: 1px solid color-mix\(in srgb, var\(--border-subtle\) 52%, transparent\)[^}]*background: color-mix\(in srgb, var\(--surface\) 96%, transparent\)/s,
    "expanded grouped tool calls should connect a light summary to a light content panel"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.card-body\s*{[^}]*border-inline-start-width/s,
    "transcript cards should not use left accent borders"
  );
  assert.match(
    mainStyleSource,
    /\.turn-card:focus-within > \.card-body,[\s\S]*\.turn-card:target > \.card-body\s*{[^}]*box-shadow:\s*var\(--focus-ring\)/s,
    "focused and targeted transcript cards should expose a strong orientation outline"
  );
  assert.match(
    mainStyleSource,
    /\.transcript-message-assistant \.transcript-message-content\s*{[^}]*width:\s*100%[^}]*max-width:\s*none[^}]*padding:\s*3px 0 4px/s,
    "assistant messages should render as plain full-width prose without avatar or card chrome"
  );
  assert.match(
    mainStyleSource,
    /\.transcript-message-assistant \.transcript-message-content > \.markdown\s*{[^}]*max-width:\s*100%[^}]*font-size:\s*var\(--trail-copy-font-size\)[^}]*line-height:\s*1\.62/s,
    "assistant markdown should use the full content width with more readable type"
  );
  assert.match(
    mainStyleSource,
    /\.transcript-message-user \.transcript-message-content\s*{[^}]*align-items:\s*flex-end/s,
    "user message content should align as a right-side transcript bubble"
  );
  assert.match(
    mainStyleSource,
    /\.message \.transcript-message-content > \.markdown\s*{[^}]*width:\s*100%/s,
    "message markdown should fill the bounded message column without visible role chrome"
  );
  assert.match(
    mainStyleSource,
    /\.message \.transcript-message-content > \.streaming-markdown\s*{[^}]*contain:\s*paint[^}]*overflow-wrap:\s*anywhere[^}]*white-space:\s*pre-wrap[^}]*word-break:\s*break-word/s,
    "streaming text should stay paint-contained and preserve newlines without reparsing Markdown"
  );
  assert.match(
    mainStyleSource,
    /:is\(\.resource, \.media-preview, \.unsupported, \.raw\)\s*{[^}]*display: grid[^}]*border: 1px solid var\(--border-subtle\)[^}]*overflow: hidden/s,
    "embedded payload details should render as compact framed evidence cards without left rails"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /:is\(\.resource, \.media-preview, \.unsupported, \.raw\)\s*{[^}]*border-inline-start/s,
    "embedded payload details should not use left accent rails"
  );
  assert.match(
    mainStyleSource,
    /\.payload-summary,\s*\.raw-summary\s*{[^}]*display: grid[^}]*grid-template-columns: minmax\(0, 1fr\) auto/s,
    "embedded payload summaries should reserve room for disclosure"
  );
  assert.match(
    mainStyleSource,
    /\.raw-summary\s*{[^}]*display: grid[^}]*grid-template-columns: minmax\(0, 1fr\) auto/s,
    "React raw-detail summaries should reserve room for the shadcn disclosure icon"
  );
  assert.match(
    mainStyleSource,
    /\.payload-summary,\s*\.raw-summary\s*{[^}]*cursor: pointer[^}]*list-style: none/s,
    "embedded payload summaries should own their interactive disclosure affordance"
  );
  assert.match(
    mainStyleSource,
    /\.payload-summary:focus-visible,\s*\.raw-summary:focus-visible\s*{[^}]*outline: 1px solid var\(--input-active-border\)[^}]*box-shadow: var\(--focus-ring\)/s,
    "keyboard-focused embedded payload summaries should keep a visible focus state"
  );
  assert.match(
    mainStyleSource,
    /\.payload-summary \[data-slot="accordion-trigger-icon"\],\s*\.raw-summary \[data-slot="accordion-trigger-icon"\]\s*{[^}]*color: var\(--trail-muted\)/s,
    "React raw-detail summaries should use the shadcn accordion icon affordance"
  );
  assert.match(
    mainStyleSource,
    /\.resource-chip small\s*{[^}]*min-width: 0[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "resource chip metadata should truncate without widening transcript cards"
  );
  assert.match(
    mainStyleSource,
    /button\.resource-chip,\s*button\.chip-button\s*{[^}]*min-width:\s*0[^}]*max-width:\s*100%[^}]*overflow:\s*hidden[^}]*border-color:\s*var\(--border-subtle\)[^}]*transition:/s,
    "clickable resource chips should stay bounded and use polished state transitions"
  );
  assert.match(
    mainStyleSource,
    /button\.resource-chip:hover,\s*button\.chip-button:hover\s*{[^}]*border-color:\s*var\(--input-active-border\)[^}]*background:\s*var\(--surface-hover\)/s,
    "clickable resource chips should expose the same hover affordance as other dense controls"
  );
  assert.match(
    mainStyleSource,
    /:is\(\.resource, \.media-preview, \.unsupported, \.raw\) \.payload-panel \.muted\s*{[^}]*display: inline-flex[^}]*width: fit-content[^}]*max-width: 100%[^}]*border: 1px solid var\(--border-subtle\)[^}]*overflow-wrap: anywhere/s,
    "embedded payload metadata notes should render as bounded evidence chips"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.resource,[\s\S]*\.media-preview,[\s\S]*\.unsupported,[\s\S]*\.raw,[\s\S]*border-color: CanvasText/s,
    "embedded payload cards should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.chips span,[\s\S]*\.resource-chip,[\s\S]*\.attachment-chip,[\s\S]*border-color: CanvasText/s,
    "evidence chips should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.event-chip,[\s\S]*\.event-fact,[\s\S]*\.event-callout,[\s\S]*border-color: CanvasText/s,
    "event evidence chips should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.audit-event:{1,2}before,[\s\S]*\.event-callout:{1,2}before[\s\S]*background: CanvasText/s,
    "event severity bands and callout meters should remain visible in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.event-warning,[\s\S]*\.event-callout-warning,[\s\S]*border-style: dashed/s,
    "warning event surfaces should keep a non-color severity cue"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.event-risk,[\s\S]*\.event-callout-risk,[\s\S]*border-style: double/s,
    "risk event surfaces should keep a stronger non-color severity cue"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.code-frame,[\s\S]*\.code-language,[\s\S]*border-color: CanvasText/s,
    "framed code previews should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.tool-activity,[\s\S]*\.tool-activity-path,[\s\S]*\.session-map-turn,[\s\S]*border-color: CanvasText/s,
    "session map activity cards should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /\.thought-summary\s*{[^}]*grid-template-columns: auto minmax\(0, 1fr\) auto auto/s,
    "thought accordion summaries should reserve room for status and disclosure"
  );
  assert.match(
    mainStyleSource,
    /\.thought-card\s*{[^}]*gap:\s*0[^}]*padding:\s*0[^}]*overflow:\s*hidden/s,
    "thought cards should let the accordion summary fill the card edge-to-edge"
  );
  assert.match(
    mainStyleSource,
    /\.audit-event\s*{[^}]*position: relative[^}]*overflow: hidden[\s\S]*\.audit-event:{1,2}before\s*{[^}]*content: ""[^}]*inset-block-start: 0[^}]*inset-inline: 0[^}]*height: 2px[^}]*pointer-events: none/s,
    "audit events should expose compact severity bands without extra DOM"
  );
  assert.match(
    mainStyleSource,
    /\.audit-success:{1,2}before\s*{[^}]*var\(--state-success-border\)[\s\S]*\.audit-warning:{1,2}before\s*{[^}]*var\(--state-warning-border\)[\s\S]*\.audit-risk:{1,2}before\s*{[^}]*height: 3px[^}]*var\(--state-danger-border\)/s,
    "audit event bands should distinguish success, warning, and risk states"
  );
  assert.match(
    mainStyleSource,
    /\.thought-summary:hover\s*{[^}]*background:\s*color-mix\(in srgb, var\(--surface-hover\) 58%, transparent\)/s,
    "thought accordion summaries should expose a subtle edge-to-edge hover affordance"
  );
  assert.match(
    mainStyleSource,
    /\.thought-summary\[aria-expanded="true"\]\s*{[^}]*border-bottom:\s*1px solid var\(--border-subtle\)[^}]*background:\s*color-mix\(in srgb, var\(--surface-muted\) 34%, transparent\)/s,
    "open thought summaries should read as attached headers instead of floating rows"
  );
  assert.match(
    mainStyleSource,
    /\.thought-summary:focus-visible\s*{[^}]*outline:\s*0[^}]*box-shadow:\s*var\(--focus-ring\)/s,
    "thought accordion summaries should keep a strong keyboard focus treatment"
  );
  assert.match(
    mainStyleSource,
    /\.thought-summary\s*{[^}]*cursor: pointer[^}]*list-style: none/s,
    "thought accordion summaries should own their disclosure affordance"
  );
  assert.match(
    mainStyleSource,
    /\.thought-summary \[data-slot="accordion-trigger-icon"\]\s*{[^}]*grid-column: 4[^}]*color: var\(--trail-muted\)/s,
    "thought accordion summaries should use the shadcn trigger icon as the disclosure affordance"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /details\.event-card|event-summary:{1,2}after/s,
    "thought cards should not keep the retired details-event disclosure CSS"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.thought-summary\s*{[^}]*grid-template-columns: auto minmax\(0, 1fr\) auto[\s\S]*\.thought-summary > \.tool-status\s*{[^}]*grid-column: 2 \/ 3/s,
    "thought status labels should wrap under the title without colliding with disclosure in narrow panes"
  );
  assert.match(
    mainStyleSource,
    /\.event-summary > \.tool-status\s*{[^}]*justify-self:\s*end[^}]*max-width:\s*min\(132px, 32vw\)/s,
    "event summary status chips should stay bounded so titles keep priority"
  );
  assert.match(
    mainStyleSource,
    /\.thought-panel\s*{[^}]*padding:\s*9px 10px 10px/s,
    "expanded thought content should keep an even inset below the attached header"
  );
  assert.match(
    mainStyleSource,
    /\.event-facts\s*{[^}]*max-height:\s*min\(156px, 32vh\)[^}]*overflow:\s*auto[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "event fact grids should stay bounded during verbose provider reports"
  );
  assert.match(
    mainStyleSource,
    /\.event-fact\[data-slot="badge"\]\s*{[^}]*width:\s*100%[^}]*height:\s*auto[^}]*min-height:\s*42px[^}]*white-space:\s*normal/s,
    "event fact badges should override shadcn's single-line pill sizing so checkpoint and usage facts do not collapse"
  );
  assert.match(
    mainStyleSource,
    /\.event-chips\s*{[^}]*max-height:\s*min\(96px, 22vh\)[^}]*overflow:\s*auto[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "event chip rows should contain long command and config inventories"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.event-action-row\s*{/,
    "event action rows should use shared inline-actions styling instead of event-only CSS"
  );
  assert.match(
    mainStyleSource,
    /\.event-callout\s*{[^}]*position: relative[^}]*overflow: hidden[^}]*padding: 8px 10px 8px 13px[\s\S]*\.event-callout:{1,2}before\s*{[^}]*inset-inline-start: 6px[^}]*width: 3px[^}]*background: var\(--border-subtle\)/s,
    "event callouts should expose compact typed meters without extra markup"
  );
  assert.match(
    mainStyleSource,
    /\.event-callout-success:{1,2}before\s*{[^}]*background: var\(--trail-checkpoint\)[\s\S]*\.event-callout-warning:{1,2}before\s*{[^}]*background: var\(--trail-review\)[\s\S]*\.event-callout-risk:{1,2}before\s*{[^}]*background: var\(--trail-risk\)/s,
    "event callout meters should carry checkpoint, warning, and risk semantics"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.event-action(?:\s|\.|:|\[|,)/,
    "event action buttons should rely on shared inline-actions button styling"
  );
  assert.match(
    mainStyleSource,
    /\.event-chip\s*{[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*font-variant-numeric:\s*tabular-nums/s,
    "event chips should truncate long provider values without resizing cards"
  );
  assert.match(
    mainStyleSource,
    /\.event-meter span\s*{[^}]*text-overflow: ellipsis[^}]*font-variant-numeric:\s*tabular-nums/s,
    "event meter values should stay optically stable"
  );
  assert.match(
    mainSource,
    /data-raw-tool-kind/,
    "tool cards should retain the raw provider kind while rendering the inferred operation"
  );
  assert.match(
    toolCallCardSourceTs,
    /function ToolMetaIconBadge[\s\S]*aria-label=\{label\}[\s\S]*title=\{label\}[\s\S]*<Icon data-icon="inline-start" aria-hidden="true" \/>/,
    "collapsed tool summary metadata should expose labels through accessible icon badges"
  );
  assert.match(
    toolCallCardSourceTs,
    /icon=\{operationIcon\(model\)\}[\s\S]*icon=\{riskIcon\(model\.riskTone, model\.riskLabel\)\}[\s\S]*icon=\{statusIcon\(status, model\.statusLabel\)\}/,
    "collapsed tool summaries should render operation, risk, and status metadata as icons"
  );
  assert.doesNotMatch(
    toolCallCardSourceTs,
    /model\.operationLabel\s*<\/Badge>|riskShortLabel\(model\.riskLabel\)|model\.statusLabel\s*<\/Badge>/,
    "tool summary metadata badges should not render visible text labels"
  );
  assert.match(
    messageCardSourceTs,
    /<MessageGroup data-message-card="">[\s\S]*<Message[\s\S]*<MessageContent[\s\S]*"transcript-message-content"[\s\S]*<div[\s\S]*className="markdown"/,
    "message nodes should render through shadcn message layout while preserving helper-owned markdown selectors"
  );
  assert.doesNotMatch(
    messageCardSourceTs,
    /MessageAvatar|MessageHeader|message-avatar|message-header|message-role-marker|message-streaming-badge|Marker|Spinner/,
    "assistant messages should not reintroduce visible avatar, role, or streaming badge chrome"
  );
  assert.doesNotMatch(
    messageCardSourceTs,
    /MessageHeader className="card-chrome"|className="streaming"|className="role message-role-marker"/,
    "message cards should not keep retired generic card chrome, role styling, or streaming pill hooks"
  );
  assert.match(
    timelineScrollerSourceTs,
    /<MessageScrollerProvider[\s\S]*<MessageScroller[\s\S]*<MessageScrollerViewport[\s\S]*id="timeline"[\s\S]*className="timeline"[\s\S]*<MessageScrollerContent[\s\S]*<MessageScrollerItem/,
    "timeline content should render through the shadcn message scroller item model while preserving transcript focus and helper markup"
  );
  assert.match(
    webviewSource,
    /function timelineScrollerItems\(nodes: RenderNode\[\]\): TimelineScrollerItemView\[\][\s\S]*items\.push\(\.\.\.renderTimeline\(nodes\)\)/,
    "webview renderer should pass stable row models to the shadcn timeline scroller island"
  );
  assert.match(
    webviewSource,
    /function renderTimelineGroup\([\s\S]*scrollAnchor: Boolean\(group\.turnId\)/,
    "turn timeline groups should become shadcn message scroller anchors"
  );
  assert.match(
    webviewSource,
    /function planNode\([\s\S]*planCardProps\.set\(node\.id,[\s\S]*data-plan-card-root/,
    "plan nodes should render through the lazy shadcn plan card island"
  );
  assert.doesNotMatch(
    webviewSource,
    /<div class="card-chrome"><span class="role">Plan<\/span><\/div>[\s\S]*<ol class="plan-list">/,
    "plan nodes should not keep the retired inline card chrome"
  );
  assert.match(
    planCardSourceTs,
    /<ol className="plan-list" aria-label="Plan steps">[\s\S]*<PlanCardRow/,
    "plan card island should preserve ordered plan semantics and legacy list selectors"
  );
  assert.match(
    planCardSourceTs,
    /<CardHeader className="plan-card-header">[\s\S]*<CardTitle className="plan-card-title">[\s\S]*<CardAction className="plan-card-action">[\s\S]*<Badge className="plan-card-count" variant="outline">/,
    "plan card header should use scoped shadcn CardAction and Badge hooks instead of retired generic card chrome"
  );
  assert.doesNotMatch(
    planCardSourceTs,
    /card-chrome|className="role"/,
    "plan card island should not depend on retired generic card chrome or role styling"
  );
  assert.match(
    planCardSourceTs,
    /<Checkbox[\s\S]*className="plan-status-checkbox"[\s\S]*checked=\{planStatusChecked\(entry\.status\)\}[\s\S]*disabled/,
    "plan card rows should render read-only shadcn checkbox status markers"
  );
  assert.match(
    webviewSource,
    /function diffNode\([\s\S]*diffCardProps\.set\(node\.id,[\s\S]*diffPreview\([\s\S]*data-diff-card-root/,
    "diff nodes should render through the lazy shadcn diff card island while preserving diff preview helpers"
  );
  assert.match(
    `${webviewSource}\n${timelineModelSourceTs}`,
    /function renderTimelineGroupBodyItems\(nodes: RenderNode\[\]\): TimelineGroupCardProps\["bodyItems"\][\s\S]*isInlineToolDiffNode\(nodes, node\)[\s\S]*continue[\s\S]*export function isInlineToolDiffNode\(nodes: RenderNode\[\], node: RenderNode\): boolean \{[\s\S]*sameTimelineRenderScope\(candidate, node\)[\s\S]*candidate\.acpToolCallId[\s\S]*comparableDiffPath/,
    "timeline groups should skip duplicate diff cards through scoped provider tool id aliases"
  );
  assert.doesNotMatch(
    webviewSource,
    /<details class="card-body diff-card">[\s\S]*<summary class="tool-summary">/,
    "diff nodes should not keep the retired details-based card chrome"
  );
  assert.match(
    diffCardSourceTs,
    /<AccordionContent className="diff-panel" keepMounted>[\s\S]*dangerouslySetInnerHTML=\{\{ __html: props\.previewHtml \}\}/,
    "diff card island should keep helper-rendered diff previews mounted for the existing hydrator"
  );
  assert.match(
    emptyStateCardSourceTs,
    /<Empty[\s\S]*className=\{cn\("empty-state", `empty-state-\$\{props\.variant\}`\)\}[\s\S]*<EmptyHeader className="empty-state-copy">[\s\S]*<EmptyContent className="[^"]*\bempty-actions\b[^"]*"/,
    "empty transcript states should render through the shadcn empty component while preserving legacy selectors"
  );
  assert.match(
    mainStyleSource,
    /\.plan-card-react-root\s*{[^}]*display:\s*contents/s,
    "plan card island roots should not add layout wrappers around transcript cards"
  );
  assert.match(
    mainStyleSource,
    /\.plan-card\s*{[^}]*gap:\s*0[^}]*overflow:\s*hidden[^}]*padding:\s*0/s,
    "plan cards should let the shadcn card own the compact framed surface"
  );
  assert.match(
    mainStyleSource,
    /\.plan-card-title\s*{[^}]*min-width:\s*0[^}]*overflow:\s*hidden[^}]*text-overflow:\s*ellipsis[^}]*white-space:\s*nowrap/s,
    "plan card titles should use a scoped bounded title hook instead of generic role styling"
  );
  assert.match(
    mainStyleSource,
    /\.plan-card-action\s*{[^}]*align-self:\s*center[^}]*min-width:\s*0[^}]*max-width:\s*100%/s,
    "plan card count actions should stay bounded through the shadcn CardAction slot"
  );
  assert.match(
    mainStyleSource,
    /\.plan-item\s*{[^}]*display:\s*grid[^}]*grid-template-columns:\s*auto minmax\(76px, max-content\) minmax\(0, 1fr\) auto[\s\S]*\.plan-status-checkbox\s*{[^}]*align-self:\s*center[\s\S]*\.plan-title\s*{[^}]*overflow-wrap:\s*anywhere/s,
    "plan rows should reserve checkbox status title and priority columns without widening the transcript"
  );
  assert.match(
    mainStyleSource,
    /\.diff-card-react-root\s*{[^}]*display:\s*contents/s,
    "diff card island roots should not add layout wrappers around transcript cards"
  );
  assert.match(
    mainStyleSource,
    /\.diff-card\s*{[^}]*gap:\s*0[^}]*padding:\s*0[^}]*overflow:\s*hidden/s,
    "diff cards should let the shadcn card own the compact framed surface"
  );
  assert.match(
    mainStyleSource,
    /\.diff-summary:{1,2}after\s*{[^}]*content:\s*none[\s\S]*\.diff-summary \[data-slot="accordion-trigger-icon"\]\s*{[^}]*grid-column:\s*4/s,
    "diff cards should use the shadcn accordion trigger icon instead of the retired details chevron"
  );
  assert.match(
    eventCardSourceTs,
    /className: "event-action-row"/,
    "checkpoint and event actions should keep a scoped semantic row hook while using InlineActions"
  );
  assert.match(
    mainStyleSource,
    /\.empty-state\s*{[^}]*display: grid[^}]*grid-template-columns: minmax\(0, 1fr\)[^}]*justify-items: center[^}]*max-width: min\(640px, calc\(100% - 32px\)\)[^}]*overflow: hidden/s,
    "empty transcript states should render as a centered welcome surface"
  );
  assert.match(
    mainStyleSource,
    /\.empty-state-media\s*{[^}]*justify-self: center[^}]*width: 36px[^}]*height: 36px[^}]*color: var\(--trail-lane\)/s,
    "empty transcript states should position shadcn EmptyMedia as a restrained centered mark"
  );
  assert.match(
    mainStyleSource,
    /\.empty-state-role\s*{[^}]*justify-self: center[^}]*min-width: 0[^}]*max-width: 100%/s,
    "empty transcript role labels should let shadcn badges own their badge chrome while staying bounded"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.empty-state\s+\.card-chrome/,
    "empty transcript states should not keep legacy card-chrome styling"
  );
  assert.doesNotMatch(
    mainStyleSource,
    /\.empty-state:{1,2}before|\.empty-state-filtered:{1,2}before|button\.empty-action:{1,2}before/,
    "welcome empty states should not keep decorative accent lines"
  );
  assert.match(
    mainStyleSource,
    /\.empty-actions\s*{[^}]*display: flex[^}]*flex-wrap: wrap[^}]*width: fit-content[^}]*max-height: min\(164px, 34vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "empty-state action groups should stay focused as compact descriptive actions"
  );
  assert.match(
    mainStyleSource,
    /button\.empty-action\s*{[^}]*display: inline-flex[^}]*gap: 6px[^}]*max-width: min\(220px, 100%\)[^}]*overflow: hidden/s,
    "empty-state actions should render icon and text while staying bounded"
  );
  assert.match(
    mainStyleSource,
    /button\.empty-action\s*{[^}]*padding: 0 10px[^}]*transition:[^}]*transform (?:80ms|\.08s) ease-out/s,
    "empty-state actions should keep compact command geometry and pressed feedback"
  );
  assert.match(
    mainStyleSource,
    /button\.empty-action-primary\s*{[^}]*height: 34px[^}]*background: var\(--button-primary-bg\)[^}]*padding-inline: 12px/s,
    "empty-state primary actions should carry clear primary treatment"
  );
  assert.doesNotMatch(emptyStateCardSourceTs, /data-empty-icon-only|className="sr-only"/, "welcome actions should show descriptive button text instead of icon-only labels");
  assert.match(mainSource, /blocked \? "Open review" : "Write a message"/, "ready welcome primary action should avoid composer jargon");
  assert.match(
    mainSource,
    /emptyStateAction\("attachSelection", "Attach selection"[\s\S]*emptyStateAction\("attachFile", "Attach file"[\s\S]*\]\s*\}\);/s,
    "ready welcome should keep context attachment actions without a settings button"
  );
  assert.doesNotMatch(mainSource, /Start in composer|emptyStateAction\("openSettings", "Settings"/, "ready welcome should remove confusing composer copy and settings action");
  assert.match(
    mainStyleSource,
    /\.empty-action \.empty-action-label,[\s\S]*\.empty-action b\s*{[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "empty-state action labels should truncate before widening the surface"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions\s*{[^}]*display: grid[^}]*grid-template-columns: repeat\(auto-fit, minmax\(min\(148px, 100%\), 1fr\)\)[^}]*width:\s*100%[^}]*max-height: min\(148px, 30vh\)[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "inline action rows should fill their host and wrap like production command rails"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.inline-actions\s*{[^}]*grid-template-columns: repeat\(auto-fit, minmax\(min\(132px, 100%\), 1fr\)\)/s,
    "medium-width VS Code panes should keep action rails in multiple columns when space allows"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\s*{[^}]*min-width: 0[^}]*min-height: 30px[^}]*overflow-wrap: anywhere/s,
    "inline action buttons should tolerate long labels"
  );
  assert.match(
    mainStyleSource,
    /\.inline-actions button\[data-inline-icon-only="true"\]\s*{[^}]*inline-size: 28px[^}]*min-inline-size: 28px[^}]*padding-inline: 0/s,
    "inline action icon-only buttons should stay compact through the shadcn icon button size"
  );
  assert.match(
    mainStyleSource,
    /\.media-preview \.inline-actions\s*{[^}]*display: inline-flex[^}]*width: fit-content/s,
    "media preview action rails should keep icon-only controls compact"
  );
  assert.match(
    mainStyleSource,
    /\.media-preview \.inline-actions button\[data-inline-icon-only="true"\]\s*{[^}]*flex: 0 0 auto/s,
    "media preview icon-only controls should target the shadcn data hook instead of retired icon-button classes"
  );
  assert.match(
    webviewSource,
    /function openMediaPreview\([\s\S]*const closeActions = inlineActions\(\{[\s\S]*className: "media-drawer-actions"[\s\S]*action: "closeDrawer"[\s\S]*\$\{closeActions\}[\s\S]*hydrateInlineActions\(\)\.then\(\(\) => drawer\.querySelector<HTMLElement>\("\[data-action='closeDrawer'\]"\)\?\.focus\(\)\)/,
    "media preview drawer close affordance should render and focus through the shadcn inline action island"
  );
  assert.doesNotMatch(
    webviewSource,
    /function openMediaPreview\([\s\S]*iconButton\("closeDrawer"/,
    "media preview drawer should not use retired raw close icon buttons"
  );
  assert.match(
    mainStyleSource,
    /\.media-drawer-actions\.inline-actions\s*{[^}]*display: inline-flex[^}]*border-color: transparent[^}]*background: transparent[\s\S]*\.media-drawer-actions\.inline-actions button::before\s*{[^}]*content: none[^}]*display: none/s,
    "media drawer shadcn close action should stay compact without helper action chrome"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.approval-detail-list\s*{[^}]*grid-template-columns: minmax\(0, 1fr\)/s,
    "approval detail rows should stack cleanly in narrow panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.empty-state\s*{[^}]*grid-template-columns: minmax\(0, 1fr\)[\s\S]*\.empty-actions\s*{[^}]*justify-content: center/s,
    "empty transcript states should keep compact actions centered in narrow panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.inline-actions,[\s\S]*border-color:\s*CanvasText/s,
    "inline action rails should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.empty-state,[\s\S]*\.empty-actions,[\s\S]*border-color: CanvasText/s,
    "empty transcript states should keep visible borders in forced-colors mode"
  );
  assert.match(eventCardSourceTs, /data: action\.target \? \{ target: action\.target \} : undefined/, "checkpoint recovery actions should carry an explicit target through InlineActions");
  assert.match(mainSource, /target: action\.target/, "checkpoint copy actions should carry the full checkpoint id into the event card island");
  assert.match(
    mainSource,
    /action\.action === "copyCheckpoint" \? "copy" : action\.action === "rewind" \? "rewind" : "message"/,
    "event recovery actions should render familiar icons"
  );
  assert.match(mainSource, /data-highlight-capable/, "file previews should expose whether lazy syntax highlighting is available");
  assert.match(mainSource, /tabindex="0">/, "code previews should be keyboard focusable for scrolling");
  assert.match(webviewSource, /attachments: attachments\.map\(composerAttachmentView\)/, "composer should pass structured attachments into the React shadcn island");
  assert.doesNotMatch(webviewSource, /function attachmentShelf|function attachmentChip|iconButton\("removeAttachment"/, "composer attachment controls should not be rendered with raw helper HTML");
  assert.match(
    webviewSource,
    /function inlineActions\([\s\S]*data-inline-actions-root/,
    "helper action rails should route through the lazy shadcn inline actions island"
  );
  assert.doesNotMatch(
    webviewSource,
    /function iconButton|icon-button/,
    "webview entry should no longer expose a raw icon-button renderer after shadcn action migration"
  );
  assert.doesNotMatch(
    webviewSource,
    /<div class="inline-actions">\s*<button data-action="(?:compareTasks|runTests|openResource|openMediaPreview)"/,
    "helper action rails should no longer render raw inline buttons for migrated actions"
  );
  assert.doesNotMatch(
    webviewSource,
    /<div class="inline-actions conflict-actions">[\s\S]*<button data-action="showConflict"/,
    "conflict helper actions should no longer render raw inline buttons"
  );
  assert.match(
    mainStyleSource,
    /\.code\s*{[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "code previews should reserve scrollbar space and keep nested scrolling controlled"
  );
  assert.match(
    mainStyleSource,
    /\.code\s*{[^}]*color:\s*var\(--syntax-default\)[^}]*line-height:\s*1\.45[^}]*tab-size:\s*2/s,
    "code previews should preserve editor-like fallback readability when Shiki is skipped"
  );
  assert.match(
    mainStyleSource,
    /\.streamdown-markdown \[data-streamdown="code-block"\]\s*{[^}]*margin-block: 8px[^}]*border: 1px solid var\(--border-subtle\)[^}]*border-radius: var\(--radius-control\)[^}]*background: var\(--vscode-textCodeBlock-background\)/s,
    "streaming markdown code blocks should use the same compact frame language as static previews"
  );
  assert.match(
    mainStyleSource,
    /\.streamdown-markdown \[data-streamdown="code-block-body"\],[\s\S]*max-height: min\(320px, 42vh\)[^}]*overflow: auto/s,
    "streaming markdown code blocks should cap their scroll body instead of filling the transcript"
  );
  assert.match(
    mainStyleSource,
    /\.code-frame\s*{[^}]*display: grid[^}]*overflow: hidden[^}]*border: 1px solid var\(--border-subtle\)[^}]*border-radius: var\(--radius-control\)[^}]*background: var\(--vscode-textCodeBlock-background\)/s,
    "code previews should render inside a stable editor-like frame"
  );
  assert.match(
    mainStyleSource,
    /\.code-tools\s*{[^}]*display: grid[^}]*grid-template-columns: minmax\(0, 1fr\) auto auto[^}]*min-height: 28px[^}]*border-bottom: 1px solid var\(--border-subtle\)/s,
    "code preview toolbars should stay tight while reserving room for title, language, and actions"
  );
  assert.match(
    webviewSource,
    /function codeBlock\([\s\S]*const codeActions = inlineActions\(\{[\s\S]*className: "code-actions"[\s\S]*action: "copyCode"[\s\S]*action: "openLocation"[\s\S]*action: "openTextPreview"[\s\S]*\$\{codeActions\}/,
    "code preview toolbar actions should render through the shadcn inline actions island"
  );
  assert.match(
    webviewSource,
    /codeBlock\(code, \{ language: language \|\| "plaintext", title: "Code" \}\)/,
    "message code blocks should use a short toolbar title"
  );
  assert.match(
    webviewSource,
    /const languageLabel = language === "plaintext" \? "text" : language[\s\S]*<span class="code-language">\$\{escapeHtml\(languageLabel\)\}<\/span>/,
    "plain text code blocks should show a compact language chip"
  );
  assert.doesNotMatch(
    webviewSource,
    /function codeBlock\([\s\S]*iconButton\("copyCode"[\s\S]*iconButton\("openTextPreview"/,
    "code preview toolbars should not use retired raw icon buttons for copy/open controls"
  );
  assert.match(
    mainStyleSource,
    /\.code-tools \.code-actions\s*{[^}]*display: inline-flex[^}]*flex-wrap: nowrap[^}]*width: auto[^}]*max-height: none[^}]*border-color: transparent[^}]*background: transparent/s,
    "code preview action rails should keep shadcn inline actions compact in the toolbar"
  );
  assert.match(
    mainStyleSource,
    /\.code-tools \.code-actions button:{1,2}before\s*{[^}]*content: none[^}]*display: none/s,
    "code preview icon-only shadcn buttons should suppress inline action meter decoration"
  );
  assert.match(
    mainStyleSource,
    /\.file-preview\s*{[^}]*display: grid[^}]*gap: 6px[^}]*margin-top: 0/s,
    "file previews should avoid a redundant nested card around the code frame"
  );
  assert.match(
    mainStyleSource,
    /\.code-title span,\s*\.code-title small\s*{[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "file preview titles and metadata should truncate inside the code toolbar"
  );
  assert.match(
    mainStyleSource,
    /\.code-frame > \.code\s*{[^}]*max-height: min\(320px, 42vh\)[^}]*margin: 0[^}]*border: 0[^}]*border-radius: 0[^}]*background: transparent/s,
    "framed code previews should avoid nested boxes around the scroll body"
  );
  assert.match(
    mainStyleSource,
    /\.code-frame:focus-within\s*{[^}]*border-color: var\(--input-active-border\)[^}]*box-shadow: var\(--focus-ring\)/s,
    "keyboard-focused framed code previews should move focus language to the frame"
  );
  assert.match(
    mainStyleSource,
    /\.code:focus-visible\s*{[^}]*border-color:\s*var\(--input-active-border\)[^}]*box-shadow:\s*var\(--focus-ring\)/s,
    "keyboard-focused code previews should show the same strong focus language as developer controls"
  );
  assert.match(
    mainStyleSource,
    /\.code-frame > \.code\[data-highlight-state="?failed"?\],[\s\S]*\.code-frame > \.code\[data-highlight-state="?too-large"?\]\s*{[^}]*box-shadow:\s*inset 3px 0 0 var\(--state-warning-border\)/s,
    "framed code previews should preserve warning cues when Shiki fails or skips large files"
  );
  assert.match(
    mainStyleSource,
    /\.code-frame > \.code\[data-highlight-state="?skipped"?\]\s*{[^}]*box-shadow:\s*inset 3px 0 0 var\(--state-provider-border\)/s,
    "framed code previews should preserve a visible plain-text cue for unsupported languages"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.code-tools\s*{[^}]*grid-template-columns: minmax\(0, 1fr\) auto auto[\s\S]*\.code-language\s*{[^}]*grid-column: auto[^}]*justify-self: end[^}]*max-width: min\(76px, 24vw\)/s,
    "code preview toolbars should keep language chips inline in narrow panes"
  );
  assert.match(
    mainSource,
    /\(effectiveKind === "read" \|\| effectiveKind === "edit"\) && type === "text"[\s\S]*buildFilePreviewModel\(\{[\s\S]*language: languageForResource\(title, "text\/plain"\)/,
    "read and edit text tool content should render as file-typed previews when file evidence is available"
  );
  assert.match(
    mainSource,
    /meta: model\.metaLabel[\s\S]*openPath: filePreviewOpenPath\(model\.title\)/,
    "file previews should fold source counts and open-path behavior into the compact code toolbar"
  );
  assert.match(
    mainSource,
    /model\.language === "markdown"/,
    "read previews should branch on markdown files"
  );
  assert.match(
    mainSource,
    /model\.language === "text" \|\| model\.language === "plaintext"/,
    "read previews should branch on plain text files"
  );
  assert.match(
    mainSource,
    /class="file-document markdown"[\s\S]*class="file-document"/,
    "read previews should render markdown and plain text as document previews instead of source-code blocks"
  );
  assert.match(
    mainStyleSource,
    /\.file-document\s*{[^}]*max-height: min\(380px, 46vh\)[^}]*overflow: auto[^}]*scrollbar-gutter: stable[^}]*border: 1px solid var\(--border-subtle\)/s,
    "file document previews should keep bounded scroll behavior without pretending to be code"
  );
  assert.match(
    mainStyleSource,
    /\.file-document:not\(\.markdown\)\s*{[^}]*white-space: pre-wrap[^}]*font-family: var\(--vscode-font-family\)/s,
    "plain text file previews should wrap as prose instead of using editor monospace code styling"
  );
  assert.doesNotMatch(
    toolCallCardSourceTs,
    /ToolActionBar|ToolEvidenceStrip|ToolFacts|ToolLocations|props\.rawDetails|isReadPermission/,
    "read previews and read permissions should share the simplified tool card without generic details chrome"
  );
  assert.match(
    mainSource,
    /function primaryToolPath\(tool\)[\s\S]*toolArgumentRecord\(asRecord\d*\(tool\?\.rawInput\)\)[\s\S]*pathFromToolTitle\(tool\?\.title\)/,
    "file previews should infer wrapped provider input paths before falling back to provider-title paths"
  );
  assert.match(
    mainSource,
    /function pathFromToolTitle\(title\)[\s\S]*tsx\?/,
    "title path inference should recognize TypeScript-family file reads"
  );
  assert.match(
    mainStyleSource,
    /\.code\.highlighted\s*{[^}]*counter-reset: code-line var\(--code-line-start, 0\)/s,
    "highlighted code previews should reset line numbering from their source offset"
  );
  assert.match(
    mainStyleSource,
    /\.code-line:{1,2}before\s*{[^}]*content: counter\(code-line\)[^}]*border-inline-end/s,
    "highlighted code previews should render a line-number gutter"
  );
  assert.match(
    mainStyleSource,
    /\.code-line:{1,2}before\s*{[^}]*font-variant-numeric:\s*tabular-nums/s,
    "highlighted code preview line numbers should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.code-line:not\(\.code-line-added\):not\(\.code-line-removed\):hover\s*{[^}]*background:\s*color-mix/s,
    "highlighted code previews should expose a subtle row-hover orientation cue"
  );
  assert.match(
    mainStyleSource,
    /\.code:focus-visible \.code-line:{1,2}before\s*{[^}]*color:\s*var\(--text\)/s,
    "keyboard-focused code previews should raise line-number gutter contrast"
  );
  assert.match(
    mainStyleSource,
    /body\.vscode-dark \.shiki-token,\s*body\.vscode-high-contrast \.shiki-token\s*{[^}]*color: var\(--shiki-dark, var\(--shiki-light, var\(--syntax-default\)\)\)/s,
    "highlighted tokens should switch to Shiki dark theme colors with VS Code theme classes"
  );
});

test("settings panel keeps search resilient for production configuration surfaces", () => {
  const extensionSource = fs.readFileSync(extensionScript, "utf8");
  const mainStyleSource = fs.readFileSync(mainStyle, "utf8");
  const settingsPanelSource = fs.readFileSync(path.join(root, "src", "views", "SettingsPanel.ts"), "utf8");

  assert.match(extensionSource, /function settingsFilterTokens/, "settings search should tokenize user queries");
  assert.match(
    extensionSource,
    /tokens\.every\(\(token\) => searchable\.includes\(token\)\)/,
    "settings search should match all query terms regardless of phrase order"
  );
  assert.match(extensionSource, /event\.key !== "Escape"/, "settings search should support Escape to clear");
  assert.match(
    extensionSource,
    /tokens\.join\(" \+ "\)/,
    "settings status should describe multi-term filters"
  );
  assert.match(extensionSource, /function visibleSettingsNavItems/, "settings navigation should skip filtered sections");
  assert.match(extensionSource, /function handleSettingsNavKeydown/, "settings navigation should expose a keyboard handler");
  assert.match(extensionSource, /event\.key === "ArrowDown"/, "settings navigation should support arrow-key movement");
  assert.match(extensionSource, /event\.key === "Home"/, "settings navigation should jump to the first visible section");
  assert.match(extensionSource, /setAttribute\("aria-current", "page"\)/, "settings navigation should mark the active section");
  assert.match(extensionSource, /function updateSettingsNavFromScroll/, "settings navigation should follow the visible scrolled section");
  assert.match(extensionSource, /getBoundingClientRect\(\)\.top <= anchorY/, "settings scroll tracking should choose sections near the top of the viewport");
  assert.match(extensionSource, /requestAnimationFrame\(\(\) =>/, "settings scroll tracking should be frame-throttled");
  assert.match(extensionSource, /addEventListener\("scroll", scheduleSettingsNavFromScroll, \{ passive: true \}\)/, "settings scroll tracking should use a passive listener");
  assert.match(
    mainStyleSource,
    /\.settings-nav\s*{[^}]*position:\s*sticky[^}]*top:\s*72px[^}]*max-height:\s*calc\(100vh - 72px\)[^}]*overflow:\s*auto[^}]*overscroll-behavior:\s*contain[^}]*scrollbar-gutter:\s*stable[^}]*scrollbar-width:\s*thin/s,
    "settings desktop navigation should stay reachable and scroll-stable for long control planes"
  );
  assert.match(
    mainStyleSource,
    /\.provider-matrix\s*{[^}]*max-height: min\(360px, 56vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "settings provider matrix should stay bounded and scroll-stable with many providers"
  );
  assert.match(
    mainStyleSource,
    /\.settings-health-list,\s*\.settings-config-list,\s*\.provider-list\s*{[^}]*max-height: min\(360px, 56vh\)[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "settings variable-length lists should stay bounded and scroll-stable with many diagnostics, config rows, or providers"
  );
  assert.match(
    mainStyleSource,
    /\.provider-matrix-head\s*{[^}]*position: sticky[^}]*top: 0[^}]*z-index: 1/s,
    "settings provider matrix header should remain visible while scrolling provider rows"
  );
  assert.match(
    mainStyleSource,
    /\.provider-matrix-row span:first-child\s*{[^}]*position: sticky[^}]*inset-inline-start: 0[^}]*border-inline-end: 1px solid var\(--border-subtle\)[^}]*box-shadow: 1px 0 0 var\(--border-subtle\)/s,
    "settings provider matrix should pin provider labels while horizontal capability columns scroll"
  );
  assert.match(
    mainStyleSource,
    /\.provider-matrix-head span:first-child\s*{[^}]*z-index: 2[^}]*background: color-mix\(in srgb, var\(--surface-muted\) 72%, transparent\)/s,
    "settings provider matrix pinned header cell should stay above scrolling body cells"
  );
  assert.match(
    mainStyleSource,
    /\.settings-config-row code\s*{[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "settings config values should keep nested scrolling contained"
  );
  assert.match(
    mainStyleSource,
    /\.settings-config-row code\s*{[^}]*overflow-wrap: anywhere/s,
    "settings config values should allow long tokens to break inside their frame"
  );
  assert.match(
    mainStyleSource,
    /\.provider-command\s*{[^}]*overflow: auto[^}]*overscroll-behavior: contain[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "settings provider commands should keep nested scrolling contained"
  );
  assert.match(
    mainStyleSource,
    /\.provider-command\s*{[^}]*overflow-wrap: anywhere/s,
    "settings provider commands should wrap long executable and argument tokens"
  );
  assert.match(
    mainStyleSource,
    /\.provider-routing-fact dd\s*{[^}]*font-variant-numeric: tabular-nums/s,
    "settings provider routing counts should stay optically stable"
  );
  assert.match(
    mainStyleSource,
    /\.settings-header-actions,\s*\.settings-inline-actions,\s*\.settings-hero-actions\s*{[^}]*display: grid[^}]*grid-template-columns: repeat\(auto-fit, minmax\(min\(148px, 100%\), 1fr\)\)[^}]*border: 1px solid color-mix\(in srgb, var\(--border-subtle\) 74%, transparent\)[^}]*padding: 4px/s,
    "settings action clusters should render as responsive command rails"
  );
  assert.match(
    mainStyleSource,
    /\.settings-header-actions button,[\s\S]*\.settings-hero-actions button,[\s\S]*\.provider-routing-action button\s*{[^}]*min-width: 0[^}]*max-width: 100%[^}]*overflow: hidden[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/s,
    "settings action buttons should truncate before overflowing narrow panes"
  );
  assert.match(extensionSource, /function settingsActionIcon\(type\)/, "settings actions should render typed command icons");
  assert.match(extensionSource, /settings-action-icon/, "settings action icons should be present in the settings markup");
  assert.doesNotMatch(
    settingsPanelSource,
    /<button\s+data-action=/,
    "settings panel should route every visible command through the typed action helper"
  );
  assert.match(
    settingsPanelSource,
    /settingsActionButton\(\{ type: "openSettings", key: row\.key, label: "Edit"/,
    "settings config-row edits should use the typed action helper"
  );
  assert.match(
    extensionSource,
    /aria-label=.*action\.label.*action\.detail/s,
    "settings action buttons should expose descriptive labels to assistive technology"
  );
  assert.match(
    mainStyleSource,
    /\.settings-action-button\s*{[^}]*position: relative[^}]*display: inline-flex[^}]*padding-inline: 14px 9px[^}]*transition:[^}]*transform (?:80ms|\.08s) ease-out/s,
    "settings action buttons should use compact icon command geometry and pressed feedback"
  );
  assert.match(
    mainStyleSource,
    /\.settings-action-button:{1,2}before\s*{[^}]*content: ""[^}]*inset-inline-start: 6px[^}]*width: 3px[^}]*background: var\(--border-subtle\)/s,
    "settings action buttons should expose compact typed meters without extra markup"
  );
  assert.match(
    mainStyleSource,
    /\.settings-action-opensettings,[\s\S]*\.settings-action-customproviders\s*{[^}]*border-color: var\(--state-provider-border\)[^}]*background: color-mix\(in srgb, var\(--trail-provider\) 6%, var\(--surface-muted\)\)/s,
    "settings edit and provider actions should carry provider/config semantics"
  );
  assert.match(
    mainStyleSource,
    /\.settings-action-doctor\s*{[^}]*border-color: var\(--state-warning-border\)[^}]*background: color-mix\(in srgb, var\(--trail-review\) 7%, var\(--surface-muted\)\)/s,
    "settings doctor actions should carry review/diagnostic semantics"
  );
  assert.match(
    mainStyleSource,
    /\.settings-action-startdaemon\s*{[^}]*border-color: var\(--state-lane-border\)[^}]*background: color-mix\(in srgb, var\(--trail-lane\) 7%, var\(--surface-muted\)\)/s,
    "settings daemon actions should carry lane/workflow semantics"
  );
  assert.match(
    mainStyleSource,
    /\.provider-card header > div\s*{[^}]*min-width: 0/s,
    "settings provider card titles should be shrinkable beside status badges"
  );
  assert.match(
    mainStyleSource,
    /\.provider-card header\s*{[^}]*display: flex[^}]*flex-wrap: wrap[^}]*min-width: 0/s,
    "settings provider card headers should wrap deliberately instead of squeezing badges"
  );
  assert.match(
    mainStyleSource,
    /\.provider-card header > div\s*{[^}]*flex: 1 1 220px[^}]*min-width: 0/s,
    "settings provider card title column should reserve useful wrapping width"
  );
  assert.match(
    mainStyleSource,
    /\.provider-badges\s*{[^}]*flex: 0 1 auto[^}]*flex-wrap: wrap[^}]*max-width: 100%/s,
    "settings provider badges should wrap within the card header"
  );
  assert.match(
    mainStyleSource,
    /\.provider-badges \.status\s*{[^}]*max-width: min\(160px, 100%\)/s,
    "settings provider badges should stay bounded for long localized labels"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 900px\)\s*{[\s\S]*\.settings-nav\s*{[^}]*max-height: none[^}]*overflow-x: auto[^}]*overflow-y: hidden[^}]*overscroll-behavior-inline: contain[^}]*scroll-snap-type: x proximity[^}]*scrollbar-gutter: stable[^}]*scrollbar-width: thin/s,
    "settings navigation should scroll predictably in narrow VS Code panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.settings-header\s*{[^}]*position: static[^}]*flex-direction: column[\s\S]*\.settings-nav\s*{[^}]*top: 0/s,
    "settings header and nav should avoid sticky overlap in very narrow panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.settings-section-heading\s*{[^}]*align-items: stretch[^}]*flex-direction: column[\s\S]*\.settings-header-actions,\s*\.settings-inline-actions,\s*\.settings-hero-actions\s*{[^}]*width: 100%[^}]*max-width: 100%[^}]*justify-content: stretch/s,
    "settings action rails should stack cleanly in narrow panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.provider-badges\s*{[^}]*justify-content: flex-start/s,
    "settings provider badges should align with wrapped content in narrow panes"
  );
  assert.match(
    mainStyleSource,
    /@media \(max-width: 640px\)\s*{[\s\S]*\.settings-facts div\s*{[^}]*grid-template-columns: minmax\(0, 1fr\)[^}]*gap: 3px/s,
    "settings facts should stack instead of squeezing long workspace diagnostics"
  );
  assert.match(
    mainStyleSource,
    /@media \(pointer: coarse\)\s*{[\s\S]*\.settings-nav a,[\s\S]*min-height:\s*44px/s,
    "settings navigation links should keep coarse-pointer touch targets"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.provider-matrix-row span:first-child\s*{[^}]*box-shadow: 1px 0 0 CanvasText/s,
    "settings provider matrix pinned column should keep a visible forced-colors divider"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.settings-header-actions,[\s\S]*\.settings-hero-actions,[\s\S]*border-color: CanvasText/s,
    "settings action rails should keep visible borders in forced-colors mode"
  );
  assert.match(
    mainStyleSource,
    /@media \(forced-colors: active\)\s*{[\s\S]*\.settings-action-button:{1,2}before,[\s\S]*button\.review-primary-action:{1,2}before[\s\S]*background: CanvasText/s,
    "settings action meters should remain visible in forced-colors mode"
  );
});

test("timeline search keeps filtered transcripts recoverable", () => {
  const mainSource = fs.readFileSync(path.join(root, "src", "webview", "main.ts"), "utf8");

  assert.match(mainSource, /function clearTimelineSearch/, "timeline filters should share a clear helper");
  assert.match(mainSource, /timeline-search-input/, "timeline search input should be identifiable for keyboard recovery");
  assert.match(mainSource, /event\.key === "Escape"/, "timeline search should support Escape-based recovery");
  assert.match(
    mainSource,
    /No transcript items matched/,
    "filtered empty transcript state should explain the active constraints"
  );
});

test("webview initializes icon registry before startup render", () => {
  const mainSource = fs.readFileSync(path.join(root, "src", "webview", "main.ts"), "utf8");
  const iconRegistryIndex = mainSource.indexOf("const LUCIDE_ICONS");
  const resizeObserverBatchIndex = mainSource.indexOf("installFrameBatchedResizeObserver();");
  const startupRenderIndex = mainSource.indexOf("render();\nvscode.postMessage({ type: \"ready\" });");

  assert.notEqual(iconRegistryIndex, -1, "webview bundle should initialize Lucide icons");
  assert.notEqual(resizeObserverBatchIndex, -1, "webview bundle should batch ResizeObserver callbacks before UI startup");
  assert.notEqual(startupRenderIndex, -1, "webview bundle should render and announce ready on startup");
  assert.ok(iconRegistryIndex < startupRenderIndex, "Lucide icons must be initialized before the first render");
  assert.ok(
    resizeObserverBatchIndex < startupRenderIndex,
    "ResizeObserver batching must be installed before lazy scroller islands mount"
  );
});

test("webview streaming state updates refresh existing islands instead of tearing down the shell", () => {
  const source = fs.readFileSync(path.join(root, "src", "webview", "main.ts"), "utf8");
  const eventCardSourceTs = fs.readFileSync(path.join(root, "src", "webview", "EventCard.tsx"), "utf8");
  const stateHandler = source.match(/if \(message\.type === "state"\) \{[\s\S]*?\n  \}\n\n  if \(message\.type === "error"\)/)?.[0] || "";

  assert.match(source, /const STREAM_RENDER_INTERVAL_MS = \d+;/, "state updates should define a bounded streaming render cadence");
  assert.match(
    source,
    /function scheduleRender\(\): void \{[\s\S]*window\.setTimeout[\s\S]*window\.requestAnimationFrame[\s\S]*renderStateUpdate\(\)/,
    "streaming renders should be coalesced and routed through the state-update renderer"
  );
  assert.match(
    source,
    /function renderStateUpdate\(\): void \{[\s\S]*prepareRenderProps\(visibleNodes\)[\s\S]*hydrateExistingShell/,
    "state-update renders should refresh props and hydrate existing React islands"
  );
  assert.match(
    source,
    /function installFrameBatchedResizeObserver\(\): void \{[\s\S]*class FrameBatchedResizeObserver implements ResizeObserver[\s\S]*pendingEntries\.set\(entry\.target, entry\)[\s\S]*window\.requestAnimationFrame\(\(\) => \{[\s\S]*callback\(deliveredEntries, this\)[\s\S]*window\.ResizeObserver = FrameBatchedResizeObserver/,
    "ResizeObserver callbacks should be frame-batched so streaming scroll writes do not run during observer delivery"
  );
  assert.match(
    source,
    /function render\(\): void \{[\s\S]*syncAppShell\(\{ headerHtml, composerHtml, reviewHtml \}\);[\s\S]*hydrateExistingShell/,
    "full renders should reconcile the existing app shell instead of blindly replacing mounted roots"
  );
  assert.match(
    source,
    /function syncAppShell\([\s\S]*querySelector<HTMLElement>\(":scope > \.shell"\)[\s\S]*shell\.className = `shell \$\{reviewVisible \? "review-open" : ""\}`\.trim\(\)[\s\S]*syncHeaderShell[\s\S]*syncComposerShell[\s\S]*syncReviewShell/,
    "app shell reconciliation should preserve header, timeline, composer, and review island roots"
  );
  assert.doesNotMatch(
    source.match(/async function hydratePatchedNodes\(\): Promise<void> \{[\s\S]*?\n}\n\nfunction applyStreamingTextDomPatch/)?.[0] || "",
    /await hydrateReactIslands\(\)/,
    "token-level live text patches should update DOM text directly without global island hydration"
  );
  assert.match(
    source,
    /patches \? patches\.every\(isLocallyHydratablePatchPayload\) : true/,
    "existing presentation and hidden chrome node patches should hydrate locally instead of forcing a full shell render"
  );
  assert.match(
    source,
    /function hydratePatchedNodes\(\): Promise<void> \{[\s\S]*const directTargets = emptyPatchedTimelineHydrationTargets\(\)[\s\S]*hydratePatchedNodeIslandDirectly\(node, directTargets\)[\s\S]*await hydratePatchedTimelineIslands\(directTargets\)[\s\S]*refreshTimelineGroupsForPatchedNodes\(presentationRefreshNodeIds\)/,
    "live component patches should update mounted node islands directly before falling back to group refresh"
  );
  assert.match(
    source,
    /function hydratePatchedNodes\(\): Promise<void> \{[\s\S]*let needsTimelineChromeHydration = false[\s\S]*if \(!node\) \{[\s\S]*state\.nodes\.some\(\(candidate\) => candidate\.id === nodeId\)[\s\S]*needsTimelineChromeHydration = true[\s\S]*await hydrateTimelineChromeForPatchedNodes\(visibleNodes\)/,
    "patched hidden timeline nodes should still refresh header and transcript navigation while filters are active"
  );
  assert.match(
    source,
    /async function hydrateTimelineChromeForPatchedNodes\(visibleNodes: RenderNode\[\]\): Promise<void> \{[\s\S]*timelineNavigation\(visibleNodes\)[\s\S]*header\(state\.task\)[\s\S]*Promise\.all\(\[hydrateHeaderBars\(\), hydrateTimelineNavigation\(\)\]\)/,
    "filtered patched-node chrome refresh should recompute navigation and header props before hydrating them"
  );
  assert.match(
    source,
    /function isPatchedTimelineCardNode\(node: RenderNode\): boolean \{[\s\S]*node\.kind === "approval"[\s\S]*node\.kind === "checkpoint"[\s\S]*node\.kind === "completion"[\s\S]*node\.kind === "resource"[\s\S]*node\.kind === "unknown"/,
    "approval, checkpoint, completion, resource, and unknown event patch updates should refresh mounted timeline cards instead of forcing full transcript renders"
  );
  assert.match(
    source,
    /function hydratePatchedNodeIslandDirectly\([\s\S]*renderNode\(node\)[\s\S]*syncNodeShellFromHtml\(node, html\)[\s\S]*targets\.nodeIds\.add\(node\.id\)[\s\S]*collectHelperIslandIdsFromHtml\(html, targets\)/,
    "direct component hydration should refresh node props and shell attributes without replacing the group body"
  );
  assert.match(
    source,
    /function canHydrateMountedNodeIslandDirectly\(node: RenderNode\): boolean \{[\s\S]*!isLiveStatus\(node\.status\)[\s\S]*document\.getElementById\(nodeDomId\(node\.id\)\)[\s\S]*mountedNodeIslandRoot\(article, node\)/,
    "direct component hydration should be limited to live mounted node islands"
  );
  assert.match(
    source,
    /function syncNodeShellFromHtml\(node: RenderNode, html: string\): boolean \{[\s\S]*current\.tagName !== next\.tagName[\s\S]*syncElementAttributes\(current, next\)/,
    "direct component hydration should sync outer article attributes while preserving mounted children"
  );
  assert.match(
    source,
    /async function hydratePatchedTimelineIslands\(targets: PatchedTimelineHydrationTargets\): Promise<void> \{[\s\S]*hydrateTimelineGroups\(\{ ids: targets\.groupIds \}\)[\s\S]*hydrateMessageCards\(\{ ids: targets\.nodeIds \}\)[\s\S]*hydrateToolCallGroupCards\(\{ ids: targets\.toolGroupIds \}\)[\s\S]*hydratePayloadDisclosures\(\{ ids: targets\.payloadDisclosureIds \}\)[\s\S]*hydrateInlineActions\(\{ ids: targets\.inlineActionIds \}\)/,
    "component status patches should hydrate only affected timeline groups, node islands, and helper islands"
  );
  assert.match(
    source,
    /async function hydratePatchedTimelineIslands\(targets: PatchedTimelineHydrationTargets\): Promise<void> \{[\s\S]*hydrateEventCards\(\{ ids: targets\.nodeIds \}\)/,
    "checkpoint, completion, resource, and other event-card patches should hydrate affected event islands only"
  );
  assert.match(
    source,
    /function mountedNodeIslandRoot\(article: HTMLElement, node: RenderNode\): Element \| null \{[\s\S]*case "approval":[\s\S]*querySelector\("\[data-tool-call-card-root\]"\)[\s\S]*case "checkpoint":[\s\S]*case "completion":[\s\S]*case "resource":[\s\S]*case "unknown":[\s\S]*querySelector\("\[data-event-card-root\]"\)/,
    "direct patched-node hydration should find approval tool-card roots and all event-card roots"
  );
  assert.match(
    eventCardSourceTs,
    /ids\?: ReadonlySet<string> \| undefined[\s\S]*options\.ids && !options\.ids\.has\(nodeId\)[\s\S]*activeIds\.add\(nodeId\)/,
    "event-card hydration should support scoped patched-node refresh without unmounting untouched event cards"
  );
  assert.match(
    source,
    /function collectPatchedTimelinePayloadDisclosureIds\(targets: PatchedTimelineHydrationTargets\): void \{[\s\S]*querySelectorAll<HTMLElement>\("\[data-payload-disclosure-id\]"\)[\s\S]*targets\.payloadDisclosureIds\.add\(id\)/,
    "patched helper hydration should collect payload disclosure ids from the affected DOM scopes after card hydration"
  );
  assert.match(
    source,
    /function collectPatchedTimelineInlineActionIds\(targets: PatchedTimelineHydrationTargets\): void \{[\s\S]*querySelectorAll<HTMLElement>\("\[data-inline-actions-id\]"\)[\s\S]*targets\.inlineActionIds\.add\(id\)/,
    "patched helper hydration should collect inline action ids from the affected DOM scopes after payload hydration"
  );
  assert.doesNotMatch(
    source.match(/async function hydratePatchedTimelineIslands\(targets: PatchedTimelineHydrationTargets\): Promise<void> \{[\s\S]*?\n}\n\nfunction collectPatchedTimelinePayloadDisclosureIds/)?.[0] || "",
    /hydratePayloadDisclosures\(\);\s*[\s\S]*hydrateInlineActions\(\);/,
    "patched-node helper hydration should not rescan every helper island"
  );
  assert.match(
    fs.readFileSync(path.join(root, "src", "webview", "renderPatchModel.ts"), "utf8"),
    /node\.kind === "tool"[\s\S]*node\.kind === "terminal"[\s\S]*node\.kind === "approval"[\s\S]*node\.kind === "checkpoint"[\s\S]*node\.kind === "completion"/,
    "tool, approval, checkpoint, and completion patches should remain eligible for local hydration"
  );
  assert.match(
    source,
    /const changes = changedRenderNodesFromPatches\(beforeNodes, patches\)/,
    "render patch changes should be derived from patch ids instead of scanning the full rendered node array"
  );
  assert.match(
    source,
    /function routeRenderChanges\([\s\S]*if \(canHydrateTimelineStructure\(visibleChanges\)\) \{[\s\S]*scheduleTimelineStructureHydration\(\{ includeChrome: needsChromeHydration \}\);[\s\S]*return;/,
    "structural timeline patches should use a timeline-only hydration path before falling back to a soft full render"
  );
  assert.match(
    source,
    /if \(canHydratePatchedNodes\(visibleChanges\)\) \{[\s\S]*const immediateStreamingNodeIds = streamingTextDomPatchableNodeIds\(visibleChanges\)[\s\S]*const keepBottomForDeferredNodes =[\s\S]*shouldKeepTimelinePinnedToBottom\(document\.querySelector<HTMLElement>\("\.timeline"\)\)[\s\S]*applyStreamingTextDomPatchesImmediately\(immediateStreamingNodeIds\)[\s\S]*const deferredNodeIds = changedNodeIdsWithout\(visibleChanges\.changedNodeIds, immediateStreamingNodeIds\)[\s\S]*schedulePatchedNodeHydration\(deferredNodeIds, \{ lockBottom: keepBottomForDeferredNodes \}\)/,
    "mixed streaming batches should update mounted Streamdown text immediately and defer only component islands"
  );
  assert.match(
    source,
    /let pendingPatchedNodeBottomLock = false[\s\S]*function schedulePatchedNodeHydration\([\s\S]*options: \{ lockBottom\?: boolean \| undefined \} = \{\}[\s\S]*pendingPatchedNodeBottomLock \|\|= Boolean\(options\.lockBottom\)[\s\S]*const forcePatchedBottomLock = pendingPatchedNodeBottomLock[\s\S]*pendingPatchedNodeBottomLock = false[\s\S]*forcePatchedBottomLock && Date\.now\(\) >= timelineBottomLockUserPauseUntil/,
    "deferred component hydration should preserve bottom-pinned intent from mixed streaming batches"
  );
  assert.match(
    source,
    /function streamingTextDomPatchableNodeIds\(changes: RenderPatchChanges\): Set<string> \{[\s\S]*streamingTextForNode\(node\) === undefined[\s\S]*querySelector\("\[data-streaming-markdown\]"\)[\s\S]*ids\.add\(id\)/,
    "immediate streaming selection should be per-node so component updates do not delay live text"
  );
  assert.match(
    source,
    /function isLocallyHydratablePatchPayload\(patch: RenderPatch\): boolean \{[\s\S]*isHydratableNodePatchPayload\(patch\)[\s\S]*isHiddenChromeNode\(patch\.node\)/,
    "timeline-only hydration should be limited to structural presentation patches and hidden chrome state"
  );
  const timelineStructureHydration = source.match(/async function hydrateTimelineStructure\(\): Promise<void> \{[\s\S]*?\n}\n\nasync function hydratePatchedNodes/)?.[0] || "";
  assert.match(
    timelineStructureHydration,
    /prepareRenderProps\(visibleNodes\)[\s\S]*hydrateTimelineScroller\(\)[\s\S]*hydrateReactIslands\(\)/,
    "timeline-only hydration should refresh timeline props and islands"
  );
  assert.match(
    timelineStructureHydration,
    /const includeChrome = pendingTimelineStructureChromeHydration[\s\S]*if \(includeChrome\) \{[\s\S]*composer\(\)[\s\S]*hydrateComposerCards\(\)\.then\(\(\) => \{[\s\S]*restoreComposerInput/,
    "timeline-only hydration should touch composer islands only when chrome state changed"
  );
  assert.doesNotMatch(
    source.match(/function applyRenderPatchMessage\([\s\S]*?\n}\n\nfunction requestRenderStateResync/)?.[0] || "",
    /persistWebviewState\(\)/,
    "render patch messages should not write webview UI state on every streaming tick"
  );
  assert.match(
    source,
    /function applyStreamingTextDomPatch\(node: RenderNode\): boolean \{[\s\S]*node\.kind !== "message" && node\.kind !== "thought"[\s\S]*querySelector<HTMLElement>\("\[data-streaming-markdown\]"\)[\s\S]*renderNode\(node\)[\s\S]*updateStreamingTextTarget\(streamTarget, streamText\)/,
    "streaming DOM fast path should push live agent messages and thoughts into mounted Streamdown islands"
  );
  assert.match(
    source,
    /function canApplyStreamingTextDomPatchesImmediately\(changes: RenderPatchChanges\): boolean \{[\s\S]*const ids = streamingTextDomPatchableNodeIds\(changes\)[\s\S]*return ids\.size > 0 && ids\.size === changes\.changedNodeIds\.size/,
    "immediate streaming path should be limited to mounted text-only message and thought nodes"
  );
  assert.match(
    source,
    /function applyStreamingTextDomPatchesImmediately\(nodeIds: Set<string>\): void \{[\s\S]*applyStreamingTextDomPatch\(node\)[\s\S]*lockTimelineToBottom\(timeline\)/,
    "immediate streaming path should keep the transcript bottom locked while updating live markdown"
  );
	  assert.match(
	    source,
	    /function updateStreamingTextTarget\(streamTarget: HTMLElement, streamText: string\): void \{[\s\S]*dataset\.streamdownMarkdown !== undefined[\s\S]*__trailQueueStreamdownText\(streamText\)[\s\S]*dispatchEvent\(new CustomEvent\(TRAIL_STREAMDOWN_UPDATE_EVENT[\s\S]*streamText\.startsWith\(current\)[\s\S]*\(firstChild as Text\)\.appendData\(streamText\.slice\(current\.length\)\)[\s\S]*streamTarget\.textContent = streamText/,
	    "streaming DOM fast path should queue mounted Streamdown updates and keep text-node append as a fallback"
	  );
		  assert.match(
		    source,
		    /const timeline = document\.querySelector<HTMLElement>\("\.timeline"\);[\s\S]*const forcePatchedBottomLock = pendingPatchedNodeBottomLock[\s\S]*const restorePatchedBottom =[\s\S]*forcePatchedBottomLock[\s\S]*shouldKeepTimelinePinnedToBottom\(timeline\)[\s\S]*if \(streamedTextDomPatchApplied && restorePatchedBottom && timeline\?\.isConnected\) \{[\s\S]*timeline\.scrollTop = timeline\.scrollHeight/,
		    "streaming DOM fast path should keep bottom-pinned transcripts anchored without invoking full scroller hydration"
		  );
		  assert.match(
		    source,
		    /function shouldKeepTimelinePinnedToBottom\(timeline: HTMLElement \| null\): boolean \{[\s\S]*isTimelinePinnedToBottom\(timeline\)[\s\S]*timelineBottomLockPinned && Date\.now\(\) >= timelineBottomLockUserPauseUntil/,
		    "bottom restore decisions should honor the internal bottom lock during transient layout drift"
		  );
		  assert.match(
		    source,
		    /function captureRenderFocus\(\): RenderFocusSnapshot \{[\s\S]*const oldTimeline = document\.querySelector<HTMLElement>\("\.timeline"\);[\s\S]*const wasPinnedToBottom = shouldKeepTimelinePinnedToBottom\(oldTimeline\)/,
		    "structural hydration should preserve bottom intent from the bottom lock, not just instantaneous scroll distance"
		  );
		  assert.match(
		    source,
		    /if \(restorePatchedBottom && timeline\?\.isConnected\) \{[\s\S]*lockTimelineToBottom\(timeline\);\s*queueTimelineBottomRestore\(\);/,
		    "deferred component hydration should queue a bottom restore after React islands resize"
		  );
		  assert.match(
		    source,
		    /let timelineBottomLockFrameHandle: number \| undefined[\s\S]*let timelineBottomLockObserver: ResizeObserver \| undefined[\s\S]*function installTimelineBottomLock\(\): void \{[\s\S]*new ResizeObserver\(\(\) => \{[\s\S]*queueTimelineBottomRestore\(\)/,
	    "timeline should install a queued ResizeObserver bottom lock for DOM-only streaming growth"
	  );
	  assert.doesNotMatch(
	    source,
	    /timelineBottomLockObserver\.observe\(timeline\)/,
	    "timeline bottom lock should not observe the scroller it mutates while restoring scrollTop"
	  );
	  assert.match(
	    source,
	    /function queueTimelineBottomRestore\(\): void \{[\s\S]*timelineBottomLockFrameHandle[\s\S]*window\.requestAnimationFrame\(\(\) => \{[\s\S]*restoreTimelineBottomFromResizeObserver\(\)/,
	    "ResizeObserver bottom lock should coalesce scrollTop writes outside the observer callback"
	  );
	  assert.match(
	    source,
	    /function restoreTimelineBottomFromResizeObserver\(\): void \{[\s\S]*timelineBottomLockPinned[\s\S]*timeline\.scrollTop = timeline\.scrollHeight/,
	    "ResizeObserver bottom lock should restore bottom after streaming content changes height"
	  );
	  assert.match(
	    source,
	    /function restoreTimelineScrollFromFocus\(focus: RenderFocusSnapshot\): void \{[\s\S]*focus\.wasPinnedToBottom[\s\S]*timeline\.scrollTop = focus\.previousScrollTop;[\s\S]*setTimelineBottomLockPinned\(false\)/,
	    "timeline bottom lock should remain disabled when the user was reading earlier transcript content"
	  );
	  assert.match(
	    source,
	    /await hydrateReactIslands\(\);[\s\S]*restoreTimelineBottomAfterIslandHydration\(focus\)/,
	    "lazy island hydration should re-anchor bottom-pinned transcripts after cards expand"
	  );
  assert.match(
    source,
    /function persistWebviewState\(\): void \{[\s\S]*const json = JSON\.stringify\(snapshot\)[\s\S]*if \(json === lastPersistedWebviewStateJson\)[\s\S]*vscode\.setState\(snapshot\)/,
    "webview UI state persistence should skip no-op writes"
  );
  assert.match(stateHandler, /persistWebviewState\(\);[\s\S]*routeRenderChanges\(\{[\s\S]*chromeStateChanged:[\s\S]*timelineFrameStateChanged:/, "host state messages should route through local hydration before falling back to a soft refresh");
  assert.doesNotMatch(stateHandler, /persistWebviewState\(\);\s*render\(\);/, "host state messages should not rebuild the entire webview shell");
});

test("extension host streams applied patches without rescanning the transcript", () => {
  const source = fs.readFileSync(path.join(root, "src", "views", "ChatPanel.ts"), "utf8");
  const patchSender = source.match(/private applyAndPostRenderPatches\([\s\S]*?\n  \}\n\n  private canCoalesceRenderPatch/)?.[0] || "";
  const completionHandler = source.match(/private handlePromptComplete\([\s\S]*?\n  \}\n\n  private finalizeCurrentTurnPatches/)?.[0] || "";
  const promptFinally = source.match(/finally \{[\s\S]*?shouldRefreshChrome[\s\S]*?\n    \}/)?.[0] || "";

  assert.match(
    patchSender,
    /const applied = applyRenderPatchesAndCollect\(this\.nodes, patches\)/,
    "host patch sender should collect actual applied patches while applying reducer changes"
  );
  assert.doesNotMatch(
    patchSender,
    /beforeById|nextIds|for \(const node of nextNodes\)/,
    "host patch sender should not diff every transcript node on each streaming tick"
  );
  assert.match(
    source,
    /function sameCoalescableRenderScope\(existing: RenderNode, incoming: RenderNode\): boolean \{[\s\S]*existing\.id === incoming\.id[\s\S]*existing\.turnId === incoming\.turnId[\s\S]*existing\.acpSessionId === incoming\.acpSessionId[\s\S]*existing\.source === incoming\.source/,
    "host component coalescing should require the same render scope, not only a reused node id"
  );
  assert.match(
    source,
    /return this\.nodes\.some\(\(existing\) => sameCoalescableRenderScope\(existing, node\)\);/,
    "host component coalescing should not treat reused ids from another turn as in-place updates"
  );
  assert.match(
    source,
    /update: \(update, sessionId\) => this\.handleAcpUpdate\(update, sessionId\)/,
    "ACP update notifications should keep the outer session id available for render context"
  );
  assert.match(
    source,
    /private handleAcpUpdate\(update: SessionUpdate, sessionId\?: string \| undefined\): void \{[\s\S]*if \(sessionId && !this\.acpSessionId\)[\s\S]*this\.streamScheduler\.push\(reduceSessionUpdate\(update, this\.renderContext\(undefined, sessionId \|\| this\.acpSessionId\)\)\)/,
    "live ACP updates should render with the provider session id even before session startup bookkeeping finishes"
  );
  assert.match(
    source,
    /private renderContext\(lane = this\.task\?\.lane \|\| "new-task", acpSessionId = this\.acpSessionId\): RenderReduceContext/,
    "render context should allow an update-scoped ACP session id override"
  );
  assert.match(
    completionHandler,
    /this\.sending = false;[\s\S]*this\.applyAndPostRenderPatches\(\[[\s\S]*this\.finalizeCurrentTurnPatches\(completion\.status,\s*completion\.updatedAt\)[\s\S]*node: completion/,
    "prompt completion should stream final node changes as render patches instead of posting a full state refresh"
  );
  assert.match(
    source,
    /this\.sending = true;\s*this\.applyAndPostRenderPatches\(\[\], \{ force: true \}\);/,
    "prompt start should update running chrome through an empty render patch instead of a full state post"
  );
  assert.doesNotMatch(
    source,
    /this\.sending = true;\s*this\.postState\(\);/,
    "prompt start should not schedule a full state render before streaming begins"
  );
  assert.match(
    source,
    /this\.applyAndPostRenderPatches\(sessionControlsToPatches\(session\.session, this\.renderContext\(\)\), \{ force: true \}\)/,
    "ACP session controls should hydrate chrome through render patches during active streaming"
  );
  assert.doesNotMatch(
    source,
    /sessionControlsToPatches\(session\.session, this\.renderContext\(\)\), \{ force: true \}\);[\s\S]{0,500}this\.postState\(\);/,
    "ACP session startup should not follow forced chrome patches with a full state post"
  );
  assert.match(
    source,
    /private handlePermission\(requestId: string, params: RequestPermissionParams\): void \{[\s\S]*if \(params\.sessionId && !this\.acpSessionId\)[\s\S]*reducePermissionRequest\(requestId, params, this\.renderContext\(undefined, params\.sessionId \|\| this\.acpSessionId\)\)/,
    "permission requests should hydrate through render patches instead of full transcript state"
  );
  assert.match(
    source,
    /private resolveApprovals\([\s\S]*renderNodeSnapshotPatches\(this\.nodes, nextNodes\)[\s\S]*\{ force: true \}/,
    "permission resolution should update approval nodes through render patches"
  );
  assert.match(
    source,
    /private markProviderFailure\([\s\S]*renderNodeSnapshotPatches\(this\.nodes, nextNodes\)[\s\S]*\{ force: true \}/,
    "provider failures during a turn should finalize live nodes through render patches"
  );
  assert.match(
    source,
    /addAttachments\([\s\S]*this\.applyAndPostRenderPatches\(\[\], \{ force: true \}\);/,
    "attachment chrome changes should not post full transcript state"
  );
  assert.match(
    source,
    /transport: "patch"/,
    "post-prompt task refresh should merge durable Trail state through render patches"
  );
  assert.match(
    source,
    /renderNodeSnapshotPatches\(this\.nodes, nextNodes\)[\s\S]*\{ force: true \}/,
    "patch refresh should post only node deltas plus chrome metadata"
  );
  assert.match(
    promptFinally,
    /const shouldRefreshChrome = this\.sending \|\| clearedAttachments;[\s\S]*if \(shouldRefreshChrome\) \{[\s\S]*this\.applyAndPostRenderPatches\(\[\], \{ force: true \}\)/,
    "prompt finally should send chrome-only patches when completion did not already close the turn"
  );
  assert.doesNotMatch(
    promptFinally,
    /this\.postState\(\)/,
    "prompt finally should not post a full state after completion already updated streaming state"
  );
  assert.match(
    source,
    /this\.applyAndPostRenderPatches\(\[\s*\{\s*type: "upsert",\s*node: \{\s*id: `message:user:\$\{this\.currentTurnId\}`/,
    "user message insertion should use render patches instead of posting full transcript state"
  );
  assert.match(
    patchSender,
    /\.\.\.this\.renderPatchStateFields\(\{ includeChrome: Boolean\(options\.force\) \}\)/,
    "stream token patches should carry only minimal state unless a chrome refresh is explicitly forced"
  );
  assert.match(
    source,
    /if \(!options\.includeChrome\) \{[\s\S]*return stateFields;[\s\S]*providers: new ProviderRegistry/,
    "full provider and task metadata should stay out of ordinary token patch messages"
  );
});

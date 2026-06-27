import type { RenderNode } from "../shared/renderModel";
import { coordinationSummaryFromSources, type CoordinationSummary } from "../shared/coordinationSummary";
import { conflictSetIdsFromSources } from "../shared/conflicts";
import { redactedJson, redactString } from "../shared/securityRedaction";
import "./styles.css";

declare const acquireVsCodeApi: () => {
  postMessage(message: unknown): void;
  getState(): unknown;
  setState(state: unknown): void;
};

interface WebviewState {
  task?: {
    id: string;
    lane: string;
    title: string;
    status: string;
    provider?: string | undefined;
    model?: string | undefined;
    changedPaths?: string[] | undefined;
    coordination?: CoordinationSummary | undefined;
    nextAction?: string | undefined;
  } | undefined;
  taskView?: unknown;
  taskOverlaps?: TaskOverlapView[] | undefined;
  nodes: RenderNode[];
  attachments?: Array<{
    id: string;
    kind: string;
    label: string;
    uri?: string | undefined;
    text?: string | undefined;
  }> | undefined;
  sending?: boolean | undefined;
  provider?: string | undefined;
  providerId?: string | undefined;
  providers?: Array<{ id: string; label: string; crabdbBacked?: boolean | undefined }> | undefined;
  acpSessionId?: string | undefined;
  persistedAcpSessionId?: string | undefined;
  acpStartMode?: "new" | "load" | "resume" | undefined;
  requestedAcpSessionId?: string | undefined;
  providerSwitchFrom?: string | undefined;
  providerFailure?: {
    message: string;
    detail?: string | undefined;
    code?: number | null | undefined;
    occurredAt: string;
  } | undefined;
  capabilities?: {
    promptCapabilities?: {
      image?: boolean | undefined;
      audio?: boolean | undefined;
      embeddedContext?: boolean | undefined;
    } | undefined;
  } | undefined;
  permissionPending?: boolean | undefined;
}

interface TaskOverlapView {
  taskId: string;
  lane: string;
  title: string;
  status: string;
  provider?: string | undefined;
  sharedPaths: string[];
  changedPaths: number;
}

const vscode = acquireVsCodeApi();
const app = document.getElementById("app");
const MAX_TEXT_CHARS = 60_000;
const MAX_RAW_JSON_CHARS = 40_000;
const MAX_INLINE_MEDIA_CHARS = 2_000_000;
const MAX_TERMINAL_CHARS = 24_000;
let state: WebviewState = {
  nodes: []
};
let announcement = "";
let composerDraft = "";
let reviewVisible = false;
const restoredState = vscode.getState() as { composerDraft?: string; reviewVisible?: boolean } | undefined;
if (typeof restoredState?.composerDraft === "string") {
  composerDraft = restoredState.composerDraft;
}
if (typeof restoredState?.reviewVisible === "boolean") {
  reviewVisible = restoredState.reviewVisible;
}

window.addEventListener("message", (event: MessageEvent) => {
  const message = event.data as { type: string; [key: string]: unknown };
  if (message.type === "state") {
    state = {
      task: message.task as WebviewState["task"],
      taskView: message.taskView,
      taskOverlaps: Array.isArray(message.taskOverlaps) ? (message.taskOverlaps as TaskOverlapView[]) : [],
      nodes: Array.isArray(message.nodes) ? (message.nodes as RenderNode[]) : [],
      attachments: Array.isArray(message.attachments) ? (message.attachments as WebviewState["attachments"]) : [],
      sending: Boolean(message.sending),
      provider: typeof message.provider === "string" ? message.provider : undefined,
      providerId: typeof message.providerId === "string" ? message.providerId : undefined,
      providers: Array.isArray(message.providers) ? (message.providers as WebviewState["providers"]) : [],
      acpSessionId: typeof message.acpSessionId === "string" ? message.acpSessionId : undefined,
      persistedAcpSessionId: typeof message.persistedAcpSessionId === "string" ? message.persistedAcpSessionId : undefined,
      acpStartMode: isAcpStartMode(message.acpStartMode) ? message.acpStartMode : undefined,
      requestedAcpSessionId: typeof message.requestedAcpSessionId === "string" ? message.requestedAcpSessionId : undefined,
      providerSwitchFrom: typeof message.providerSwitchFrom === "string" ? message.providerSwitchFrom : undefined,
      providerFailure: asProviderFailure(message.providerFailure),
      capabilities: asCapabilityState(message.capabilities),
      permissionPending: Boolean(message.permissionPending)
    };
    if (state.permissionPending) {
      announcement = "Permission request pending.";
    } else if (state.sending) {
      announcement = "Prompt running.";
    }
    persistWebviewState();
    render();
    return;
  }

  if (message.type === "error") {
    announcement = String(message.message || "Unknown error");
    toast(announcement, "error");
    return;
  }

  if (message.type === "status") {
    announcement = String(message.message || "Status updated");
    toast(announcement, "status");
    return;
  }

  if (message.type === "compareTasks") {
    openCompareDrawer(message.result);
    return;
  }

  if (message.type === "conflictDetails") {
    openConflictDrawer(message.result);
    return;
  }

  if (["diff", "applyDryRun", "rewind", "queueMerge", "laneTest", "laneEval"].includes(message.type)) {
    openJsonDrawer(message.type, message.result);
  }
});

document.addEventListener("click", (event) => {
  const target = event.target as HTMLElement | null;
  const action = target?.closest<HTMLElement>("[data-action]");
  if (!action) {
    return;
  }
  const name = action.dataset.action;
  if (!name) {
    return;
  }
  event.preventDefault();

  if (name === "send") {
    sendPrompt();
  } else if (name === "refresh") {
    vscode.postMessage({ type: "refresh" });
  } else if (name === "cancel") {
    vscode.postMessage({ type: "cancel" });
  } else if (name === "dryRunApply") {
    vscode.postMessage({ type: "dryRunApply" });
  } else if (name === "queueMerge") {
    vscode.postMessage({ type: "queueMerge" });
  } else if (name === "openDiff") {
    vscode.postMessage({ type: "openDiff" });
  } else if (name === "compareTasks") {
    vscode.postMessage({ type: "compareTasks" });
  } else if (name === "showConflict") {
    const conflictId = action.dataset.conflictId;
    if (conflictId) {
      vscode.postMessage({ type: "showConflict", conflictId });
    }
  } else if (name === "runTests") {
    vscode.postMessage({ type: "runTests" });
  } else if (name === "runEvals") {
    vscode.postMessage({ type: "runEvals" });
  } else if (name === "openWorkdir") {
    vscode.postMessage({ type: "openWorkdir" });
  } else if (name === "toggleReview") {
    reviewVisible = !reviewVisible;
    persistWebviewState();
    render();
    if (reviewVisible) {
      focusReview();
    }
  } else if (name === "focusReview") {
    focusReview();
  } else if (name === "openSettings") {
    vscode.postMessage({ type: "openSettings" });
  } else if (name === "startFollowUp") {
    vscode.postMessage({ type: "startFollowUp" });
    focusComposer();
  } else if (name === "showAcpLogs") {
    vscode.postMessage({ type: "showAcpLogs" });
  } else if (name === "openNodeDiff") {
    vscode.postMessage({ type: "openNodeDiff", nodeId: action.dataset.nodeId });
  } else if (name === "openTerminal") {
    vscode.postMessage({ type: "openTerminal", nodeId: action.dataset.nodeId });
  } else if (name === "copyTerminalOutput") {
    void copyTerminalOutput(action);
  } else if (name === "copyCode") {
    void copyCode(action);
  } else if (name === "openTextPreview") {
    openTextPreview(action);
  } else if (name === "openMediaPreview") {
    openMediaPreview(action);
  } else if (name === "openLocation") {
    const line = Number(action.dataset.line);
    vscode.postMessage({
      type: "openLocation",
      path: action.dataset.path,
      line: Number.isFinite(line) ? line : undefined
    });
  } else if (name === "openResource") {
    vscode.postMessage({ type: "openResource", uri: action.dataset.uri });
  } else if (name === "rewind") {
    vscode.postMessage({ type: "rewind", target: "before-last-turn" });
  } else if (name === "preserveFailedAttempt") {
    vscode.postMessage({ type: "preserveFailedAttempt" });
  } else if (name === "removeTask") {
    vscode.postMessage({ type: "removeTask" });
  } else if (name === "removeAttachment") {
    vscode.postMessage({ type: "removeAttachment", attachmentId: action.dataset.attachmentId });
  } else if (name === "attachSelection") {
    vscode.postMessage({ type: "attachSelection" });
  } else if (name === "attachFile") {
    vscode.postMessage({ type: "attachFile" });
  } else if (name === "attachDiagnostics") {
    vscode.postMessage({ type: "attachDiagnostics" });
  } else if (name === "attachTerminalOutput") {
    vscode.postMessage({ type: "attachTerminalOutput" });
  } else if (name === "attachChangedFiles") {
    vscode.postMessage({ type: "attachChangedFiles" });
  } else if (name === "attachHistory") {
    vscode.postMessage({ type: "attachHistory" });
  } else if (name === "approve") {
    vscode.postMessage({ type: "approve", requestId: action.dataset.requestId, optionId: action.dataset.optionId });
  } else if (name === "reject") {
    vscode.postMessage({ type: "reject", requestId: action.dataset.requestId });
  } else if (name === "closeDrawer") {
    closeJsonDrawer();
  }
});

document.addEventListener("input", (event) => {
  const target = event.target as HTMLInputElement | HTMLTextAreaElement | null;
  if (target?.classList.contains("composer-input")) {
    composerDraft = target.value;
    persistWebviewState();
  } else if (target?.classList.contains("terminal-search") && target instanceof HTMLInputElement) {
    filterTerminalOutput(target);
  }
});

document.addEventListener("change", (event) => {
  const target = event.target as HTMLSelectElement | null;
  const action = target?.closest<HTMLSelectElement>("[data-action]");
  if (!action) {
    return;
  }
  const name = action.dataset.action;
  if (!name) {
    return;
  }

  if (name === "insertCommand") {
    if (action.value) {
      insertSlashCommand(action.value, action.selectedOptions[0]?.dataset.hint || "");
      action.selectedIndex = 0;
    }
  } else if (name === "setMode") {
    vscode.postMessage({ type: "setMode", modeId: action.value });
  } else if (name === "setConfigOption") {
    vscode.postMessage({ type: "setConfigOption", configId: action.dataset.configId, value: action.value });
  } else if (name === "switchProvider") {
    vscode.postMessage({ type: "switchProvider", providerId: action.value });
  }
});

document.addEventListener("keydown", (event) => {
  if (event.isComposing) {
    return;
  }
  const target = event.target as HTMLElement | null;
  const composerInput = target?.closest<HTMLTextAreaElement>(".composer-input");
  if (
    composerInput &&
    event.key === "Enter" &&
    !event.shiftKey &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey
  ) {
    event.preventDefault();
    sendPrompt();
    return;
  }
  if (event.key === "Escape") {
    closeJsonDrawer();
    return;
  }
  if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
    if (event.key === "1") {
      event.preventDefault();
      focusTranscript();
      return;
    }
    if (event.key === "2") {
      event.preventDefault();
      focusComposer();
      return;
    }
    if (event.key === "3") {
      event.preventDefault();
      focusReview();
      return;
    }
  }
  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
    event.preventDefault();
    sendPrompt();
  }
});

render();
vscode.postMessage({ type: "ready" });

function render(): void {
  if (!app) {
    return;
  }

  const task = state.task;
  const visibleNodes = visibleTimelineNodes();
  const active = document.activeElement as HTMLTextAreaElement | null;
  const composerFocused = Boolean(active?.classList.contains("composer-input"));
  const selectionStart = composerFocused ? active?.selectionStart : undefined;
  const selectionEnd = composerFocused ? active?.selectionEnd : undefined;
  const oldTimeline = document.querySelector<HTMLElement>(".timeline");
  const wasPinnedToBottom = oldTimeline
    ? oldTimeline.scrollHeight - oldTimeline.scrollTop - oldTimeline.clientHeight < 48
    : true;
  const previousScrollTop = oldTimeline?.scrollTop ?? 0;
  app.innerHTML = `
    <section class="shell ${reviewVisible ? "review-open" : ""}">
      <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">${escapeHtml(announcement)}</div>
      ${skipLinks()}
      ${header(task)}
      <section id="timeline" class="timeline" aria-label="Agent transcript" tabindex="-1">
        ${state.providerFailure ? providerFailureBanner(state.providerFailure) : ""}
        ${overlapWarningBanner()}
        ${visibleNodes.length ? visibleNodes.map(renderNode).join("") : emptyTimeline()}
      </section>
      ${composer()}
      ${reviewVisible ? reviewDrawer(task) : ""}
    </section>
  `;
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  if (input) {
    input.value = composerDraft;
    if (composerFocused) {
      input.focus();
      if (selectionStart !== undefined && selectionEnd !== undefined) {
        input.setSelectionRange(selectionStart, selectionEnd);
      }
    }
  }
  const timeline = document.querySelector<HTMLElement>(".timeline");
  if (timeline) {
    timeline.scrollTop = wasPinnedToBottom ? timeline.scrollHeight : previousScrollTop;
  }
}

function skipLinks(): string {
  return `
    <nav class="skip-links" aria-label="Chat landmarks">
      <a href="#timeline">Transcript</a>
      <a href="#composer">Composer</a>
      ${reviewVisible ? `<a href="#review">Review</a>` : `<button data-action="toggleReview">Review</button>`}
      <span aria-hidden="true">Alt+1/2/3</span>
    </nav>
  `;
}

function providerFailureBanner(failure: NonNullable<WebviewState["providerFailure"]>): string {
  const when = failure.occurredAt ? new Date(failure.occurredAt).toLocaleTimeString() : "";
  return `
    <article class="recovery-banner" role="alert" aria-live="assertive">
      <div>
        <div class="card-chrome">
          <span class="role">Agent interrupted</span>
          ${failure.code !== undefined && failure.code !== null ? `<span class="tool-status">exit ${failure.code}</span>` : ""}
          ${when ? `<span class="tool-status">${escapeHtml(when)}</span>` : ""}
        </div>
        <h2>${escapeHtml(failure.message)}</h2>
        <p>Partial transcript and lane changes remain in CrabDB. Review the task or start a follow-up from the latest checkpoint.</p>
        ${failure.detail ? `<p class="muted">${escapeHtml(failure.detail)}</p>` : ""}
      </div>
      <div class="recovery-actions">
        <button data-action="focusReview">Open review</button>
        <button class="primary" data-action="startFollowUp">Start follow-up</button>
        <button data-action="showAcpLogs">Show logs</button>
      </div>
    </article>
  `;
}

function overlapWarningBanner(): string {
  const overlaps = state.taskOverlaps || [];
  if (!overlaps.length) {
    return "";
  }
  const sharedCount = uniqueStrings(overlaps.flatMap((overlap) => overlap.sharedPaths)).length;
  const top = overlaps[0];
  return `
    <article class="overlap-banner" role="status" aria-live="polite">
      <div>
        <div class="card-chrome">
          <span class="role">Parallel work overlap</span>
          <span class="tool-status">${overlaps.length} task${overlaps.length === 1 ? "" : "s"}</span>
          <span class="tool-status">${sharedCount} shared path${sharedCount === 1 ? "" : "s"}</span>
        </div>
        <h2>${escapeHtml(top ? `${top.title} also changes ${top.sharedPaths[0] || "this task's files"}` : "Another task changes the same files")}</h2>
        <p>Compare tasks or refresh CrabDB state before applying this lane.</p>
        <div class="overlap-paths">
          ${overlaps
            .slice(0, 3)
            .map(
              (overlap) => `
                <span>
                  <b>${escapeHtml(shortLabel(overlap.title))}</b>
                  ${escapeHtml(overlap.sharedPaths.slice(0, 3).map(shortLabel).join(", "))}
                </span>
              `
            )
            .join("")}
        </div>
      </div>
      <div class="recovery-actions">
        <button data-action="compareTasks">Compare tasks</button>
        <button data-action="refresh">Refresh</button>
        <button data-action="queueMerge">Queue merge</button>
      </div>
    </article>
  `;
}

function header(task: WebviewState["task"]): string {
  const status = task?.status || "new";
  const changed = task?.changedPaths?.length || 0;
  const usage = state.nodes.find((node) => node.kind === "usage") as Extract<RenderNode, { kind: "usage" }> | undefined;
  const modeLabel = currentModeLabel();
  const configCount = currentConfigOptions().length;
  const sessionState = sessionStateLabel();
  const providerTitle = providerSessionTitle(task?.title);
  const coordination = coordinationSummaryFromSources(task, state.taskView);
  return `
    <header class="chat-header">
      <div class="title-block">
        <div class="eyebrow">
          <span class="provider">${escapeHtml(state.provider || task?.provider || "provider")}</span>
          <span class="status status-${escapeClass(status)}">${escapeHtml(status)}</span>
          ${state.acpSessionId ? `<span class="muted">Session ${escapeHtml(state.acpSessionId)}</span>` : ""}
          ${sessionState ? `<span class="status status-${escapeClass(sessionState.tone)}">${escapeHtml(sessionState.label)}</span>` : ""}
        </div>
        <h1>${escapeHtml(task?.title || "New agent task")}</h1>
        ${providerTitle ? `<p class="provider-title">Provider session: ${escapeHtml(providerTitle)}</p>` : ""}
        <div class="meta-row">
          <span>Lane ${escapeHtml(task?.lane || "pending")}</span>
          <span>${changed} changed path${changed === 1 ? "" : "s"}</span>
          ${modeLabel ? `<span>Mode ${escapeHtml(modeLabel)}</span>` : ""}
          ${configCount ? `<span>${configCount} config option${configCount === 1 ? "" : "s"}</span>` : ""}
          ${coordination.labels.slice(0, 3).map((label) => `<span class="coordination-chip coordination-${escapeClass(coordination.severity)}">${escapeHtml(label)}</span>`).join("")}
          ${task?.nextAction ? `<span>${escapeHtml(task.nextAction)}</span>` : ""}
        </div>
      </div>
      <div class="header-actions">
        ${usage ? contextMeter(usage.used, usage.size) : ""}
        ${iconButton("toggleReview", reviewVisible ? "Hide review" : "Open review", "review", {
          className: reviewVisible ? "active" : "",
          attrs: `aria-pressed="${reviewVisible ? "true" : "false"}"`
        })}
        ${iconButton("refresh", "Refresh task", "refresh")}
        ${iconButton("openDiff", "Open diff", "diff")}
        ${iconButton("openSettings", "Open CrabDB settings", "settings")}
        ${iconButton("dryRunApply", "Dry-run apply", "check", { className: "primary" })}
        ${iconButton("cancel", "Cancel current turn", "stop", { className: "danger", disabled: !state.sending && !state.permissionPending })}
      </div>
    </header>
  `;
}

function contextMeter(used: number, size: number): string {
  const pct = size > 0 ? Math.min(100, Math.round((used / size) * 100)) : 0;
  const tone = pct >= 90 ? "risk" : pct >= 70 ? "review" : "ok";
  return `
    <div class="context-meter" title="${used} / ${size} tokens">
      <span>${pct}%</span>
      <progress class="meter ${tone}" value="${pct}" max="100" aria-label="Context usage ${pct}%"></progress>
    </div>
  `;
}

function emptyTimeline(): string {
  return `
    <article class="empty-state">
      <h2>No transcript yet</h2>
      <p>${escapeHtml(state.provider || "Agent provider")} is ready.</p>
    </article>
  `;
}

function renderNode(node: RenderNode): string {
  switch (node.kind) {
    case "message":
      return messageNode(node);
    case "thought":
      return activityNode("Thinking", "In progress", "thought");
    case "plan":
      return planNode(node);
    case "tool":
      return toolNode(node);
    case "diff":
      return diffNode(node);
    case "terminal":
      return terminalNode(node);
    case "approval":
      return approvalNode(node);
    case "checkpoint":
      return checkpointNode(node);
    case "completion":
      return completionNode(node);
    case "mode":
      return "";
    case "config":
      return "";
    case "commands":
      return "";
    case "session":
      return activityNode("Session updated", node.title || "Metadata changed", "session");
    case "usage":
      return "";
    case "resource":
      return resourceBlock(node.content);
    case "unknown":
      return unknownNode(node);
    default:
      return "";
  }
}

function messageNode(node: Extract<RenderNode, { kind: "message" }>): string {
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card message ${node.role}">
      <div class="rail"></div>
      <div class="card-body">
        <div class="card-chrome">
          <span class="role">${node.role === "user" ? "You" : "Agent"}</span>
          ${node.streaming ? `<span class="streaming">streaming</span>` : ""}
        </div>
        <div class="markdown">${renderContentBlocks(node.content)}</div>
      </div>
    </article>
  `;
}

function planNode(node: Extract<RenderNode, { kind: "plan" }>): string {
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card plan">
      <div class="rail"></div>
      <div class="card-body">
        <div class="card-chrome"><span class="role">Plan</span></div>
        <ol class="plan-list">
          ${node.entries
            .map((entry) => {
              const status = String(entry.status || "pending");
              return `<li class="plan-${escapeClass(status)}"><span>${escapeHtml(status)}</span>${escapeHtml(entry.title || entry.content || "Task")}</li>`;
            })
            .join("")}
        </ol>
      </div>
    </article>
  `;
}

type ToolNodeView = Extract<RenderNode, { kind: "tool" }>;

function toolVisual(kind: ToolNodeView["toolKind"]): {
  label: string;
  description: string;
  icon: IconName;
  tone: "default" | "file" | "change" | "query" | "terminal" | "risk";
} {
  switch (kind) {
    case "read":
      return { label: "Read", description: "Inspecting workspace context", icon: "file", tone: "file" };
    case "edit":
      return { label: "Edit", description: "Changing file content", icon: "changed", tone: "change" };
    case "delete":
      return { label: "Delete", description: "Removing a file or resource", icon: "close", tone: "risk" };
    case "move":
      return { label: "Move", description: "Moving or renaming a path", icon: "changed", tone: "change" };
    case "search":
      return { label: "Search", description: "Finding matches in the workspace", icon: "search", tone: "query" };
    case "execute":
      return { label: "Run", description: "Executing a command", icon: "terminal", tone: "terminal" };
    case "think":
      return { label: "Think", description: "Planning or reasoning", icon: "review", tone: "default" };
    case "fetch":
      return { label: "Fetch", description: "Reading an external or linked resource", icon: "open", tone: "query" };
    case "switch_mode":
      return { label: "Mode", description: "Changing provider session mode", icon: "settings", tone: "default" };
    default:
      return { label: "Tool", description: "Provider tool call", icon: "settings", tone: "default" };
  }
}

function isRiskyTool(node: ToolNodeView): boolean {
  return ["delete", "execute"].includes(node.toolKind) || node.toolStatus === "failed";
}

function toolStatusLabel(status: string): string {
  switch (status) {
    case "in_progress":
      return "running";
    case "completed":
      return "done";
    case "failed":
      return "failed";
    case "pending":
      return "pending";
    default:
      return status;
  }
}

function toolStats(node: ToolNodeView): Array<[string, string]> {
  const diffBlocks = node.content.filter((item) => asRecord(item).type === "diff");
  const terminalBlocks = node.content.filter((item) => asRecord(item).type === "terminal");
  const contentBlocks = node.content.filter((item) => asRecord(item).type === "content");
  const stats: Array<[string, string]> = [];
  if (node.locations.length) {
    stats.push([`location${node.locations.length === 1 ? "" : "s"}`, String(node.locations.length)]);
  }
  if (diffBlocks.length) {
    stats.push([`diff${diffBlocks.length === 1 ? "" : "s"}`, String(diffBlocks.length)]);
  }
  if (terminalBlocks.length) {
    stats.push([`terminal${terminalBlocks.length === 1 ? "" : "s"}`, String(terminalBlocks.length)]);
  }
  if (contentBlocks.length) {
    stats.push([`content block${contentBlocks.length === 1 ? "" : "s"}`, String(contentBlocks.length)]);
  }
  if (!stats.length) {
    stats.push(["state", toolStatusLabel(node.toolStatus)]);
  }
  return stats.slice(0, 4);
}

function toolInputSummary(node: ToolNodeView): string {
  const input = asRecord(node.rawInput);
  if (!Object.keys(input).length) {
    return "";
  }
  const command = terminalCommand(input);
  const facts: Array<[string, string]> = [];
  const path = stringChoice(input, ["path", "file", "filePath", "target", "targetPath"]);
  const from = stringChoice(input, ["from", "oldPath", "source", "sourcePath"]);
  const to = stringChoice(input, ["to", "newPath", "destination", "destinationPath"]);
  const query = stringChoice(input, ["query", "pattern", "regex", "search"]);
  const url = stringChoice(input, ["url", "uri", "href"]);
  const cwd = stringChoice(input, ["cwd", "workingDirectory", "working_directory"]);
  const line = numberChoice(input, ["line", "startLine", "start_line"]);
  if (path) {
    facts.push(["Path", path]);
  }
  if (from || to) {
    facts.push(["Move", [from, to].filter(Boolean).join(" -> ")]);
  }
  if (query) {
    facts.push(["Query", query]);
  }
  if (url) {
    facts.push(["Resource", url]);
  }
  if (command) {
    facts.push(["Command", command]);
  }
  if (cwd) {
    facts.push(["Cwd", cwd]);
  }
  if (typeof line === "number") {
    facts.push(["Line", String(line)]);
  }
  if (!facts.length) {
    return "";
  }
  return `
    <dl class="tool-facts">
      ${facts
        .slice(0, 5)
        .map(([label, value]) => `<div><dt>${escapeHtml(label)}</dt><dd>${escapeHtml(shortLabel(redactString(value)))}</dd></div>`)
        .join("")}
    </dl>
  `;
}

function toolNode(node: Extract<RenderNode, { kind: "tool" }>): string {
  const visual = toolVisual(node.toolKind);
  const risky = isRiskyTool(node);
  const open = risky || node.toolStatus === "in_progress";
  const stats = toolStats(node);
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card tool tool-${escapeClass(node.toolKind)} ${risky ? "risky" : ""}">
      <div class="rail"></div>
      <details class="card-body tool-card tool-tone-${visual.tone}" ${open ? "open" : ""}>
        <summary class="tool-summary">
          <span class="tool-icon">${iconSvg(visual.icon)}</span>
          <span class="tool-summary-main">
            <span class="tool-title">${escapeHtml(node.title || visual.label)}</span>
            <span class="tool-subtitle">${escapeHtml(visual.description)}</span>
          </span>
          <span class="tool-summary-meta">
            <span class="tool-kind">${escapeHtml(visual.label)}</span>
            <span class="tool-status tool-status-${escapeClass(node.toolStatus)}">${escapeHtml(toolStatusLabel(node.toolStatus))}</span>
          </span>
        </summary>
        <div class="tool-overview">
          ${stats.map(([label, value]) => `<span><b>${escapeHtml(value)}</b>${escapeHtml(label)}</span>`).join("")}
        </div>
        ${toolInputSummary(node)}
        ${node.locations.length ? `<div class="chips">${node.locations.map((loc) => locationChip(String(loc.path || ""), typeof loc.line === "number" ? loc.line : undefined)).join("")}</div>` : ""}
        ${node.content.length ? `<div class="tool-content">${node.content.map(renderToolContent).join("")}</div>` : `<p class="muted">No rendered tool output yet.</p>`}
        ${node.rawInput || node.rawOutput ? rawDetails({ input: node.rawInput, output: node.rawOutput }) : ""}
      </details>
    </article>
  `;
}

function diffNode(node: Extract<RenderNode, { kind: "diff" }>): string {
  const stats = diffStats(node.oldText || "", node.newText);
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card diff">
      <div class="rail"></div>
      <details class="card-body diff-card">
        <summary class="tool-summary">
          <span class="tool-icon">${iconSvg("diff")}</span>
          <span class="tool-summary-main">
            <span class="tool-title">${escapeHtml(node.path)}</span>
            <span class="tool-subtitle">${escapeHtml(diffSummaryText(stats))}</span>
          </span>
          <span class="tool-summary-meta">
            <span class="diff-stat additions">+${stats.additions}</span>
            <span class="diff-stat deletions">-${stats.deletions}</span>
          </span>
        </summary>
        <div class="inline-actions">${iconButton("openNodeDiff", "Open native diff", "diff", { attrs: `data-node-id="${escapeHtml(node.id)}"` })}</div>
        ${codeBlock(compactDiff(node.oldText || "", node.newText), { language: "diff", title: node.path })}
      </details>
    </article>
  `;
}

function terminalNode(node: Extract<RenderNode, { kind: "terminal" }>): string {
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card terminal terminal-${escapeClass(node.status)}">
      <div class="rail"></div>
      <details class="card-body" ${node.status === "failed" ? "open" : ""}>
        <summary>
          <span class="tool-kind">terminal</span>
          <span>${escapeHtml(node.title || node.command || node.terminalId)}</span>
          <span class="tool-status">${escapeHtml(node.terminalStatus || node.status)}</span>
        </summary>
        ${terminalPreview(node, node.id)}
      </details>
    </article>
  `;
}

function approvalNode(node: Extract<RenderNode, { kind: "approval" }>): string {
  const resolved = node.status !== "pending";
  const stateLabel = node.status === "completed" ? "approved" : node.status === "cancelled" ? "rejected" : "pending";
  const locations = node.tool.locations || [];
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card approval">
      <div class="rail"></div>
      <div class="card-body">
        <div class="card-chrome">
          <span class="role">Permission required</span>
          <span class="tool-status">${escapeHtml(node.tool.toolKind)}</span>
          <span class="tool-status">${escapeHtml(stateLabel)}</span>
        </div>
        <h2>${escapeHtml(node.title)}</h2>
        <dl class="approval-summary">
          <div><dt>Action</dt><dd>${escapeHtml(node.tool.title)}</dd></div>
          <div><dt>Provider</dt><dd>${escapeHtml(node.provider || state.provider || "provider")}</dd></div>
          <div><dt>Scope</dt><dd>${escapeHtml(locations.length ? `${locations.length} affected location${locations.length === 1 ? "" : "s"}` : node.lane)}</dd></div>
        </dl>
        ${locations.length ? `<div class="chips">${locations.map((loc) => locationChip(String(loc.path || ""), typeof loc.line === "number" ? loc.line : undefined)).join("")}</div>` : ""}
        ${node.tool.content.length ? `<div class="tool-content">${node.tool.content.map(renderToolContent).join("")}</div>` : ""}
        <div class="approval-actions">
          ${node.options
            .map(
              (option, index) =>
                `<button class="${index === 0 ? "primary" : ""}" title="${escapeHtml(option.description || option.label)}" data-action="approve" data-request-id="${escapeHtml(node.requestId)}" data-option-id="${escapeHtml(option.optionId)}" ${resolved ? "disabled" : ""}>${escapeHtml(option.label)}</button>`
            )
            .join("")}
          <button class="danger" data-action="reject" data-request-id="${escapeHtml(node.requestId)}" ${resolved ? "disabled" : ""}>Reject</button>
        </div>
        ${rawDetails(node.raw)}
      </div>
    </article>
  `;
}

function checkpointNode(node: Extract<RenderNode, { kind: "checkpoint" }>): string {
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card checkpoint">
      <div class="rail rail-dot"></div>
      <div class="card-body">
        <div class="card-chrome"><span class="role">Checkpoint</span></div>
        <p>${escapeHtml(node.label)}</p>
      </div>
    </article>
  `;
}

function completionNode(node: Extract<RenderNode, { kind: "completion" }>): string {
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card completion completion-${escapeClass(node.status)}">
      <div class="rail ${node.status === "pending" ? "" : "rail-dot"}"></div>
      <div class="card-body">
        <div class="card-chrome">
          <span class="role">Turn ${escapeHtml(node.status)}</span>
          <span class="tool-status">${escapeHtml(node.stopReason)}</span>
        </div>
        <p>${escapeHtml(node.label)}</p>
        ${node.checkpointPending ? `<p class="muted">Waiting for CrabDB checkpoint confirmation.</p>` : ""}
      </div>
    </article>
  `;
}

function activityNode(title: string, detail: string, kind: string): string {
  return `
    <article class="turn-card activity ${escapeClass(kind)}">
      <div class="rail"></div>
      <div class="card-body compact">
        <span class="role">${escapeHtml(title)}</span>
        <span>${escapeHtml(detail)}</span>
      </div>
    </article>
  `;
}

function unknownNode(node: Extract<RenderNode, { kind: "unknown" }>): string {
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card unknown">
      <div class="rail"></div>
      <details class="card-body">
        <summary>${escapeHtml(node.label)}</summary>
        ${rawDetails(node.payload)}
      </details>
    </article>
  `;
}

function renderContentBlocks(blocks: unknown[]): string {
  return blocks.map((block) => renderContentBlock(block)).join("");
}

function renderContentBlock(block: unknown): string {
  const record = asRecord(block);
  const type = typeof record.type === "string" ? record.type : "unknown";
  if (type === "text") {
    return markdownText(String(record.text || ""));
  }
  if (type === "image") {
    const mime = String(record.mimeType || "image/png");
    const data = String(record.data || "");
    const label = contentLabel(record, "Agent supplied image");
    if (data.length > MAX_INLINE_MEDIA_CHARS) {
      return unsupportedContent(`Image too large to preview inline`, { mimeType: mime, bytes: data.length });
    }
    return mediaPreviewBlock("image", mime, data, label, true);
  }
  if (type === "audio") {
    const mime = String(record.mimeType || "audio/mpeg");
    const data = String(record.data || "");
    const label = contentLabel(record, "Agent supplied audio");
    if (data.length > MAX_INLINE_MEDIA_CHARS) {
      return unsupportedContent(`Audio too large to preview inline`, { mimeType: mime, bytes: data.length });
    }
    return mediaPreviewBlock("audio", mime, data, label, false);
  }
  if (type === "resource_link") {
    const uri = String(record.uri || "");
    const label = String(record.title || record.name || record.uri || "resource");
    const detail = [record.mimeType, typeof record.size === "number" ? `${record.size} bytes` : ""]
      .filter(Boolean)
      .join(" - ");
    return `<button class="resource-chip chip-button" data-action="openResource" data-uri="${escapeHtml(uri)}">${escapeHtml(label)}${detail ? `<small>${escapeHtml(detail)}</small>` : ""}</button>`;
  }
  if (type === "resource") {
    const resource = asRecord(record.resource);
    const uri = String(resource.uri || "");
    const mime = String(resource.mimeType || "text/plain");
    const text = typeof resource.text === "string" ? truncateText(resource.text, MAX_TEXT_CHARS) : undefined;
    const blob = typeof resource.blob === "string" ? resource.blob : undefined;
    const summaryParts = [uri || "embedded resource", mime].filter(Boolean);
    return `<details class="resource"><summary>${escapeHtml(summaryParts.join(" - "))}</summary>${uri ? `<div class="inline-actions"><button data-action="openResource" data-uri="${escapeHtml(uri)}">Open source</button></div>` : ""}${
      text
        ? `<p class="muted">${lineCount(text.text)} lines${text.truncated ? " - preview truncated" : ""}</p>${codeBlock(text.text, { language: languageForResource(uri, mime), title: uri || "Embedded resource" })}${text.truncated ? `<p class="muted">Truncated after ${MAX_TEXT_CHARS} characters.</p>` : ""}`
        : embeddedBinaryResourcePreview(uri, mime, blob)
    }</details>`;
  }
  return unsupportedContent(`Unsupported content: ${type}`, block);
}

function embeddedBinaryResourcePreview(uri: string, mime: string, blob: string | undefined): string {
  if (!blob) {
    return `<p class="muted">Binary resource.</p>`;
  }
  if (blob.length > MAX_INLINE_MEDIA_CHARS) {
    return unsupportedContent(`Binary resource too large to preview inline`, { uri, mimeType: mime, bytes: blob.length });
  }
  if (mime.startsWith("image/")) {
    return mediaPreviewBlock("image", mime, blob, uri || "Embedded image", true);
  }
  if (mime.startsWith("audio/")) {
    return mediaPreviewBlock("audio", mime, blob, uri || "Embedded audio", false);
  }
  return `<p class="muted">Binary resource, ${blob.length} encoded characters.</p>`;
}

function mediaPreviewBlock(kind: "image" | "audio", mime: string, data: string, label: string, open: boolean): string {
  const src = `data:${escapeHtml(mime)};base64,${escapeHtml(data)}`;
  const preview =
    kind === "image"
      ? `<img class="inline-image" src="${src}" alt="${escapeHtml(label)}">`
      : `<audio controls src="${src}"></audio>`;
  return `
    <details class="media-preview" ${open ? "open" : ""}>
      <summary>${escapeHtml(label)}</summary>
      <div class="inline-actions">${iconButton("openMediaPreview", `Open ${kind} preview`, "open")}</div>
      ${preview}
    </details>
  `;
}

function renderToolContent(content: unknown): string {
  const record = asRecord(content);
  const type = typeof record.type === "string" ? record.type : "unknown";
  if (type === "content") {
    return renderContentBlock(record.content);
  }
  if (type === "diff") {
    const path = String(record.path || "Tool diff");
    const oldText = String(record.oldText || "");
    const newText = String(record.newText || "");
    const stats = diffStats(oldText, newText);
    return codeBlock(compactDiff(String(record.oldText || ""), String(record.newText || "")), {
      language: "diff",
      title: `${path} (${diffSummaryText(stats)})`
    });
  }
  if (type === "terminal") {
    return `<div class="terminal-inline">${terminalPreview(record)}</div>`;
  }
  return unsupportedContent(`Unsupported tool content: ${type}`, content);
}

function terminalPreview(value: unknown, nodeId?: string): string {
  const record = asRecord(value);
  const command = terminalCommand(record);
  const cwd = stringChoice(record, ["cwd", "workingDirectory", "working_directory"]);
  const status = stringChoice(record, ["terminalStatus", "status", "state"]);
  const exitCode = numberChoice(record, ["exitCode", "exit_code"]);
  const elapsedMs = numberChoice(record, ["elapsedMs", "elapsed_ms", "durationMs"]);
  const stdout = stringChoice(record, ["stdout", "stdoutPreview", "stdout_preview"]);
  const stderr = stringChoice(record, ["stderr", "stderrPreview", "stderr_preview"]);
  const output = stringChoice(record, ["output"]);
  const meta = [
    command ? `<div><dt>Command</dt><dd><code>${escapeHtml(redactString(command))}</code></dd></div>` : "",
    cwd ? `<div><dt>Cwd</dt><dd>${escapeHtml(cwd)}</dd></div>` : "",
    status ? `<div><dt>Status</dt><dd>${escapeHtml(status)}</dd></div>` : "",
    typeof exitCode === "number" ? `<div><dt>Exit</dt><dd>${exitCode}</dd></div>` : "",
    typeof elapsedMs === "number" ? `<div><dt>Elapsed</dt><dd>${formatDuration(elapsedMs)}</dd></div>` : ""
  ].filter(Boolean);
  const sections = [
    terminalOutputSection("Output", output),
    terminalOutputSection("Stdout", stdout),
    terminalOutputSection("Stderr", stderr)
  ].filter(Boolean);
  return `
    ${nodeId || command ? `<div class="inline-actions">${nodeId ? iconButton("openTerminal", "Open terminal", "terminal", { attrs: `data-node-id="${escapeHtml(nodeId)}"` }) : ""}</div>` : ""}
    ${meta.length ? `<dl class="terminal-meta">${meta.join("")}</dl>` : ""}
    ${sections.length ? `<div class="terminal-output-grid">${sections.join("")}</div>` : `<p class="muted">No terminal output preview is available.</p>`}
  `;
}

function terminalOutputSection(label: string, value: string | undefined): string {
  if (!value) {
    return "";
  }
  const text = truncateText(redactString(value), MAX_TERMINAL_CHARS);
  return `
    <details class="terminal-output" ${label === "Stderr" ? "open" : ""}>
      <summary>${escapeHtml(label)}${text.truncated ? " - truncated" : ""}</summary>
      <div class="terminal-output-tools">
        <input class="terminal-search" type="search" placeholder="Filter ${escapeHtml(label.toLowerCase())}" aria-label="Filter ${escapeHtml(label)}">
        ${iconButton("copyTerminalOutput", `Copy ${label}`, "copy")}
      </div>
      <pre class="code">${escapeHtml(text.text)}</pre>
      ${text.truncated ? `<p class="muted">Preview truncated after ${MAX_TERMINAL_CHARS} characters.</p>` : ""}
    </details>
  `;
}

async function copyTerminalOutput(action: HTMLElement): Promise<void> {
  const pre = action.closest(".terminal-output")?.querySelector<HTMLPreElement>("pre.code");
  const text = pre?.dataset.fullText || pre?.textContent || "";
  if (!text) {
    return;
  }
  try {
    await navigator.clipboard.writeText(text);
    toast("Copied terminal output.", "status");
  } catch {
    fallbackCopy(text, "terminal output");
  }
}

function fallbackCopy(text: string, label = "preview text"): void {
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "true");
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.append(textarea);
  textarea.select();
  const copied = document.execCommand("copy");
  textarea.remove();
  toast(copied ? `Copied ${label}.` : `Unable to copy ${label}.`, copied ? "status" : "error");
}

function filterTerminalOutput(input: HTMLInputElement): void {
  const pre = input.closest(".terminal-output")?.querySelector<HTMLPreElement>("pre.code");
  if (!pre) {
    return;
  }
  const source = pre.dataset.fullText || pre.textContent || "";
  pre.dataset.fullText = source;
  const query = input.value.trim().toLowerCase();
  if (!query) {
    pre.textContent = source;
    return;
  }
  const matches = source.split("\n").filter((line) => line.toLowerCase().includes(query));
  pre.textContent = matches.length ? matches.join("\n") : "No matching lines in this preview.";
}

function terminalCommand(record: Record<string, unknown>): string | undefined {
  const command = record.command;
  if (typeof command === "string") {
    return command;
  }
  if (Array.isArray(command)) {
    return command.map((part) => String(part)).join(" ");
  }
  return stringChoice(record, ["commandLine", "command_line"]);
}

async function copyCode(action: HTMLElement): Promise<void> {
  const text = codeTextForAction(action);
  if (!text) {
    return;
  }
  try {
    await navigator.clipboard.writeText(text);
    toast("Copied preview text.", "status");
  } catch {
    fallbackCopy(text);
  }
}

function openTextPreview(action: HTMLElement): void {
  const text = codeTextForAction(action);
  if (!text) {
    return;
  }
  vscode.postMessage({
    type: "openTextPreview",
    text,
    title: action.dataset.title || "CrabDB preview",
    language: action.dataset.language || "plaintext"
  });
}

function openMediaPreview(action: HTMLElement): void {
  const preview = action.closest(".media-preview");
  const image = preview?.querySelector<HTMLImageElement>("img.inline-image");
  const audio = preview?.querySelector<HTMLAudioElement>("audio");
  const media = image || audio;
  if (!media?.src) {
    toast("No media preview is available.", "status");
    return;
  }
  const label = preview?.querySelector("summary")?.textContent || "Media preview";
  closeJsonDrawer();
  const drawer = document.createElement("section");
  drawer.className = "json-drawer media-drawer";
  drawer.setAttribute("role", "dialog");
  drawer.setAttribute("aria-label", label);
  drawer.setAttribute("tabindex", "-1");
  drawer.innerHTML = `
    <div class="drawer-header">
      <h2>${escapeHtml(label)}</h2>
      ${iconButton("closeDrawer", "Close media preview", "close")}
    </div>
    ${
      image
        ? `<img class="media-full" src="${escapeHtml(media.src)}" alt="${escapeHtml(label)}">`
        : `<audio class="media-full-audio" controls src="${escapeHtml(media.src)}"></audio>`
    }
  `;
  document.body.append(drawer);
  drawer.querySelector<HTMLElement>("[data-action='closeDrawer']")?.focus();
}

function codeTextForAction(action: HTMLElement): string {
  return action.closest(".code-frame")?.querySelector<HTMLElement>(".code")?.textContent || "";
}

function stringChoice(record: Record<string, unknown>, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value) {
      return value;
    }
  }
  return undefined;
}

function numberChoice(record: Record<string, unknown>, keys: string[]): number | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
  }
  return undefined;
}

function resourceBlock(block: unknown): string {
  return `
    <article class="turn-card resource-node">
      <div class="rail"></div>
      <div class="card-body">${renderContentBlock(block)}</div>
    </article>
  `;
}

function composer(): string {
  const attachments = state.attachments || [];
  const controlsDisabled = Boolean(state.sending || state.permissionPending);
  const disabled = controlsDisabled ? "disabled" : "";
  const placeholder = state.permissionPending ? "Permission pending" : "Message agent";
  return `
    <section id="composer" class="composer" aria-label="Prompt composer" tabindex="-1">
      <div class="composer-box">
        <div class="composer-topbar">
          <div class="composer-session">
            <span class="provider-chip">${escapeHtml(state.provider || "provider")}</span>
            ${providerSelector(disabled)}
            ${sessionControlSelectors()}
          </div>
          ${capabilityChips()}
        </div>
        ${
          attachments.length
            ? `<div class="attachments" aria-label="Attached context">${attachments
                .map(
                  (attachment) =>
                    `<span class="attachment-chip"><b>${escapeHtml(attachment.kind)}</b>${escapeHtml(shortLabel(attachment.label))}<small>${escapeHtml(attachmentMode(attachment))}</small>${iconButton("removeAttachment", `Remove ${attachment.label}`, "close", { attrs: `data-attachment-id="${escapeHtml(attachment.id)}"`, className: "micro" })}</span>`
                )
                .join("")}</div>`
            : ""
        }
        <textarea class="composer-input" rows="4" placeholder="${escapeHtml(placeholder)}" aria-keyshortcuts="Enter Control+Enter Meta+Enter" ${disabled}>${escapeHtml(composerDraft)}</textarea>
        <div class="composer-actions">
          <span class="composer-count">${attachments.length} attachment${attachments.length === 1 ? "" : "s"}</span>
          <span class="composer-icon-tools" role="group" aria-label="Context attachments">
            ${iconButton("attachSelection", "Attach the current editor selection", "selection", { disabled: controlsDisabled })}
            ${iconButton("attachFile", "Attach the active file", "file", { disabled: controlsDisabled })}
            ${iconButton("attachDiagnostics", "Attach diagnostics for the active file", "diagnostics", { disabled: controlsDisabled })}
            ${iconButton("attachTerminalOutput", "Attach the latest terminal output from this chat", "terminal", { disabled: controlsDisabled })}
            ${iconButton("attachChangedFiles", "Attach the changed file list for this task", "changed", { disabled: controlsDisabled })}
            ${iconButton("attachHistory", "Attach CrabDB history for the active file", "history", { disabled: controlsDisabled })}
            ${iconButton("rewind", "Rewind latest turn", "rewind")}
            ${iconButton("send", state.permissionPending ? "Permission required before sending" : state.sending ? "Sending prompt" : "Send prompt", "send", {
              className: "primary send-button",
              disabled: controlsDisabled
            })}
          </span>
        </div>
      </div>
    </section>
  `;
}

function reviewDrawer(task: WebviewState["task"]): string {
  const changed = task?.changedPaths || [];
  const taskView = asRecord(state.taskView);
  const review = asRecord(taskView.review);
  const readiness = asRecord(taskView.readiness || review.readiness);
  const turns = arrayField(taskView, "turns");
  const events = arrayField(taskView, "events");
  const changes = arrayField(taskView, "changes").concat(arrayField(review, "changed_paths"));
  const blockers = reviewStrings(readiness, ["blockers", "blocking", "failed_gates"]).concat(
    reviewStrings(review, ["blockers", "blocking", "failed_gates"])
  );
  const warnings = reviewStrings(readiness, ["warnings", "stale_base", "risky_files", "ignored_paths"]).concat(
    reviewStrings(review, ["warnings", "stale_base", "risky_files", "ignored_paths"])
  );
  const testRuns = arrayField(review, "recent_gates").concat(arrayField(review, "latest_test").filter(Boolean));
  const transcriptLinks = transcriptAnchors();
  const coordination = coordinationSummaryFromSources(task, taskView, review, readiness);
  const conflictIds = conflictSetIdsFromSources(readiness, review);
  const overlaps = state.taskOverlaps || [];
  const providerTitle = providerSessionTitle(task?.title);
  return `
    <aside id="review" class="review-drawer" aria-label="Review" tabindex="-1">
      <h2>Review</h2>
      <div class="review-status status-${escapeClass(task?.status || "new")}">${escapeHtml(task?.status || "New task")}</div>
      <section class="review-section">
        <h3>Summary</h3>
        <dl class="review-facts">
          <div><dt>Task</dt><dd>${escapeHtml(task?.title || "New agent task")}</dd></div>
          ${providerTitle ? `<div><dt>Provider session</dt><dd>${escapeHtml(providerTitle)}</dd></div>` : ""}
          <div><dt>Provider</dt><dd>${escapeHtml(state.provider || task?.provider || "provider")}</dd></div>
          ${state.providerSwitchFrom ? `<div><dt>Switched from</dt><dd>${escapeHtml(state.providerSwitchFrom)}</dd></div>` : ""}
          ${task?.model ? `<div><dt>Model</dt><dd>${escapeHtml(task.model)}</dd></div>` : ""}
          <div><dt>Lane</dt><dd>${escapeHtml(task?.lane || "pending")}</dd></div>
          <div><dt>Session</dt><dd>${escapeHtml(sessionStateLabel()?.label || "New session")}</dd></div>
          ${state.persistedAcpSessionId ? `<div><dt>Saved session</dt><dd>${escapeHtml(state.persistedAcpSessionId)}</dd></div>` : ""}
          <div><dt>Turns</dt><dd>${turns.length || transcriptTurnCount()}</dd></div>
          <div><dt>Events</dt><dd>${events.length}</dd></div>
          <div><dt>Changes</dt><dd>${changed.length || changes.length}</dd></div>
        </dl>
      </section>
      <section class="review-section">
        <h3>Readiness</h3>
        ${
          blockers.length || warnings.length
            ? `<ul>${[...blockers.map((item) => `Blocked: ${item}`), ...warnings.map((item) => `Warning: ${item}`)].map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul>`
            : `<p class="muted">${readinessText(task?.status || "new")}</p>`
        }
      </section>
      <section class="review-section">
        <h3>Coordination</h3>
        ${coordinationPanel(coordination)}
      </section>
      ${
        overlaps.length
          ? `<section class="review-section">
              <h3>Parallel work</h3>
              ${overlapReviewList(overlaps)}
              <div class="inline-actions">
                <button data-action="compareTasks">Compare tasks</button>
                <button data-action="refresh">Refresh</button>
                <button data-action="queueMerge">Queue merge</button>
              </div>
            </section>`
          : ""
      }
      ${
        conflictIds.length
          ? `<section class="review-section">
              <h3>Conflicts</h3>
              <p class="muted">CrabDB reports ${conflictIds.length} open conflict set${conflictIds.length === 1 ? "" : "s"} for this task.</p>
              <div class="inline-actions conflict-actions">
                ${conflictIds
                  .map(
                    (id) =>
                      `<button data-action="showConflict" data-conflict-id="${escapeHtml(id)}">Open ${escapeHtml(shortLabel(id))}</button>`
                  )
                  .join("")}
              </div>
            </section>`
          : ""
      }
      <section class="review-section">
        <h3>Tests and evals</h3>
        ${testSummary(taskView, testRuns)}
        <div class="inline-actions">
          <button data-action="runTests">Run test</button>
          <button data-action="runEvals">Run eval</button>
        </div>
      </section>
      <section class="review-section">
        <h3>Diffs</h3>
        ${changed.length || changes.length ? `<ul>${uniqueStrings(changed.concat(changes.map(reviewValueLabel))).map((file) => `<li>${locationChip(file)}</li>`).join("")}</ul>` : `<p class="muted">No changed paths recorded yet.</p>`}
      </section>
      <section class="review-section">
        <h3>Transcript</h3>
        ${transcriptLinks.length ? `<ul>${transcriptLinks.map((link) => `<li><a href="#${escapeHtml(link.id)}">${escapeHtml(link.label)}</a></li>`).join("")}</ul>` : `<p class="muted">No persisted turns yet.</p>`}
      </section>
      <div class="review-actions">
        <button data-action="openDiff">Open diff</button>
        <button data-action="compareTasks">Compare tasks</button>
        <button data-action="openWorkdir">Open workdir</button>
        <button class="primary" data-action="dryRunApply">Dry-run apply</button>
        <button data-action="queueMerge">Queue merge</button>
        <button data-action="rewind">Rewind</button>
        <button data-action="preserveFailedAttempt">Preserve and rewind</button>
        <button class="danger" data-action="removeTask">Remove task</button>
      </div>
    </aside>
  `;
}

function overlapReviewList(overlaps: TaskOverlapView[]): string {
  return `
    <ul class="overlap-list">
      ${overlaps
        .slice(0, 6)
        .map(
          (overlap) => `
            <li>
              <strong>${escapeHtml(overlap.title)}</strong>
              <span>${escapeHtml(overlap.lane)} - ${escapeHtml(overlap.status)}${overlap.provider ? ` - ${escapeHtml(overlap.provider)}` : ""}</span>
              <div class="chips">${overlap.sharedPaths.slice(0, 6).map((path) => locationChip(path)).join("")}</div>
            </li>
          `
        )
        .join("")}
    </ul>
    ${overlaps.length > 6 ? `<p class="muted">Showing 6 of ${overlaps.length} overlapping tasks.</p>` : ""}
  `;
}

function sendPrompt(): void {
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  const text = input?.value || "";
  if (!text.trim()) {
    if (!state.attachments?.length) {
      return;
    }
  }
  if (state.sending) {
    return;
  }
  if (state.permissionPending) {
    toast("Resolve the pending permission request before sending another prompt.", "status");
    return;
  }
  vscode.postMessage({ type: "sendPrompt", text });
  composerDraft = "";
  vscode.setState({ composerDraft });
  if (input) {
    input.value = "";
  }
}

function coordinationPanel(summary: CoordinationSummary): string {
  const cards: Array<[string, string]> = [
    ["Changed", String(summary.changedPaths)],
    ["Conflicts", String(summary.conflicts)],
    ["Approvals", String(summary.pendingApprovals)],
    ["Queue", String(summary.queuedMerges)],
    ["Test", summary.latestTestStatus || "none"],
    ["Eval", summary.latestEvalStatus || "none"]
  ];
  const stale = summary.staleBaseOperations ? `<p class="muted">Lane base is ${summary.staleBaseOperations} operation${summary.staleBaseOperations === 1 ? "" : "s"} behind its target.</p>` : "";
  const dirty = summary.workdirDirty ? `<p class="muted">Lane workdir has unrecorded changes.</p>` : "";
  const issues = summary.issues.slice(0, 8);
  return `
    <div class="coordination-grid">
      ${cards
        .map(
          ([label, value]) => `
            <div class="coordination-card coordination-${escapeClass(summary.severity)}">
              <span>${escapeHtml(label)}</span>
              <strong>${escapeHtml(value)}</strong>
            </div>
          `
        )
        .join("")}
    </div>
    ${summary.labels.length ? `<div class="chips coordination-labels">${summary.labels.map((label) => `<span class="coordination-chip coordination-${escapeClass(summary.severity)}">${escapeHtml(label)}</span>`).join("")}</div>` : ""}
    ${issues.length ? `<ul>${issues.map((issue) => `<li><span class="tool-status">${escapeHtml(issue.tone)}</span> ${escapeHtml(issue.message)}</li>`).join("")}</ul>` : `<p class="muted">No CrabDB coordination blockers reported.</p>`}
    ${stale}${dirty}
  `;
}

function insertSlashCommand(commandName: string, hint: string): void {
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  if (!input) {
    return;
  }
  const prefix = `/${commandName}`;
  const existing = input.value.trim();
  input.value = existing ? `${prefix} ${existing}` : `${prefix}${hint ? " " : ""}`;
  composerDraft = input.value;
  vscode.setState({ composerDraft });
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
}

function focusComposer(): void {
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  input?.focus();
}

function focusTranscript(): void {
  const timeline = document.querySelector<HTMLElement>(".timeline");
  if (!timeline) {
    return;
  }
  timeline.scrollIntoView({ block: "nearest", inline: "nearest" });
  timeline.focus();
}

function focusReview(): void {
  if (!reviewVisible) {
    reviewVisible = true;
    persistWebviewState();
    render();
  }
  const review = document.querySelector<HTMLElement>(".review-drawer");
  if (!review) {
    return;
  }
  review.scrollIntoView({ block: "nearest", inline: "nearest" });
  review.focus();
}

function toast(message: string, tone: "error" | "status"): void {
  const existing = document.querySelector(".toast");
  existing?.remove();
  const node = document.createElement("div");
  node.className = `toast ${tone}`;
  node.setAttribute("role", tone === "error" ? "alert" : "status");
  node.textContent = message;
  document.body.append(node);
  setTimeout(() => node.remove(), 6000);
}

function openJsonDrawer(type: string, result: unknown): void {
  closeJsonDrawer();
  const drawer = document.createElement("section");
  const json = truncateText(redactedJson(result), MAX_RAW_JSON_CHARS);
  drawer.className = "json-drawer";
  drawer.setAttribute("role", "dialog");
  drawer.setAttribute("aria-label", drawerTitle(type));
  drawer.setAttribute("tabindex", "-1");
  drawer.innerHTML = `
    <div class="drawer-header">
      <h2>${escapeHtml(drawerTitle(type))}</h2>
      ${iconButton("closeDrawer", "Close result drawer", "close")}
    </div>
    ${codeBlock(json.text, { language: "json", title: drawerTitle(type), copyLabel: "Copy redacted JSON" })}
    ${json.truncated ? `<p class="muted">Result truncated after ${MAX_RAW_JSON_CHARS} characters.</p>` : ""}
  `;
  document.body.append(drawer);
  drawer.querySelector<HTMLElement>("[data-action='closeDrawer']")?.focus();
}

function openCompareDrawer(result: unknown): void {
  closeJsonDrawer();
  const report = asRecord(result);
  const left = asRecord(report.left);
  const right = asRecord(report.right);
  const leftRisk = asRecord(report.left_risk || report.leftRisk);
  const rightRisk = asRecord(report.right_risk || report.rightRisk);
  const shared = arrayField(report, "shared_paths");
  const leftOnly = arrayField(report, "left_only_paths");
  const rightOnly = arrayField(report, "right_only_paths");
  const recommendation = asRecord(report.recommendation);
  const suggestions = arrayField(report, "suggestions").map(asRecord);
  const drawer = document.createElement("section");
  drawer.className = "json-drawer compare-drawer";
  drawer.setAttribute("role", "dialog");
  drawer.setAttribute("aria-label", "Task compare");
  drawer.setAttribute("tabindex", "-1");
  drawer.innerHTML = `
    <div class="drawer-header">
      <h2>Task compare</h2>
      ${iconButton("closeDrawer", "Close compare drawer", "close")}
    </div>
    ${report.summary ? `<p class="compare-summary">${escapeHtml(String(report.summary))}</p>` : ""}
    <div class="compare-grid">
      ${compareTaskCard("Left", left, leftRisk)}
      ${compareTaskCard("Right", right, rightRisk)}
    </div>
    <section class="compare-section">
      <h3>Overlap</h3>
      <div class="compare-metrics">
        ${compareMetric("Shared", shared.length, "risk")}
        ${compareMetric("Left only", leftOnly.length, "lane")}
        ${compareMetric("Right only", rightOnly.length, "provider")}
      </div>
      ${comparePathList("Shared changed files", shared, "shared")}
      ${comparePathList("Left only", leftOnly, "single")}
      ${comparePathList("Right only", rightOnly, "single")}
    </section>
    ${recommendation.command || recommendation.reason ? `
      <section class="compare-section">
        <h3>Recommendation</h3>
        ${recommendation.command ? `<code class="compare-command">${escapeHtml(String(recommendation.command))}</code>` : ""}
        ${recommendation.reason ? `<p>${escapeHtml(String(recommendation.reason))}</p>` : ""}
      </section>
    ` : ""}
    ${suggestions.length ? `
      <section class="compare-section">
        <h3>Next commands</h3>
        <ul class="compare-suggestions">
          ${suggestions.slice(0, 6).map((suggestion) => `<li>${suggestionText(suggestion)}</li>`).join("")}
        </ul>
      </section>
    ` : ""}
    ${rawDetails(result)}
  `;
  document.body.append(drawer);
  drawer.querySelector<HTMLElement>("[data-action='closeDrawer']")?.focus();
}

function openConflictDrawer(result: unknown): void {
  closeJsonDrawer();
  const report = asRecord(result);
  const explanation = asRecord(report.explanation);
  const merge = asRecord(explanation.merge);
  const id = stringChoice(report, ["conflict_set_id", "conflictSetId", "id"]) || "Conflict set";
  const status = stringChoice(report, ["status"]) || "unknown";
  const source =
    stringChoice(report, ["source_ref", "sourceRef"]) || stringChoice(merge, ["source_ref", "sourceRef", "source"]);
  const target =
    stringChoice(report, ["target_ref", "targetRef"]) || stringChoice(merge, ["target_ref", "targetRef", "target"]);
  const mergeId = stringChoice(report, ["merge_id", "mergeId"]) || stringChoice(merge, ["merge_id", "mergeId", "id"]);
  const createdAt = timestampLabel(report.created_at ?? report.createdAt);
  const paths = arrayField(explanation, "paths").concat(arrayField(report, "paths"));
  const details = arrayField(report, "details").concat(arrayField(explanation, "details"));
  const recommendations = arrayField(explanation, "recommendations").concat(arrayField(report, "recommendations"));
  const nextSteps = arrayField(explanation, "next_steps")
    .concat(arrayField(explanation, "nextSteps"))
    .concat(arrayField(report, "next_steps"))
    .concat(arrayField(report, "nextSteps"));
  const drawer = document.createElement("section");
  drawer.className = "json-drawer conflict-drawer";
  drawer.setAttribute("role", "dialog");
  drawer.setAttribute("aria-label", "Conflict details");
  drawer.setAttribute("tabindex", "-1");
  drawer.innerHTML = `
    <div class="drawer-header">
      <h2>Conflict details</h2>
      ${iconButton("closeDrawer", "Close conflict drawer", "close")}
    </div>
    <div class="conflict-summary">
      <span class="status status-${escapeClass(status)}">${escapeHtml(status)}</span>
      <strong>${escapeHtml(id)}</strong>
    </div>
    <dl class="review-facts conflict-facts">
      ${conflictFact("Source", source)}
      ${conflictFact("Target", target)}
      ${conflictFact("Merge", mergeId)}
      ${conflictFact("Created", createdAt)}
    </dl>
    ${
      details.length
        ? `<section class="conflict-section">
            <h3>Summary</h3>
            <ul class="conflict-list">${details.slice(0, 12).map((detail) => `<li>${escapeHtml(compactValueLabel(detail, "detail"))}</li>`).join("")}</ul>
            ${details.length > 12 ? `<p class="muted">Showing 12 of ${details.length} details.</p>` : ""}
          </section>`
        : ""
    }
    <section class="conflict-section">
      <h3>Affected paths</h3>
      ${
        paths.length
          ? `<div class="conflict-path-list">${paths.slice(0, 20).map(conflictPathCard).join("")}</div>
             ${paths.length > 20 ? `<p class="muted">Showing 20 of ${paths.length} paths.</p>` : ""}`
          : `<p class="muted">No path-level explanation was returned for this conflict set.</p>`
      }
    </section>
    ${recommendations.length ? conflictItemSection("Recommendations", recommendations, "resolution option") : ""}
    ${nextSteps.length ? conflictItemSection("Next steps", nextSteps, "next step") : ""}
    ${rawDetails(result)}
  `;
  document.body.append(drawer);
  drawer.querySelector<HTMLElement>("[data-action='closeDrawer']")?.focus();
}

function closeJsonDrawer(): void {
  document.querySelector(".json-drawer")?.remove();
}

function drawerTitle(type: string): string {
  switch (type) {
    case "applyDryRun":
      return "Dry-run apply";
    case "queueMerge":
      return "Queue merge";
    case "laneTest":
      return "Lane test";
    case "laneEval":
      return "Lane eval";
    case "compareTasks":
      return "Task compare";
    case "conflictDetails":
      return "Conflict details";
    default:
      return type;
  }
}

function conflictFact(label: string, value: string | undefined): string {
  if (!value) {
    return "";
  }
  return `<div><dt>${escapeHtml(label)}</dt><dd>${escapeHtml(value)}</dd></div>`;
}

function conflictPathCard(value: unknown, index: number): string {
  const record = asRecord(value);
  const filePath = stringChoice(record, ["path", "file"]);
  const path = filePath || `Path ${index + 1}`;
  const conflictClass = stringChoice(record, ["conflict_class", "conflictClass", "class", "kind"]);
  const summary = stringChoice(record, ["summary", "message", "title"]);
  const reason = stringChoice(record, ["reason", "why"]);
  const recommendation = stringChoice(record, ["recommendation", "resolution", "next_step", "nextStep"]);
  const signature = stringChoice(record, ["signature"]);
  const lines = arrayField(record, "lines");
  const knownResolutions = arrayField(record, "known_resolutions").concat(arrayField(record, "knownResolutions"));
  return `
    <article class="conflict-path">
      <header>
        ${filePath ? locationChip(filePath) : `<span class="resource-chip">${escapeHtml(path)}</span>`}
        ${conflictClass ? `<span class="conflict-tag">${escapeHtml(conflictClass)}</span>` : ""}
      </header>
      ${summary ? `<p>${escapeHtml(summary)}</p>` : ""}
      ${reason ? `<p class="muted">${escapeHtml(reason)}</p>` : ""}
      <dl class="review-facts conflict-provenance">
        ${conflictProvenance("Target", record.target)}
        ${conflictProvenance("Source", record.source)}
        ${conflictFact("Signature", signature)}
      </dl>
      ${recommendation ? `<p class="conflict-recommendation"><strong>Recommendation</strong>${escapeHtml(recommendation)}</p>` : ""}
      ${lines.length ? conflictItemDetails("Lines", lines, "line") : ""}
      ${knownResolutions.length ? conflictItemDetails("Known resolutions", knownResolutions, "resolution") : ""}
    </article>
  `;
}

function conflictProvenance(label: string, value: unknown): string {
  const record = asRecord(value);
  if (!Object.keys(record).length) {
    return "";
  }
  const text =
    stringChoice(record, ["operation_id", "operationId", "change_id", "changeId", "ref", "lane", "summary", "title"]) ||
    compactValueLabel(value, label.toLowerCase());
  return conflictFact(label, shortLabel(text));
}

function conflictItemSection(title: string, items: unknown[], fallback: string): string {
  return `
    <section class="conflict-section">
      <h3>${escapeHtml(title)}</h3>
      <ul class="conflict-list">
        ${items.slice(0, 12).map((item) => `<li>${conflictItemText(item, fallback)}</li>`).join("")}
      </ul>
      ${items.length > 12 ? `<p class="muted">Showing 12 of ${items.length} items.</p>` : ""}
    </section>
  `;
}

function conflictItemDetails(title: string, items: unknown[], fallback: string): string {
  return `
    <details class="conflict-details">
      <summary>${escapeHtml(title)} (${items.length})</summary>
      <ul class="conflict-list">
        ${items.slice(0, 12).map((item) => `<li>${conflictItemText(item, fallback)}</li>`).join("")}
      </ul>
      ${items.length > 12 ? `<p class="muted">Showing 12 of ${items.length} items.</p>` : ""}
    </details>
  `;
}

function conflictItemText(value: unknown, fallback: string): string {
  const record = asRecord(value);
  const command = stringChoice(record, ["command", "cli", "next_command", "nextCommand"]);
  const reason = stringChoice(record, ["reason", "description", "summary", "message", "title"]);
  if (command || reason) {
    return `${command ? `<code>${escapeHtml(command)}</code>` : ""}${reason ? `<span>${escapeHtml(reason)}</span>` : ""}`;
  }
  return escapeHtml(compactValueLabel(value, fallback));
}

function compactValueLabel(value: unknown, fallback: string): string {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  const record = asRecord(value);
  const label = stringChoice(record, ["summary", "message", "title", "reason", "command", "path", "name"]);
  if (label) {
    return label;
  }
  const json = truncateText(redactedJson(value), 320);
  return json.text || fallback;
}

function timestampLabel(value: unknown): string | undefined {
  if (typeof value === "string" && value) {
    return value;
  }
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return undefined;
  }
  const milliseconds = value > 10_000_000_000 ? value : value * 1000;
  const date = new Date(milliseconds);
  if (Number.isNaN(date.getTime())) {
    return undefined;
  }
  return date.toLocaleString();
}

function compareTaskCard(label: string, task: Record<string, unknown>, risk: Record<string, unknown>): string {
  const title = String(task.title || task.name || task.lane || "Agent task");
  const lane = String(task.lane || "unknown lane");
  const status = String(task.status || "unknown");
  const changed = arrayField(task, "changed_paths").length || arrayField(task, "changedPaths").length;
  const turns = typeof task.turns === "number" ? task.turns : arrayField(task, "turns").length;
  const riskLevel = String(risk.level || "unknown");
  const riskScore = typeof risk.score === "number" ? `${risk.score}/100` : "not scored";
  return `
    <article class="compare-task">
      <div class="card-chrome">
        <span class="role">${escapeHtml(label)}</span>
        <span class="status status-${escapeClass(status)}">${escapeHtml(status)}</span>
      </div>
      <h3>${escapeHtml(title)}</h3>
      <dl class="review-facts">
        <div><dt>Lane</dt><dd>${escapeHtml(lane)}</dd></div>
        <div><dt>Risk</dt><dd>${escapeHtml(riskLevel)} (${escapeHtml(riskScore)})</dd></div>
        <div><dt>Changes</dt><dd>${changed}</dd></div>
        <div><dt>Turns</dt><dd>${turns}</dd></div>
      </dl>
    </article>
  `;
}

function compareMetric(label: string, value: number, tone: string): string {
  return `
    <div class="compare-metric compare-${escapeClass(tone)}">
      <span>${escapeHtml(label)}</span>
      <strong>${value}</strong>
    </div>
  `;
}

function comparePathList(title: string, paths: unknown[], mode: "shared" | "single"): string {
  if (!paths.length) {
    return "";
  }
  return `
    <details class="compare-paths" ${mode === "shared" ? "open" : ""}>
      <summary>${escapeHtml(title)} (${paths.length})</summary>
      <ul>
        ${paths.slice(0, 30).map((path) => comparePathRow(path, mode)).join("")}
      </ul>
      ${paths.length > 30 ? `<p class="muted">Showing 30 of ${paths.length} paths.</p>` : ""}
    </details>
  `;
}

function comparePathRow(value: unknown, mode: "shared" | "single"): string {
  const record = asRecord(value);
  if (mode === "shared") {
    const left = asRecord(record.left);
    const right = asRecord(record.right);
    return `
      <li>
        <span>${escapeHtml(String(record.path || "changed path"))}</span>
        <small>left ${escapeHtml(diffSummaryLabel(left))} / right ${escapeHtml(diffSummaryLabel(right))}</small>
      </li>
    `;
  }
  return `
    <li>
      <span>${escapeHtml(String(record.path || value || "changed path"))}</span>
      <small>${escapeHtml(diffSummaryLabel(record))}</small>
    </li>
  `;
}

function diffSummaryLabel(record: Record<string, unknown>): string {
  const kind = String(record.kind || "changed");
  const additions = typeof record.additions === "number" ? record.additions : 0;
  const deletions = typeof record.deletions === "number" ? record.deletions : 0;
  return `${kind} +${additions} -${deletions}`;
}

function suggestionText(suggestion: Record<string, unknown>): string {
  const command = suggestion.command ? `<code>${escapeHtml(String(suggestion.command))}</code>` : "";
  const reason = suggestion.reason ? `<span>${escapeHtml(String(suggestion.reason))}</span>` : "";
  return [command, reason].filter(Boolean).join(" ");
}

function markdownText(text: string): string {
  const truncated = truncateText(text, MAX_TEXT_CHARS);
  const segments = markdownSegments(truncated.text);
  return (
    segments
      .map((segment) =>
        segment.kind === "code"
          ? codeBlock(segment.text, { language: segment.language || "plaintext", title: "Message code block" })
          : markdownInline(segment.text)
      )
      .join("") +
    (truncated.truncated ? `<p class="muted">Message preview truncated after ${MAX_TEXT_CHARS} characters.</p>` : "")
  );
}

function markdownSegments(text: string): Array<{ kind: "text" | "code"; text: string; language?: string | undefined }> {
  const segments: Array<{ kind: "text" | "code"; text: string; language?: string | undefined }> = [];
  const fence = /```([^\n`]*)\n?([\s\S]*?)```/g;
  let cursor = 0;
  for (const match of text.matchAll(fence)) {
    if (match.index === undefined) {
      continue;
    }
    if (match.index > cursor) {
      segments.push({ kind: "text", text: text.slice(cursor, match.index) });
    }
    segments.push({
      kind: "code",
      language: cleanLanguage(match[1] || "plaintext"),
      text: match[2] || ""
    });
    cursor = match.index + match[0].length;
  }
  if (cursor < text.length) {
    segments.push({ kind: "text", text: text.slice(cursor) });
  }
  return segments.length ? segments : [{ kind: "text", text }];
}

function markdownInline(text: string): string {
  if (!text) {
    return "";
  }
  return escapeHtml(text)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\n/g, "<br>");
}

function diffStats(oldText: string, newText: string): { additions: number; deletions: number; kind: string } {
  const oldLines = oldText ? oldText.split("\n").filter((line, index, lines) => line || index < lines.length - 1).length : 0;
  const newLines = newText ? newText.split("\n").filter((line, index, lines) => line || index < lines.length - 1).length : 0;
  if (!oldText && newText) {
    return { additions: newLines, deletions: 0, kind: "created" };
  }
  if (oldText && !newText) {
    return { additions: 0, deletions: oldLines, kind: "deleted" };
  }
  return { additions: newLines, deletions: oldLines, kind: "changed" };
}

function diffSummaryText(stats: { additions: number; deletions: number; kind: string }): string {
  return `${stats.kind} with ${stats.additions} new line${stats.additions === 1 ? "" : "s"} and ${stats.deletions} old line${stats.deletions === 1 ? "" : "s"}`;
}

function compactDiff(oldText: string, newText: string): string {
  if (!oldText) {
    return `+ ${newText.split("\n").slice(0, 80).join("\n+ ")}`;
  }
  const oldLines = oldText.split("\n");
  const newLines = newText.split("\n");
  const lines = [`- ${oldLines.slice(0, 40).join("\n- ")}`, `+ ${newLines.slice(0, 40).join("\n+ ")}`];
  return lines.join("\n");
}

function rawDetails(value: unknown): string {
  const json = truncateText(redactedJson(value), MAX_RAW_JSON_CHARS);
  return `<details class="raw"><summary>Details</summary>${codeBlock(json.text, { language: "json", title: "Redacted details", copyLabel: "Copy redacted JSON" })}${json.truncated ? `<p class="muted">Details truncated after ${MAX_RAW_JSON_CHARS} characters.</p>` : ""}</details>`;
}

function codeBlock(
  text: string,
  options: { language?: string | undefined; title?: string | undefined; copyLabel?: string | undefined } = {}
): string {
  const language = cleanLanguage(options.language || "plaintext");
  const title = shortLabel(options.title || "Preview");
  const copyLabel = options.copyLabel || "Copy";
  return `
    <div class="code-frame">
      <div class="code-tools">
        <span class="code-title">${escapeHtml(title)}</span>
        <span class="code-language">${escapeHtml(language)}</span>
        ${iconButton("copyCode", copyLabel, "copy")}
        ${iconButton("openTextPreview", "Open preview in editor", "open", {
          attrs: `data-language="${escapeHtml(language)}" data-title="${escapeHtml(title)}"`
        })}
      </div>
      <pre class="code">${escapeHtml(text)}</pre>
    </div>
  `;
}

function cleanLanguage(value: string): string {
  const cleaned = value.trim().replace(/[^a-zA-Z0-9_+.-]/g, "").slice(0, 40);
  return cleaned || "plaintext";
}

function languageForResource(uri: string, mime: string): string {
  const lowerMime = mime.toLowerCase();
  if (lowerMime.includes("json")) {
    return "json";
  }
  if (lowerMime.includes("markdown")) {
    return "markdown";
  }
  if (lowerMime.includes("javascript")) {
    return "javascript";
  }
  if (lowerMime.includes("typescript")) {
    return "typescript";
  }
  if (lowerMime.includes("html")) {
    return "html";
  }
  if (lowerMime.includes("css")) {
    return "css";
  }
  const extension = uri.split(/[?#]/, 1)[0]?.split(".").pop()?.toLowerCase();
  switch (extension) {
    case "md":
      return "markdown";
    case "js":
    case "jsx":
      return "javascript";
    case "ts":
    case "tsx":
      return "typescript";
    case "json":
    case "html":
    case "css":
    case "rust":
    case "go":
    case "python":
    case "yaml":
    case "xml":
      return extension;
    case "rs":
      return "rust";
    case "py":
      return "python";
    case "yml":
      return "yaml";
    default:
      return "plaintext";
  }
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      default:
        return "&#39;";
    }
  });
}

function escapeClass(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9_-]/g, "-");
}

function nodeDomId(value: string): string {
  return `node-${escapeClass(value)}`;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function arrayField(record: Record<string, unknown>, key: string): unknown[] {
  const value = record[key];
  return Array.isArray(value) ? value : [];
}

function reviewStrings(record: Record<string, unknown>, keys: string[]): string[] {
  return keys.flatMap((key) => {
    const value = record[key];
    if (Array.isArray(value)) {
      return value.map(reviewValueLabel).filter(Boolean);
    }
    if (typeof value === "string" && value) {
      return [value];
    }
    if (typeof value === "boolean" && value) {
      return [key.replace(/_/g, " ")];
    }
    return [];
  });
}

function reviewValueLabel(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  const record = asRecord(value);
  return String(record.message || record.title || record.path || record.name || "");
}

function readinessText(status: string): string {
  switch (status) {
    case "ready":
      return "Ready for review. Run a dry-run apply before changing the main worktree.";
    case "blocked":
      return "Blocked. Resolve the listed gate or permission request before applying.";
    case "conflicted":
      return "Conflicted. Compare or rebase the lane before queueing a merge.";
    case "applied":
      return "Applied. The review record is retained by CrabDB.";
    default:
      return "No readiness blockers reported by CrabDB yet.";
  }
}

function transcriptTurnCount(): number {
  return new Set(state.nodes.map((node) => node.turnId).filter(Boolean)).size;
}

function transcriptAnchors(): Array<{ id: string; label: string }> {
  const seen = new Set<string>();
  const links: Array<{ id: string; label: string }> = [];
  for (const node of state.nodes) {
    if (!node.turnId || seen.has(node.turnId)) {
      continue;
    }
    seen.add(node.turnId);
    links.push({
      id: nodeDomId(node.id),
      label: `Turn ${links.length + 1}`
    });
  }
  return links.slice(0, 12);
}

function testSummary(taskView: Record<string, unknown>, extraRuns: unknown[] = []): string {
  const rawTests = [
    ...arrayField(taskView, "tests"),
    ...arrayField(taskView, "evals"),
    ...arrayField(taskView, "evaluations"),
    ...extraRuns
  ];
  if (!rawTests.length) {
    return `<p class="muted">No test or eval runs reported by CrabDB yet.</p>`;
  }
  return `<ul>${rawTests
    .slice(0, 8)
    .map((item) => {
      const record = asRecord(item);
      const command = Array.isArray(record.command) ? record.command.join(" ") : record.command;
      const name = String(record.name || command || record.title || record.kind || "Run");
      const status = String(record.status || record.outcome || "recorded");
      return `<li>${escapeHtml(name)} <span class="tool-status">${escapeHtml(status)}</span></li>`;
    })
    .join("")}</ul>`;
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (!value || seen.has(value)) {
      continue;
    }
    seen.add(value);
    result.push(value);
  }
  return result;
}

function truncateText(value: string, limit: number): { text: string; truncated: boolean } {
  if (value.length <= limit) {
    return { text: value, truncated: false };
  }
  return {
    text: `${value.slice(0, limit)}\n\n[truncated]`,
    truncated: true
  };
}

function lineCount(value: string): number {
  return value ? value.split("\n").length : 0;
}

function formatDuration(ms: number): string {
  if (ms < 1000) {
    return `${Math.round(ms)} ms`;
  }
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s`;
}

function contentLabel(record: Record<string, unknown>, fallback: string): string {
  const annotations = asRecord(record.annotations);
  return String(record.title || record.name || annotations.title || annotations.label || fallback);
}

type IconName =
  | "changed"
  | "check"
  | "close"
  | "copy"
  | "diagnostics"
  | "diff"
  | "file"
  | "history"
  | "open"
  | "refresh"
  | "rewind"
  | "review"
  | "search"
  | "selection"
  | "send"
  | "settings"
  | "stop"
  | "terminal";

function iconButton(
  action: string,
  label: string,
  icon: IconName,
  options: { attrs?: string | undefined; className?: string | undefined; disabled?: boolean | undefined } = {}
): string {
  const classes = ["icon-button", "icon-only", options.className].filter(Boolean).join(" ");
  const attrs = options.attrs ? ` ${options.attrs}` : "";
  const disabled = options.disabled ? " disabled" : "";
  return `<button class="${escapeHtml(classes)}" data-action="${escapeHtml(action)}" title="${escapeHtml(label)}" aria-label="${escapeHtml(label)}"${attrs}${disabled}>${iconSvg(icon)}<span class="sr-only">${escapeHtml(label)}</span></button>`;
}

function iconSvg(icon: IconName): string {
  const open = `<svg class="icon" viewBox="0 0 20 20" aria-hidden="true" focusable="false" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">`;
  switch (icon) {
    case "changed":
      return `${open}<path d="M5 5h7"/><path d="M5 10h10"/><path d="M5 15h5"/><path d="M14 13l2 2 2-2"/><path d="M16 8v7"/></svg>`;
    case "check":
      return `${open}<path d="M4 10l4 4 8-9"/></svg>`;
    case "close":
      return `${open}<path d="M5 5l10 10"/><path d="M15 5L5 15"/></svg>`;
    case "copy":
      return `${open}<rect x="7" y="5" width="9" height="11" rx="1.5"/><path d="M4 13V4a1 1 0 0 1 1-1h8"/></svg>`;
    case "diagnostics":
      return `${open}<path d="M10 3l8 14H2L10 3z"/><path d="M10 8v4"/><path d="M10 15h.01"/></svg>`;
    case "diff":
      return `${open}<path d="M6 4v12"/><path d="M14 4v12"/><path d="M3 8h6"/><path d="M11 12h6"/><path d="M14 9v6"/></svg>`;
    case "file":
      return `${open}<path d="M6 3h6l4 4v10H6z"/><path d="M12 3v5h4"/></svg>`;
    case "history":
      return `${open}<path d="M4 5v4h4"/><path d="M5 9a6 6 0 1 0 2-4"/><path d="M10 7v4l3 2"/></svg>`;
    case "open":
      return `${open}<path d="M7 5h8v8"/><path d="M15 5l-9 9"/><path d="M5 8v7h7"/></svg>`;
    case "refresh":
      return `${open}<path d="M4 7a6 6 0 0 1 10-3l2 2"/><path d="M16 2v4h-4"/><path d="M16 13a6 6 0 0 1-10 3l-2-2"/><path d="M4 18v-4h4"/></svg>`;
    case "rewind":
      return `${open}<path d="M11 6l-6 4 6 4V6z"/><path d="M17 6l-6 4 6 4V6z"/></svg>`;
    case "review":
      return `${open}<path d="M6 4h8"/><path d="M6 8h8"/><path d="M6 12h5"/><path d="M4 4h.01"/><path d="M4 8h.01"/><path d="M4 12h.01"/><path d="M13 15l2 2 3-5"/></svg>`;
    case "search":
      return `${open}<circle cx="9" cy="9" r="5"/><path d="M13 13l4 4"/></svg>`;
    case "selection":
      return `${open}<rect x="4" y="4" width="12" height="12" rx="2" stroke-dasharray="2 2"/><path d="M8 8h4"/><path d="M8 12h2"/></svg>`;
    case "send":
      return `${open}<path d="M3 10l14-7-4 14-3-6-7-1z"/><path d="M10 11l7-8"/></svg>`;
    case "settings":
      return `${open}<circle cx="10" cy="10" r="2.5"/><path d="M10 3v2"/><path d="M10 15v2"/><path d="M4.4 5.2l1.4 1.4"/><path d="M14.2 13.4l1.4 1.4"/><path d="M3 10h2"/><path d="M15 10h2"/><path d="M4.4 14.8l1.4-1.4"/><path d="M14.2 6.6l1.4-1.4"/></svg>`;
    case "stop":
      return `${open}<rect x="5" y="5" width="10" height="10" rx="1.5"/></svg>`;
    case "terminal":
      return `${open}<path d="M4 6l4 4-4 4"/><path d="M10 14h6"/></svg>`;
    default:
      return `${open}<circle cx="10" cy="10" r="6"/></svg>`;
  }
}

function persistWebviewState(): void {
  vscode.setState({ composerDraft, reviewVisible });
}

function unsupportedContent(label: string, detail: unknown): string {
  return `<details class="unsupported"><summary>${escapeHtml(label)}</summary>${rawDetails(detail)}</details>`;
}

function asCapabilityState(value: unknown): WebviewState["capabilities"] {
  const record = asRecord(value);
  const prompt = asRecord(record.promptCapabilities);
  return {
    promptCapabilities: {
      image: prompt.image === true,
      audio: prompt.audio === true,
      embeddedContext: prompt.embeddedContext === true
    }
  };
}

function asProviderFailure(value: unknown): WebviewState["providerFailure"] {
  const record = asRecord(value);
  if (typeof record.message !== "string" || typeof record.occurredAt !== "string") {
    return undefined;
  }
  return {
    message: record.message,
    detail: typeof record.detail === "string" ? record.detail : undefined,
    code: typeof record.code === "number" || record.code === null ? record.code : undefined,
    occurredAt: record.occurredAt
  };
}

function isAcpStartMode(value: unknown): value is NonNullable<WebviewState["acpStartMode"]> {
  return value === "new" || value === "load" || value === "resume";
}

function sessionStateLabel(): { label: string; tone: string } | undefined {
  if (state.providerSwitchFrom && !state.acpStartMode) {
    return { label: "Provider switch follow-up", tone: "dirty" };
  }
  if (state.acpStartMode === "resume") {
    return { label: "Resumed session", tone: "ready" };
  }
  if (state.acpStartMode === "load") {
    return { label: "Loaded session", tone: "ready" };
  }
  if (state.requestedAcpSessionId && state.acpStartMode === "new") {
    return { label: "Checkpoint follow-up", tone: "dirty" };
  }
  if (state.persistedAcpSessionId && !state.acpStartMode) {
    return { label: "Reopenable session", tone: "active" };
  }
  if (state.acpStartMode === "new") {
    return { label: "New session", tone: "active" };
  }
  return undefined;
}

function locationChip(locationPath: string, line?: number): string {
  const label = `${locationPath}${line ? `:${line}` : ""}`;
  return `<button class="resource-chip chip-button" data-action="openLocation" data-path="${escapeHtml(locationPath)}"${line ? ` data-line="${line}"` : ""}>${escapeHtml(label)}</button>`;
}

function visibleTimelineNodes(): RenderNode[] {
  return state.nodes.filter((node) => !["usage", "mode", "config", "commands"].includes(node.kind));
}

function currentConfigOptions(): Array<Record<string, unknown>> {
  const config = [...state.nodes].reverse().find((node) => node.kind === "config") as
    | Extract<RenderNode, { kind: "config" }>
    | undefined;
  return config?.configOptions || [];
}

function currentModeNode(): Extract<RenderNode, { kind: "mode" }> | undefined {
  return [...state.nodes].reverse().find((node) => node.kind === "mode") as
    | Extract<RenderNode, { kind: "mode" }>
    | undefined;
}

function currentSessionNode(): Extract<RenderNode, { kind: "session" }> | undefined {
  return [...state.nodes].reverse().find((node) => node.kind === "session") as
    | Extract<RenderNode, { kind: "session" }>
    | undefined;
}

function providerSessionTitle(taskTitle: string | undefined): string | undefined {
  const title = currentSessionNode()?.title?.trim();
  if (!title) {
    return undefined;
  }
  if (taskTitle && title.toLowerCase() === taskTitle.trim().toLowerCase()) {
    return undefined;
  }
  return title;
}

function currentModeLabel(): string | undefined {
  const modeConfig = currentConfigOptions().find((option) => String(option.category || "") === "mode");
  if (modeConfig) {
    return optionValueLabel(modeConfig, String(modeConfig.currentValue || ""));
  }
  const mode = currentModeNode();
  if (!mode) {
    return undefined;
  }
  const available = mode.availableModes.find((candidate) => candidate.id === mode.modeId);
  return available?.name || mode.modeId;
}

function currentCommands(): Array<Record<string, unknown>> {
  const commands = [...state.nodes].reverse().find((node) => node.kind === "commands") as
    | Extract<RenderNode, { kind: "commands" }>
    | undefined;
  return commands?.availableCommands || [];
}

function sessionControlSelectors(): string {
  const controls = [...configSelectors(), modeSelector(), commandSelector()].filter(Boolean);
  if (!controls.length) {
    return "";
  }
  return `<span class="session-controls" aria-label="Session controls">${controls.join("")}</span>`;
}

function providerSelector(disabled: string): string {
  const providers = state.providers || [];
  if (providers.length <= 1) {
    return "";
  }
  return `
    <label class="select-control provider-control" title="Switching provider starts a new follow-up from CrabDB's current checkpoint.">
      <span>Provider</span>
      <select data-action="switchProvider" aria-label="Provider" ${disabled}>
        ${providers
          .map((provider) => {
            const selected = provider.id === state.providerId ? "selected" : "";
            const suffix = provider.crabdbBacked === false ? " (raw)" : "";
            return `<option value="${escapeHtml(provider.id)}" ${selected}>${escapeHtml(provider.label + suffix)}</option>`;
          })
          .join("")}
      </select>
    </label>
  `;
}

function configSelectors(): string[] {
  return currentConfigOptions()
    .filter((option) => option.type === "select" && Array.isArray(option.options))
    .slice(0, 4)
    .map((option) => {
      const options = (option.options as unknown[]).map(asRecord);
      const currentValue = String(option.currentValue || "");
      return `
        <label class="select-control" title="${escapeHtml(String(option.description || option.name || "Configuration"))}">
          <span>${escapeHtml(String(option.name || option.id))}</span>
          <select data-action="setConfigOption" data-config-id="${escapeHtml(String(option.id))}" aria-label="${escapeHtml(String(option.name || option.id))}">
            ${options
              .map((value) => {
                const optionValue = String(value.value || "");
                const selected = optionValue === currentValue ? "selected" : "";
                return `<option value="${escapeHtml(optionValue)}" ${selected}>${escapeHtml(String(value.name || optionValue))}</option>`;
              })
              .join("")}
          </select>
        </label>
      `;
    });
}

function modeSelector(): string {
  if (currentConfigOptions().some((option) => String(option.category || "") === "mode")) {
    return "";
  }
  const mode = currentModeNode();
  if (!mode || !mode.availableModes.length) {
    return "";
  }
  return `
    <label class="select-control">
      <span>Mode</span>
      <select data-action="setMode" aria-label="Mode">
        ${mode.availableModes
          .map((candidate) => {
            const selected = candidate.id === mode.modeId ? "selected" : "";
            return `<option value="${escapeHtml(candidate.id)}" ${selected}>${escapeHtml(candidate.name || candidate.id)}</option>`;
          })
          .join("")}
      </select>
    </label>
  `;
}

function commandSelector(): string {
  const commands = currentCommands();
  if (!commands.length) {
    return "";
  }
  return `
    <label class="select-control command-control">
      <span>Command</span>
      <select data-action="insertCommand" aria-label="Command">
        <option value="">Slash command</option>
        ${commands
          .map((command) => {
            const input = asRecord(command.input);
            return `<option value="${escapeHtml(String(command.name || ""))}" data-hint="${escapeHtml(String(input.hint || ""))}">/${escapeHtml(String(command.name || ""))}</option>`;
          })
          .join("")}
      </select>
    </label>
  `;
}

function optionValueLabel(option: Record<string, unknown>, value: string): string {
  const values = Array.isArray(option.options) ? option.options.map(asRecord) : [];
  return String(values.find((candidate) => String(candidate.value || "") === value)?.name || value);
}

function capabilityChips(): string {
  const prompt = state.capabilities?.promptCapabilities;
  const chips = [
    { label: "text", enabled: true },
    { label: "links", enabled: true },
    { label: "inline", enabled: prompt?.embeddedContext === true },
    { label: "image", enabled: prompt?.image === true },
    { label: "audio", enabled: prompt?.audio === true }
  ];
  return `<span class="capabilities" aria-label="Prompt capabilities">${chips
    .map(
      (chip) =>
        `<span class="capability-chip ${chip.enabled ? "on" : "off"}" title="${chip.enabled ? "Supported" : "Unavailable"}">${escapeHtml(chip.label)}</span>`
    )
    .join("")}</span>`;
}

function attachmentMode(attachment: NonNullable<WebviewState["attachments"]>[number]): string {
  if (attachment.text !== undefined && attachment.uri) {
    return state.capabilities?.promptCapabilities?.embeddedContext === true ? "inline" : "text";
  }
  if (attachment.uri) {
    return "link";
  }
  return "text";
}

function shortLabel(value: string): string {
  if (value.length <= 96) {
    return value;
  }
  return `${value.slice(0, 45)}...${value.slice(-45)}`;
}

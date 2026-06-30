import {
  CircleAlert,
  Copy,
  Diff as DiffIcon,
  ExternalLink,
  FileDiff,
  FileText,
  FolderGit2,
  History,
  ListTree,
  MessageSquare,
  MessagesSquare,
  PanelRightOpen,
  RefreshCw,
  RotateCcw,
  Search,
  Send,
  Settings,
  SquareCheckBig,
  SquareDashedMousePointer,
  SquareStop,
  Terminal,
  Wrench,
  X
} from "lucide";
import type { ToolCallContent } from "../shared/acpTypes";
import type { RenderNode, RenderPatch, ToolPermissionRequest } from "../shared/renderModel";
import { coordinationSummaryFromSources, type CoordinationSummary } from "../shared/coordinationSummary";
import { conflictSetIdsFromSources } from "../shared/conflicts";
import { redactedJson, redactString } from "../shared/securityRedaction";
import {
  approvalDecisionDescription,
  approvalDecisionTone,
  approvalImpactText,
  approvalActionLabel,
  approvalStateLabel,
  approvalTone,
  type ApprovalDecisionTone
} from "./approvalModel";
import {
  attachmentModeSummary,
  composerDraftState,
  composerMetrics as formatComposerMetrics,
  composerRailItems,
  composerSendBlockedReason as blockedComposerSendReason,
  type ComposerRailItem,
  MAX_COMPOSER_DRAFT_CHARS
} from "./composerModel";
import { textContentValue, textOnlyContent } from "./contentTextModel";
import type { DiffModel, DiffRow, DiffSegment } from "./diffModel";
import type { DiffCardProps } from "./DiffCard";
import { buildEventPresentation, type EventAction, type EventFact, type EventPresentation } from "./eventModel";
import { buildFilePreviewModel, type FilePreviewModel } from "./filePreviewModel";
import { dispatchFloatingMenuClose } from "./floatingMenu";
import {
  buildTimelineConversationGroups,
  buildToolActivitySummary,
  filterTimelineNodes,
  hasTimelineDisplayStructuralChange,
  isInlineToolDiffNode,
  isTimelineFilter,
  sortTimelineNodes,
  TIMELINE_FILTERS,
  timelineDisplayPatchChanges,
  timelineDisplayNodes,
  timelineSearchTokens,
  timelineFilterCounts,
  type TimelineConversationGroup,
  type TimelineFilter
} from "./timelineModel";
import { buildReviewReadiness, type ReviewAction, type ReviewActionGroup } from "./reviewModel";
import {
  buildToolPresentation,
  toolArgumentRecord,
  toolStatusLabel,
  type ToolPresentation
} from "./toolModel";
import {
  buildTerminalPresentation,
  terminalCommand,
  type TerminalPresentation
} from "./terminalModel";
import { buildToolbarModel, type ToolbarAction } from "./toolbarModel";
import type { MessageCardProps } from "./MessageCard";
import type { TimelineScrollerItemView, TimelineScrollerProps } from "./TimelineScroller";
import type {
  ToolCallApprovalAction,
  ToolCallApprovalProps,
  ToolCallCardLocation,
  ToolCallCardProps,
  ToolCallStructuredDetails
} from "./ToolCallCard";
import type { ToolCallGroupCardProps } from "./ToolCallGroupCard";
import { summarizeToolCallGroup } from "./toolCallGroupSummary";
import type { EmptyStateAction, EmptyStateCardProps } from "./EmptyStateCard";
import type { ComposerCardProps } from "./ComposerCard";
import type { EventCardAction, EventCardFact, EventCardProps } from "./EventCard";
import type { HeaderBarProps } from "./HeaderBar";
import type { InlineActionsProps } from "./InlineActions";
import type { PayloadDisclosureProps } from "./PayloadDisclosure";
import type { PlanCardProps } from "./PlanCard";
import type { RawDetailsView } from "./RawDetails";
import {
  changedRenderNodes,
  applyRenderPatchesLocally,
  changedRenderNodesFromPatches,
  isHydratableNodePatchPayload,
  parseBaseRenderRevision,
  parseRenderRevision,
  renderPatchBatchDecision,
  shouldAcceptRenderRevision,
  type RenderPatchChanges
} from "./renderPatchModel";
import type { RecoveryBannerProps } from "./RecoveryBanner";
import type { ResultDrawerProps, ResultDrawerWidget } from "./ResultDrawer";
import type { ReviewDrawerProps } from "./ReviewDrawer";
import type { TerminalCardProps, TerminalTranscriptRow } from "./TerminalCard";
import type { ThoughtCardProps } from "./ThoughtCard";
import type { TimelineGroupCardProps } from "./TimelineGroup";
import type { TimelineNavigationProps } from "./TimelineNavigation";

declare const acquireVsCodeApi: () => {
  postMessage(message: unknown): void;
  getState(): unknown;
  setState(state: unknown): void;
};

interface WebviewState {
  renderRevision?: number | undefined;
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
  providers?:
    | Array<{
        id: string;
        label: string;
        crabdbBacked?: boolean | undefined;
        supportsFromRef?: boolean | undefined;
      }>
    | undefined;
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

type PromptAttachmentView = NonNullable<WebviewState["attachments"]>[number];

interface ComposerStatus {
  tone: "ready" | "context" | "running" | "waiting" | "warning";
  label: string;
  detail: string;
}

interface PendingDiffPreview {
  id: string;
  path: string;
  oldText: string;
  newText: string;
  compact?: boolean | undefined;
  patch?: string | undefined;
  additions?: number | undefined;
  deletions?: number | undefined;
  nodeId?: string | undefined;
  title?: string | undefined;
}

type StreamingMarkdownTarget = HTMLElement & {
  __crabdbQueueStreamdownText?: ((text: string) => void) | undefined;
  __crabdbStreamingText?: string | undefined;
}

const vscode = acquireVsCodeApi();
const app = document.getElementById("app");
const MAX_TEXT_CHARS = 60_000;
const MAX_RAW_JSON_CHARS = 40_000;
const MAX_INLINE_MEDIA_CHARS = 2_000_000;
const MAX_TERMINAL_CHARS = 24_000;
const FLOATING_DETAILS_SELECTOR = ".composer-controls,.header-details,.toolbar-capabilities,.timeline-toolbar";
const DRAWER_FOCUSABLE_SELECTOR =
  'a[href],button:not([disabled]),textarea:not([disabled]),input:not([disabled]),select:not([disabled]),audio[controls],video[controls],[tabindex]:not([tabindex="-1"])';
const STREAM_RENDER_INTERVAL_MS = 80;
const CRABDB_STREAMDOWN_UPDATE_EVENT = "crabdb-streamdown-update";

installFrameBatchedResizeObserver();

let highlightModulePromise: Promise<typeof import("./highlight.js")> | undefined;
let diffModelModulePromise: Promise<typeof import("./diffModel.js")> | undefined;
let diffEnhancerModulePromise: Promise<typeof import("./diffEnhancer.js")> | undefined;
let diffReviewDrawerModulePromise: Promise<typeof import("./diffReviewDrawer.js")> | undefined;
let diffEnhancerModule: typeof import("./diffEnhancer.js") | undefined;
let markdownModulePromise: Promise<typeof import("./markdownModel.js")> | undefined;
let markdownModule: typeof import("./markdownModel.js") | undefined;
let composerCardModulePromise: Promise<typeof import("./ComposerCard.js")> | undefined;
let diffCardModulePromise: Promise<typeof import("./DiffCard.js")> | undefined;
let emptyStateCardModulePromise: Promise<typeof import("./EmptyStateCard.js")> | undefined;
let eventCardModulePromise: Promise<typeof import("./EventCard.js")> | undefined;
let headerBarModulePromise: Promise<typeof import("./HeaderBar.js")> | undefined;
let inlineActionsModulePromise: Promise<typeof import("./InlineActions.js")> | undefined;
let messageCardModulePromise: Promise<typeof import("./MessageCard.js")> | undefined;
let payloadDisclosureModulePromise: Promise<typeof import("./PayloadDisclosure.js")> | undefined;
let planCardModulePromise: Promise<typeof import("./PlanCard.js")> | undefined;
let recoveryBannerModulePromise: Promise<typeof import("./RecoveryBanner.js")> | undefined;
let resultDrawerModulePromise: Promise<typeof import("./ResultDrawer.js")> | undefined;
let resultDrawerModule: typeof import("./ResultDrawer.js") | undefined;
let reviewDrawerModulePromise: Promise<typeof import("./ReviewDrawer.js")> | undefined;
let terminalCardModulePromise: Promise<typeof import("./TerminalCard.js")> | undefined;
let thoughtCardModulePromise: Promise<typeof import("./ThoughtCard.js")> | undefined;
let timelineGroupModulePromise: Promise<typeof import("./TimelineGroup.js")> | undefined;
let timelineNavigationModulePromise: Promise<typeof import("./TimelineNavigation.js")> | undefined;
let timelineScrollerModulePromise: Promise<typeof import("./TimelineScroller.js")> | undefined;
let toolCallCardModulePromise: Promise<typeof import("./ToolCallCard.js")> | undefined;
let toolCallGroupCardModulePromise: Promise<typeof import("./ToolCallGroupCard.js")> | undefined;
let diffPreviewCounter = 0;
let diffRenderEpoch = 0;
let renderEpoch = 0;
let renderTimeoutHandle: number | undefined;
let renderAnimationFrameHandle: number | undefined;
let renderScheduled = false;
let lastRenderAt = 0;
let patchedNodeHydrationFrameHandle: number | undefined;
let timelineStructureHydrationFrameHandle: number | undefined;
let chromeHydrationFrameHandle: number | undefined;
const pendingPatchedNodeIds = new Set<string>();
let pendingPatchedNodeBottomLock = false;
let pendingTimelineStructureChromeHydration = false;
let timelineBottomLockContent: Element | undefined;
let timelineBottomLockCleanup: (() => void) | undefined;
let timelineBottomLockFrameHandle: number | undefined;
let timelineBottomLockObserver: ResizeObserver | undefined;
let timelineBottomLockPinned = true;
let timelineBottomLockTimeline: HTMLElement | undefined;
let timelineBottomLockUserPauseUntil = 0;
let pendingDiffPreviews: PendingDiffPreview[] = [];
let composerCardProps: ComposerCardProps | undefined;
let diffCardProps = new Map<string, DiffCardProps>();
let emptyStateCardProps = new Map<string, EmptyStateCardProps>();
let eventCardProps = new Map<string, EventCardProps>();
let headerBarProps: HeaderBarProps | undefined;
let inlineActionsProps = new Map<string, InlineActionsProps>();
let messageCardProps = new Map<string, MessageCardProps>();
let lastUserMessageNodeId: string | undefined;
let payloadDisclosureProps = new Map<string, PayloadDisclosureProps>();
let planCardProps = new Map<string, PlanCardProps>();
let recoveryBannerProps = new Map<string, RecoveryBannerProps>();
let reviewDrawerProps: ReviewDrawerProps | undefined;
let terminalCardProps = new Map<string, TerminalCardProps>();
let thoughtCardProps = new Map<string, ThoughtCardProps>();
let timelineGroupProps = new Map<string, TimelineGroupCardProps>();
let timelineNavigationProps: TimelineNavigationProps | undefined;
let timelineScrollerProps: TimelineScrollerProps | undefined;
let toolCallCardProps = new Map<string, ToolCallCardProps>();
let toolCallGroupCardProps = new Map<string, ToolCallGroupCardProps>();
let state: WebviewState = {
  nodes: []
};
let latestRenderRevision = 0;
let announcement = "";
let composerDraft = "";
let pendingTimelineSearchFocus = false;
let payloadDisclosureCounter = 0;
let inlineActionsCounter = 0;
type ComposerSendMode = "fast" | "draft";
const COMPOSER_SEND_MODES = new Set<ComposerSendMode>(["fast", "draft"]);
const HIDDEN = new Set<RenderNode["kind"]>(["commands", "config", "mode", "session", "usage"]);
const COMPOSER_PROMPT_PRESETS: Array<{ id: string; label: string; detail: string; text: string; icon: IconName }> = [
  {
    id: "implement",
    label: "Implement",
    detail: "Start a focused code change",
    text: "Implement this change. First inspect the relevant files, then update the code and focused tests.",
    icon: "tool"
  },
  {
    id: "review",
    label: "Review",
    detail: "Look for bugs and gaps",
    text: "Review the current changes for bugs, regressions, risky edge cases, and missing tests.",
    icon: "review"
  },
  {
    id: "test",
    label: "Test",
    detail: "Run and fix focused tests",
    text: "Run the focused tests for this change. If anything fails, diagnose it and fix the issue.",
    icon: "check"
  },
  {
    id: "explain",
    label: "Explain",
    detail: "Summarize the current implementation",
    text: "Explain how this part works, what changed recently, and the safest next step.",
    icon: "message"
  }
];
let composerSendMode: ComposerSendMode = "fast";
let reviewVisible = false;
let timelineFilter: TimelineFilter = "all";
let timelineQuery = "";
let drawerRestoreFocus: HTMLElement | undefined;
let renderResyncRequested = false;
let lastPersistedWebviewStateJson = "";
const restoredState = vscode.getState() as
  | { composerDraft?: string; composerSendMode?: unknown; reviewVisible?: boolean; timelineFilter?: unknown; timelineQuery?: unknown }
  | undefined;
if (typeof restoredState?.composerDraft === "string") {
  composerDraft = restoredState.composerDraft;
}
if (isComposerSendMode(restoredState?.composerSendMode)) {
  composerSendMode = restoredState.composerSendMode;
}
if (typeof restoredState?.reviewVisible === "boolean") {
  reviewVisible = restoredState.reviewVisible;
}
if (isTimelineFilter(restoredState?.timelineFilter)) {
  timelineFilter = restoredState.timelineFilter;
}
if (typeof restoredState?.timelineQuery === "string") {
  timelineQuery = restoredState.timelineQuery;
}

interface FrameBatchedResizeObserverConstructor {
  new (callback: ResizeObserverCallback): ResizeObserver;
  __crabdbFrameBatched?: true | undefined;
}

function installFrameBatchedResizeObserver(): void {
  if (typeof ResizeObserver === "undefined") {
    return;
  }
  const NativeResizeObserver = window.ResizeObserver as FrameBatchedResizeObserverConstructor;
  if (NativeResizeObserver.__crabdbFrameBatched) {
    return;
  }
  class FrameBatchedResizeObserver implements ResizeObserver {
    private frameHandle: number | undefined;
    private readonly nativeObserver: ResizeObserver;
    private readonly pendingEntries = new Map<Element, ResizeObserverEntry>();

    constructor(callback: ResizeObserverCallback) {
      this.nativeObserver = new NativeResizeObserver((entries) => {
        for (const entry of entries) {
          this.pendingEntries.set(entry.target, entry);
        }
        if (this.frameHandle !== undefined) {
          return;
        }
        this.frameHandle = window.requestAnimationFrame(() => {
          this.frameHandle = undefined;
          const deliveredEntries = [...this.pendingEntries.values()];
          this.pendingEntries.clear();
          if (deliveredEntries.length) {
            callback(deliveredEntries, this);
          }
        });
      });
    }

    observe(target: Element, options?: ResizeObserverOptions): void {
      this.nativeObserver.observe(target, options);
    }

    unobserve(target: Element): void {
      this.nativeObserver.unobserve(target);
      this.pendingEntries.delete(target);
    }

    disconnect(): void {
      if (this.frameHandle !== undefined) {
        window.cancelAnimationFrame(this.frameHandle);
        this.frameHandle = undefined;
      }
      this.pendingEntries.clear();
      this.nativeObserver.disconnect();
    }
  }
  (FrameBatchedResizeObserver as FrameBatchedResizeObserverConstructor).__crabdbFrameBatched = true;
  window.ResizeObserver = FrameBatchedResizeObserver as unknown as typeof ResizeObserver;
}

window.addEventListener("message", (event: MessageEvent) => {
  const message = event.data as { type: string; [key: string]: unknown };
  if (message.type === "state") {
    const previousState = state;
    const previousChromeSignature = chromeStateSignature(previousState);
    const previousTimelineSignature = timelineFrameStateSignature(previousState);
    const beforeNodes = state.nodes;
    const renderRevision = parseRenderRevision(message.renderRevision);
    if (!shouldAcceptRenderRevision(renderRevision, latestRenderRevision)) {
      return;
    }
    if (renderRevision !== undefined) {
      latestRenderRevision = renderRevision;
    }
    state = {
      renderRevision,
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
    const changes = changedRenderNodes(new Map(beforeNodes.map((node) => [node.id, node])), state.nodes);
    if (state.permissionPending) {
      announcement = "Permission request pending.";
    } else if (state.sending) {
      announcement = "Prompt running.";
    }
    persistWebviewState();
    routeRenderChanges({
      beforeNodes,
      changes,
      chromeStateChanged: previousChromeSignature !== chromeStateSignature(state),
      timelineFrameStateChanged: previousTimelineSignature !== timelineFrameStateSignature(state)
    });
    return;
  }

  if (message.type === "renderPatches") {
    applyRenderPatchMessage(message);
    return;
  }

  if (message.type === "error") {
    announceToast(String(message.message || "Unknown error"), "error");
    return;
  }

  if (message.type === "status") {
    announceToast(String(message.message || "Status updated"), "status");
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

  if (message.type === "diff") {
    void openDiffReviewDrawer(message.result);
    return;
  }

  if (["applyDryRun", "rewind", "queueMerge", "laneTest", "laneEval"].includes(message.type)) {
    openJsonDrawer(message.type, message.result);
  }
});

document.addEventListener("click", (event) => {
  const target = event.target as HTMLElement | null;
  const activeFloatingDetails = target?.closest<HTMLElement>(FLOATING_DETAILS_SELECTOR);
  closeFloatingDetails(activeFloatingDetails || undefined);
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
  } else if (name === "setTimelineFilter") {
    if (isTimelineFilter(action.dataset.timelineFilter)) {
      timelineFilter = action.dataset.timelineFilter;
      persistWebviewState();
      render();
    }
  } else if (name === "clearTimelineSearch") {
    clearTimelineSearch(true);
  } else if (name === "focusReview") {
    focusReview();
  } else if (name === "focusTranscript") {
    focusTranscript();
  } else if (name === "focusComposer") {
    focusComposer();
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
  } else if (name === "focusToolDiff") {
    focusToolDiff(action);
  } else if (name === "copyTimelineGroupId") {
    void copyTimelineGroupId(action);
  } else if (name === "copyCheckpoint") {
    void copyCheckpoint(action);
  } else if (name === "copyCode") {
    void copyCode(action);
  } else if (name === "copyDiff") {
    void copyDiff(action);
  } else if (name === "openTextPreview") {
    openTextPreview(action);
  } else if (name === "openDiffPreview") {
    openDiffPreview(action);
  } else if (name === "selectDiffReviewFile") {
    selectDiffReviewFile(action.dataset.path || "");
  } else if (name === "insertDiffSuggestion") {
    insertDiffSuggestion(action);
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
    vscode.postMessage({ type: "rewind", target: action.dataset.target || "before-last-turn" });
  } else if (name === "preserveFailedAttempt") {
    vscode.postMessage({ type: "preserveFailedAttempt" });
  } else if (name === "removeTask") {
    vscode.postMessage({ type: "removeTask" });
  } else if (name === "removeAttachment") {
    vscode.postMessage({ type: "removeAttachment", attachmentId: action.dataset.attachmentId });
  } else if (name === "insertPromptPreset") {
    insertPromptPreset(action.dataset.presetId || "");
  } else if (name === "clearComposerDraft") {
    clearComposerDraft();
  } else if (name === "setComposerSendMode") {
    setComposerSendMode(action.dataset.sendMode);
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
  if (target instanceof HTMLTextAreaElement && target.classList.contains("composer-input")) {
    composerDraft = target.value;
    persistWebviewState();
    resizeComposerInput(target);
    syncComposerAffordances();
  } else if (target instanceof HTMLInputElement && target.classList.contains("timeline-search-input")) {
    timelineQuery = target.value;
    persistWebviewState();
    render();
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
  if (handleJsonDrawerKeydown(event)) {
    return;
  }
  if (event.key === "Escape" && closeFloatingDetails(undefined, true)) {
    event.preventDefault();
    return;
  }
  const composerInput = target?.closest<HTMLTextAreaElement>(".composer-input");
  if (
    composerInput &&
    event.key === "Enter" &&
    !event.shiftKey &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    composerSendMode === "fast"
  ) {
    event.preventDefault();
    sendPrompt();
    return;
  }
  const timelineSearchInput = target?.closest<HTMLInputElement>(".timeline-search-input");
  if (timelineSearchInput && event.key === "Escape" && (timelineQuery || timelineFilter !== "all")) {
    event.preventDefault();
    clearTimelineSearch(true);
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

interface RenderFocusSnapshot {
  composerFocused: boolean;
  timelineSearchFocused: boolean;
  selectionStart?: number | null | undefined;
  selectionEnd?: number | null | undefined;
  searchSelectionStart?: number | null | undefined;
  searchSelectionEnd?: number | null | undefined;
  hadTimeline: boolean;
  wasPinnedToBottom: boolean;
  previousScrollTop: number;
}

interface RenderPass {
  renderEpoch: number;
  diffEpoch: number;
}

interface PatchedTimelineHydrationTargets {
  groupIds: Set<string>;
  inlineActionIds: Set<string>;
  nodeIds: Set<string>;
  payloadDisclosureIds: Set<string>;
  toolGroupIds: Set<string>;
  needsDiffHydration: boolean;
}

function scheduleRender(): void {
  cancelPatchedNodeHydration();
  cancelTimelineStructureHydration();
  cancelChromeHydration();
  if (renderScheduled) {
    return;
  }
  renderScheduled = true;
  const elapsed = Date.now() - lastRenderAt;
  const delay = Math.max(0, STREAM_RENDER_INTERVAL_MS - elapsed);
  renderTimeoutHandle = window.setTimeout(() => {
    renderTimeoutHandle = undefined;
    renderAnimationFrameHandle = window.requestAnimationFrame(() => {
      renderAnimationFrameHandle = undefined;
      renderStateUpdate();
    });
  }, delay);
}

function clearScheduledRender(): void {
  if (renderTimeoutHandle !== undefined) {
    window.clearTimeout(renderTimeoutHandle);
    renderTimeoutHandle = undefined;
  }
  if (renderAnimationFrameHandle !== undefined) {
    window.cancelAnimationFrame(renderAnimationFrameHandle);
    renderAnimationFrameHandle = undefined;
  }
  renderScheduled = false;
}

function applyRenderPatchMessage(message: { [key: string]: unknown }): void {
  const renderRevision = parseRenderRevision(message.renderRevision);
  const baseRenderRevision = parseBaseRenderRevision(message.baseRenderRevision);
  const decision = renderPatchBatchDecision(baseRenderRevision, renderRevision, latestRenderRevision);
  if (decision === "drop") {
    return;
  }
  if (decision === "resync") {
    requestRenderStateResync();
    return;
  }
  if (renderRevision === undefined) {
    return;
  }
  const patches = Array.isArray(message.patches) ? (message.patches as RenderPatch[]) : [];
  const previousState = state;
  const previousChromeSignature = chromeStateSignature(previousState);
  const previousTimelineSignature = timelineFrameStateSignature(previousState);
  const beforeNodes = state.nodes;
  const nextNodes = applyRenderPatchesLocally(beforeNodes, patches);
  const changes = changedRenderNodesFromPatches(beforeNodes, patches);
  state = stateWithRenderPatchMetadata(message, renderRevision, nextNodes);
  latestRenderRevision = renderRevision;
  if (state.permissionPending) {
    announcement = "Permission request pending.";
  } else if (state.sending) {
    announcement = "Prompt running.";
  }
  routeRenderChanges({
    beforeNodes,
    changes,
    patches,
    chromeStateChanged: previousChromeSignature !== chromeStateSignature(state),
    timelineFrameStateChanged: previousTimelineSignature !== timelineFrameStateSignature(state)
  });
}

function requestRenderStateResync(): void {
  if (renderResyncRequested) {
    return;
  }
  renderResyncRequested = true;
  vscode.postMessage({ type: "refresh" });
  window.setTimeout(() => {
    renderResyncRequested = false;
  }, 250);
}

function stateWithRenderPatchMetadata(
  message: { [key: string]: unknown },
  renderRevision: number,
  nodes: RenderNode[]
): WebviewState {
  return {
    ...state,
    renderRevision,
    nodes,
    task: messageHasField(message, "task") ? (message.task as WebviewState["task"]) : state.task,
    taskView: messageHasField(message, "taskView") ? message.taskView : state.taskView,
    taskOverlaps: messageHasField(message, "taskOverlaps")
      ? Array.isArray(message.taskOverlaps)
        ? (message.taskOverlaps as TaskOverlapView[])
        : []
      : state.taskOverlaps,
    attachments: messageHasField(message, "attachments")
      ? Array.isArray(message.attachments)
        ? (message.attachments as WebviewState["attachments"])
        : []
      : state.attachments,
    sending: typeof message.sending === "boolean" ? message.sending : state.sending,
    provider: typeof message.provider === "string" ? message.provider : state.provider,
    providerId: typeof message.providerId === "string" ? message.providerId : state.providerId,
    providers: messageHasField(message, "providers")
      ? Array.isArray(message.providers)
        ? (message.providers as WebviewState["providers"])
        : []
      : state.providers,
    acpSessionId: typeof message.acpSessionId === "string" ? message.acpSessionId : state.acpSessionId,
    persistedAcpSessionId:
      typeof message.persistedAcpSessionId === "string" ? message.persistedAcpSessionId : state.persistedAcpSessionId,
    acpStartMode: isAcpStartMode(message.acpStartMode) ? message.acpStartMode : state.acpStartMode,
    requestedAcpSessionId:
      typeof message.requestedAcpSessionId === "string" ? message.requestedAcpSessionId : state.requestedAcpSessionId,
    providerSwitchFrom:
      typeof message.providerSwitchFrom === "string" ? message.providerSwitchFrom : state.providerSwitchFrom,
    providerFailure: messageHasField(message, "providerFailure") ? asProviderFailure(message.providerFailure) : state.providerFailure,
    capabilities: messageHasField(message, "capabilities") ? asCapabilityState(message.capabilities) : state.capabilities,
    permissionPending: typeof message.permissionPending === "boolean" ? message.permissionPending : state.permissionPending
  };
}

function messageHasField(message: { [key: string]: unknown }, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(message, key);
}

function routeRenderChanges({
  beforeNodes,
  changes,
  chromeStateChanged,
  patches,
  timelineFrameStateChanged
}: {
  beforeNodes: RenderNode[];
  changes: RenderPatchChanges;
  chromeStateChanged: boolean;
  patches?: RenderPatch[] | undefined;
  timelineFrameStateChanged: boolean;
}): void {
  const visibleChanges = timelineDisplayPatchChanges(beforeNodes, state.nodes, changes);
  const hiddenStateChanged = hasRenderPatchChange(beforeNodes, state.nodes, changes, isHiddenChromeNode);
  const needsChromeHydration = chromeStateChanged || hiddenStateChanged;
  const localPayloads = patches ? patches.every(isLocallyHydratablePatchPayload) : true;

  if (!existingShellCanHydrate() || renderScheduled) {
    scheduleRender();
    return;
  }
  if (timelineSearchTokens(timelineQuery).length) {
    scheduleRender();
    return;
  }
  if (!localPayloads) {
    scheduleRender();
    return;
  }

  if (timelineFrameStateChanged) {
    scheduleTimelineStructureHydration({ includeChrome: needsChromeHydration });
    return;
  }
  if (hasAnyPatchChanges(visibleChanges)) {
    if (hasTimelineDisplayStructuralChange(beforeNodes, state.nodes, visibleChanges)) {
      scheduleTimelineStructureHydration({ includeChrome: needsChromeHydration });
      return;
    }
    if (canHydratePatchedNodes(visibleChanges)) {
      const immediateStreamingNodeIds = streamingTextDomPatchableNodeIds(visibleChanges);
      const keepBottomForDeferredNodes =
        Boolean(immediateStreamingNodeIds.size) &&
        shouldKeepTimelinePinnedToBottom(document.querySelector<HTMLElement>(".timeline"));
      if (immediateStreamingNodeIds.size) {
        applyStreamingTextDomPatchesImmediately(immediateStreamingNodeIds);
      }
      const deferredNodeIds = changedNodeIdsWithout(visibleChanges.changedNodeIds, immediateStreamingNodeIds);
      if (deferredNodeIds.size) {
        schedulePatchedNodeHydration(deferredNodeIds, { lockBottom: keepBottomForDeferredNodes });
      }
      if (needsChromeHydration) {
        scheduleChromeHydration();
      }
      return;
    }
    if (canHydrateTimelineStructure(visibleChanges)) {
      scheduleTimelineStructureHydration({ includeChrome: needsChromeHydration });
      return;
    }
    scheduleRender();
    return;
  }
  if (needsChromeHydration) {
    scheduleChromeHydration();
    return;
  }
}

function filterRenderPatchChanges(
  beforeNodes: RenderNode[],
  nextNodes: RenderNode[],
  changes: RenderPatchChanges,
  predicate: (node: RenderNode) => boolean
): RenderPatchChanges {
  const beforeById = new Map(beforeNodes.map((node) => [node.id, node]));
  const nextById = new Map(nextNodes.map((node) => [node.id, node]));
  const keep = (id: string) => {
    const node = nextById.get(id) || beforeById.get(id);
    return node ? predicate(node) : true;
  };
  return {
    changedNodeIds: new Set([...changes.changedNodeIds].filter(keep)),
    addedNodeIds: new Set([...changes.addedNodeIds].filter(keep)),
    removedNodeIds: new Set([...changes.removedNodeIds].filter(keep))
  };
}

function hasRenderPatchChange(
  beforeNodes: RenderNode[],
  nextNodes: RenderNode[],
  changes: RenderPatchChanges,
  predicate: (node: RenderNode) => boolean
): boolean {
  return hasAnyPatchChanges(filterRenderPatchChanges(beforeNodes, nextNodes, changes, predicate));
}

function hasAnyPatchChanges(changes: RenderPatchChanges): boolean {
  return Boolean(changes.changedNodeIds.size || changes.addedNodeIds.size || changes.removedNodeIds.size);
}

function isHiddenChromeNode(node: RenderNode): boolean {
  return HIDDEN.has(node.kind);
}

function isLocallyHydratablePatchPayload(patch: RenderPatch): boolean {
  return patch.type === "remove" || isHydratableNodePatchPayload(patch) || Boolean(patch.node && isHiddenChromeNode(patch.node));
}

function chromeStateSignature(snapshot: WebviewState): string {
  return JSON.stringify({
    task: snapshot.task,
    taskView: snapshot.taskView,
    attachments: snapshot.attachments,
    sending: snapshot.sending,
    provider: snapshot.provider,
    providerId: snapshot.providerId,
    providers: snapshot.providers,
    acpSessionId: snapshot.acpSessionId,
    persistedAcpSessionId: snapshot.persistedAcpSessionId,
    acpStartMode: snapshot.acpStartMode,
    requestedAcpSessionId: snapshot.requestedAcpSessionId,
    providerSwitchFrom: snapshot.providerSwitchFrom,
    capabilities: snapshot.capabilities,
    permissionPending: snapshot.permissionPending,
    hiddenNodes: snapshot.nodes.filter(isHiddenChromeNode)
  });
}

function timelineFrameStateSignature(snapshot: WebviewState): string {
  return JSON.stringify({
    providerFailure: snapshot.providerFailure,
    taskOverlaps: snapshot.taskOverlaps
  });
}

function canHydratePatchedNodes(
  changes: RenderPatchChanges
): boolean {
  if (!existingShellCanHydrate() || renderScheduled || state.providerFailure) {
    return false;
  }
  if (timelineSearchTokens(timelineQuery).length) {
    return false;
  }
  if (changes.addedNodeIds.size || changes.removedNodeIds.size) {
    return false;
  }
  return [...changes.changedNodeIds].every((id) => {
    const node = state.nodes.find((candidate) => candidate.id === id);
    return Boolean(node && isPatchedTimelineCardNode(node));
  });
}

function isPatchedTimelineCardNode(node: RenderNode): boolean {
  return (
    node.kind === "message" ||
    node.kind === "thought" ||
    node.kind === "plan" ||
    node.kind === "tool" ||
    node.kind === "diff" ||
    node.kind === "terminal" ||
    node.kind === "approval" ||
    node.kind === "checkpoint" ||
    node.kind === "completion" ||
    node.kind === "resource" ||
    node.kind === "unknown"
  );
}

function canApplyStreamingTextDomPatchesImmediately(changes: RenderPatchChanges): boolean {
  const ids = streamingTextDomPatchableNodeIds(changes);
  return ids.size > 0 && ids.size === changes.changedNodeIds.size;
}

function streamingTextDomPatchableNodeIds(changes: RenderPatchChanges): Set<string> {
  const ids = new Set<string>();
  if (changes.addedNodeIds.size || changes.removedNodeIds.size) {
    return ids;
  }
  if (!changes.changedNodeIds.size) {
    return ids;
  }
  for (const id of changes.changedNodeIds) {
    const node = state.nodes.find((candidate) => candidate.id === id);
    if (!node || (node.kind !== "message" && node.kind !== "thought") || streamingTextForNode(node) === undefined) {
      continue;
    }
    const article = document.getElementById(nodeDomId(id));
    if (article?.querySelector("[data-streaming-markdown]")) {
      ids.add(id);
    }
  }
  return ids;
}

function changedNodeIdsWithout(nodeIds: Set<string>, excludedNodeIds: Set<string>): Set<string> {
  return new Set([...nodeIds].filter((id) => !excludedNodeIds.has(id)));
}

function applyStreamingTextDomPatchesImmediately(nodeIds: Set<string>): void {
  const timeline = document.querySelector<HTMLElement>(".timeline");
  const restoreBottom = shouldKeepTimelinePinnedToBottom(timeline);
  const fallbackNodeIds = new Set<string>();
  for (const nodeId of nodeIds) {
    const node = state.nodes.find((candidate) => candidate.id === nodeId);
    if (!node || !applyStreamingTextDomPatch(node)) {
      fallbackNodeIds.add(nodeId);
    }
  }
  if (restoreBottom && timeline?.isConnected) {
    lockTimelineToBottom(timeline);
    queueTimelineBottomRestore();
  }
  if (fallbackNodeIds.size) {
    schedulePatchedNodeHydration(fallbackNodeIds, { lockBottom: restoreBottom });
  }
}

function canHydrateTimelineStructure(
  changes: RenderPatchChanges
): boolean {
  if (!existingShellCanHydrate() || renderScheduled || state.providerFailure) {
    return false;
  }
  if (timelineSearchTokens(timelineQuery).length) {
    return false;
  }
  if (!changes.addedNodeIds.size && !changes.removedNodeIds.size) {
    return false;
  }
  return true;
}

function schedulePatchedNodeHydration(
  nodeIds: Set<string>,
  options: { lockBottom?: boolean | undefined } = {}
): void {
  for (const nodeId of nodeIds) {
    pendingPatchedNodeIds.add(nodeId);
  }
  pendingPatchedNodeBottomLock ||= Boolean(options.lockBottom);
  if (patchedNodeHydrationFrameHandle !== undefined) {
    return;
  }
  patchedNodeHydrationFrameHandle = window.requestAnimationFrame(() => {
    patchedNodeHydrationFrameHandle = undefined;
    void hydratePatchedNodes();
  });
}

function scheduleTimelineStructureHydration(options: { includeChrome?: boolean | undefined } = {}): void {
  pendingTimelineStructureChromeHydration ||= Boolean(options.includeChrome);
  if (timelineStructureHydrationFrameHandle !== undefined) {
    return;
  }
  timelineStructureHydrationFrameHandle = window.requestAnimationFrame(() => {
    timelineStructureHydrationFrameHandle = undefined;
    void hydrateTimelineStructure();
  });
}

function scheduleChromeHydration(): void {
  if (chromeHydrationFrameHandle !== undefined) {
    return;
  }
  chromeHydrationFrameHandle = window.requestAnimationFrame(() => {
    chromeHydrationFrameHandle = undefined;
    void hydrateChromeState();
  });
}

async function hydrateTimelineStructure(): Promise<void> {
  if (!existingShellCanHydrate()) {
    scheduleRender();
    return;
  }
  const includeChrome = pendingTimelineStructureChromeHydration;
  pendingTimelineStructureChromeHydration = false;
  const focus = captureRenderFocus();
  const visibleNodes = visibleTimelineNodes();
  const pass = prepareRenderProps(visibleNodes);
  timelineNavigation(visibleNodes);
  header(state.task);
  if (includeChrome) {
    composer();
    if (reviewVisible) {
      reviewDrawer(state.task);
    }
  }
  syncLiveAnnouncement();
  const hydrationTasks: Array<Promise<void>> = [
    hydrateHeaderBars(),
    hydrateTimelineNavigation().then(() => {
      restoreTimelineSearchInput({
        searchFocused: focus.timelineSearchFocused || pendingTimelineSearchFocus,
        selectionStart: focus.searchSelectionStart,
        selectionEnd: focus.searchSelectionEnd
      });
      pendingTimelineSearchFocus = false;
    })
  ];
  if (includeChrome) {
    hydrationTasks.push(
      hydrateComposerCards().then(() => {
        restoreComposerInput({
          composerFocused: focus.composerFocused,
          selectionStart: focus.selectionStart,
          selectionEnd: focus.selectionEnd
        });
      })
    );
    if (reviewVisible) {
      hydrationTasks.push(hydrateReviewDrawers());
    }
  }
  await Promise.all(hydrationTasks);
  if (!isCurrentRender(pass.renderEpoch)) {
    return;
  }
  await hydrateTimelineScroller();
  if (!isCurrentRender(pass.renderEpoch)) {
    return;
  }
  restoreTimelineScrollFromFocus(focus);
  await hydrateReactIslands();
  if (!isCurrentRender(pass.renderEpoch)) {
    return;
  }
  restoreTimelineBottomAfterIslandHydration(focus);
  window.requestAnimationFrame(() => {
    if (!isCurrentRender(pass.renderEpoch)) {
      return;
    }
    void highlightCodeBlocks();
    void hydrateDiffPreviews(pass.diffEpoch);
  });
}

async function hydrateChromeState(): Promise<void> {
  if (!existingShellCanHydrate()) {
    scheduleRender();
    return;
  }
  const focus = captureRenderFocus();
  const visibleNodes = visibleTimelineNodes();
  const pass = prepareRenderProps(visibleNodes);
  timelineNavigation(visibleNodes);
  header(state.task);
  composer();
  if (reviewVisible) {
    reviewDrawer(state.task);
  }
  syncLiveAnnouncement();
  await Promise.all([
    hydrateHeaderBars(),
    hydrateTimelineNavigation().then(() => {
      restoreTimelineSearchInput({
        searchFocused: focus.timelineSearchFocused || pendingTimelineSearchFocus,
        selectionStart: focus.searchSelectionStart,
        selectionEnd: focus.searchSelectionEnd
      });
      pendingTimelineSearchFocus = false;
    }),
    hydrateComposerCards().then(() => {
      restoreComposerInput({
        composerFocused: focus.composerFocused,
        selectionStart: focus.selectionStart,
        selectionEnd: focus.selectionEnd
      });
    }),
    reviewVisible ? hydrateReviewDrawers() : Promise.resolve()
  ]);
  if (!isCurrentRender(pass.renderEpoch)) {
    return;
  }
  syncComposerAffordances();
}

async function hydratePatchedNodes(): Promise<void> {
  if (!pendingPatchedNodeIds.size) {
    return;
  }
  if (!existingShellCanHydrate()) {
    pendingPatchedNodeIds.clear();
    pendingPatchedNodeBottomLock = false;
    scheduleRender();
    return;
  }
  const nodeIds = [...pendingPatchedNodeIds];
  pendingPatchedNodeIds.clear();
  const forcePatchedBottomLock = pendingPatchedNodeBottomLock;
  pendingPatchedNodeBottomLock = false;
  const visibleNodes = visibleTimelineNodes();
  const visibleById = new Map(visibleNodes.map((node) => [node.id, node]));
  let needsFullRender = false;
  let needsDiffHydration = false;
  let needsIslandHydration = false;
  let needsTimelineChromeHydration = false;
  let streamedTextDomPatchApplied = false;
  const directTargets = emptyPatchedTimelineHydrationTargets();
  const presentationRefreshNodeIds: string[] = [];
  const timeline = document.querySelector<HTMLElement>(".timeline");
  const restorePatchedBottom =
    (forcePatchedBottomLock && Date.now() >= timelineBottomLockUserPauseUntil) ||
    shouldKeepTimelinePinnedToBottom(timeline);
  for (const nodeId of nodeIds) {
    const node = visibleById.get(nodeId);
    if (!node) {
      if (state.nodes.some((candidate) => candidate.id === nodeId)) {
        needsTimelineChromeHydration = true;
      }
      continue;
    }
    if (applyStreamingTextDomPatch(node)) {
      streamedTextDomPatchApplied = true;
      continue;
    }
    if (isPatchedTimelineCardNode(node)) {
      if (hydratePatchedNodeIslandDirectly(node, directTargets)) {
        needsDiffHydration ||= directTargets.needsDiffHydration;
        needsIslandHydration = true;
        continue;
      }
      presentationRefreshNodeIds.push(nodeId);
      needsIslandHydration = true;
    } else {
      needsFullRender = true;
    }
  }
  if (needsFullRender) {
    scheduleRender();
    return;
  }
  if (streamedTextDomPatchApplied && restorePatchedBottom && timeline?.isConnected) {
    lockTimelineToBottom(timeline);
    queueTimelineBottomRestore();
  }
  if (needsTimelineChromeHydration) {
    await hydrateTimelineChromeForPatchedNodes(visibleNodes);
  }
  if (!needsIslandHydration) {
    return;
  }
  if (hasPatchedTimelineHydrationTargets(directTargets)) {
    await hydratePatchedTimelineIslands(directTargets);
  }
  if (presentationRefreshNodeIds.length) {
    const targets = refreshTimelineGroupsForPatchedNodes(presentationRefreshNodeIds);
    needsDiffHydration ||= targets.needsDiffHydration;
    await hydratePatchedTimelineIslands(targets);
  }
  if (restorePatchedBottom && timeline?.isConnected) {
    lockTimelineToBottom(timeline);
    queueTimelineBottomRestore();
  }
  window.requestAnimationFrame(() => {
    void highlightCodeBlocks();
    if (needsDiffHydration) {
      void hydrateDiffPreviews(++diffRenderEpoch).then(() => hydrateInlineActions());
    }
  });
}

async function hydrateTimelineChromeForPatchedNodes(visibleNodes: RenderNode[]): Promise<void> {
  timelineNavigation(visibleNodes);
  header(state.task);
  await Promise.all([hydrateHeaderBars(), hydrateTimelineNavigation()]);
}

function applyStreamingTextDomPatch(node: RenderNode): boolean {
  if (node.kind !== "message" && node.kind !== "thought") {
    return false;
  }
  const streamText = streamingTextForNode(node);
  if (streamText === undefined) {
    return false;
  }
  const article = document.getElementById(nodeDomId(node.id));
  const streamTarget = article?.querySelector<HTMLElement>("[data-streaming-markdown]");
  if (!streamTarget) {
    return false;
  }
  renderNode(node);
  updateStreamingTextTarget(streamTarget, streamText);
  return true;
}

function updateStreamingTextTarget(streamTarget: HTMLElement, streamText: string): void {
  const streamdownTarget = streamTarget as StreamingMarkdownTarget;
  if (streamTarget.dataset.streamdownMarkdown !== undefined) {
    if (streamdownTarget.__crabdbStreamingText === streamText) {
      return;
    }
    streamdownTarget.__crabdbStreamingText = streamText;
    if (streamdownTarget.__crabdbQueueStreamdownText) {
      streamdownTarget.__crabdbQueueStreamdownText(streamText);
      return;
    }
    streamTarget.dispatchEvent(new CustomEvent(CRABDB_STREAMDOWN_UPDATE_EVENT, { detail: { text: streamText } }));
    return;
  }
  const current = streamTarget.textContent || "";
  if (current === streamText) {
    return;
  }
  const firstChild = streamTarget.firstChild;
  if (
    firstChild &&
    firstChild.nodeType === Node.TEXT_NODE &&
    streamTarget.childNodes.length === 1 &&
    streamText.startsWith(current)
  ) {
    (firstChild as Text).appendData(streamText.slice(current.length));
    return;
  }
  streamTarget.textContent = streamText;
}

function emptyPatchedTimelineHydrationTargets(): PatchedTimelineHydrationTargets {
  return {
    groupIds: new Set<string>(),
    inlineActionIds: new Set<string>(),
    nodeIds: new Set<string>(),
    payloadDisclosureIds: new Set<string>(),
    toolGroupIds: new Set<string>(),
    needsDiffHydration: false
  };
}

function hasPatchedTimelineHydrationTargets(targets: PatchedTimelineHydrationTargets): boolean {
  return Boolean(
    targets.groupIds.size ||
      targets.inlineActionIds.size ||
      targets.nodeIds.size ||
      targets.payloadDisclosureIds.size ||
      targets.toolGroupIds.size ||
      targets.needsDiffHydration
  );
}

function hydratePatchedNodeIslandDirectly(
  node: RenderNode,
  targets: PatchedTimelineHydrationTargets
): boolean {
  if (!canHydrateMountedNodeIslandDirectly(node)) {
    return false;
  }
  const previousDiffPreviewCount = pendingDiffPreviews.length;
  const html = renderNode(node);
  if (!syncNodeShellFromHtml(node, html)) {
    return false;
  }
  targets.nodeIds.add(node.id);
  collectHelperIslandIdsFromHtml(html, targets);
  targets.needsDiffHydration ||= pendingDiffPreviews.length > previousDiffPreviewCount;
  return true;
}

function canHydrateMountedNodeIslandDirectly(node: RenderNode): boolean {
  if (!isLiveStatus(node.status) || !canHydrateDirectNodeKind(node)) {
    return false;
  }
  const article = document.getElementById(nodeDomId(node.id));
  if (!article?.isConnected) {
    return false;
  }
  return Boolean(mountedNodeIslandRoot(article, node));
}

function canHydrateDirectNodeKind(node: RenderNode): boolean {
  return isPatchedTimelineCardNode(node);
}

function mountedNodeIslandRoot(article: HTMLElement, node: RenderNode): Element | null {
  switch (node.kind) {
    case "message":
      return article.querySelector("[data-message-card-root]");
    case "thought":
      return article.querySelector("[data-thought-card-root]");
    case "plan":
      return article.querySelector("[data-plan-card-root]");
    case "tool":
    case "approval":
      return article.querySelector("[data-tool-call-card-root]");
    case "diff":
      return article.querySelector("[data-diff-card-root]");
    case "terminal":
      return article.querySelector("[data-terminal-card-root]");
    case "checkpoint":
    case "completion":
    case "resource":
    case "unknown":
      return article.querySelector("[data-event-card-root]");
    default:
      return null;
  }
}

function syncNodeShellFromHtml(node: RenderNode, html: string): boolean {
  const current = document.getElementById(nodeDomId(node.id));
  const next = htmlElement(html);
  if (!current || !next || current.tagName !== next.tagName || current.id !== next.id) {
    return false;
  }
  syncElementAttributes(current, next);
  return true;
}

function syncElementAttributes(current: Element, next: Element): void {
  for (const attr of Array.from(current.attributes)) {
    if (!next.hasAttribute(attr.name)) {
      current.removeAttribute(attr.name);
    }
  }
  for (const attr of Array.from(next.attributes)) {
    if (current.getAttribute(attr.name) !== attr.value) {
      current.setAttribute(attr.name, attr.value);
    }
  }
}

function isTimelinePinnedToBottom(timeline: HTMLElement | null): boolean {
  if (!timeline) {
    return true;
  }
  return timeline.scrollHeight - timeline.scrollTop - timeline.clientHeight < 48;
}

function shouldKeepTimelinePinnedToBottom(timeline: HTMLElement | null): boolean {
  if (isTimelinePinnedToBottom(timeline)) {
    return true;
  }
  return timelineBottomLockPinned && Date.now() >= timelineBottomLockUserPauseUntil;
}

function restoreTimelineScrollFromFocus(focus: RenderFocusSnapshot): void {
  const timeline = document.querySelector<HTMLElement>(".timeline");
  if (!timeline) {
    setTimelineBottomLockPinned(focus.wasPinnedToBottom);
    return;
  }
  installTimelineBottomLock();
  if (!focus.hadTimeline) {
    setTimelineBottomLockPinned(true);
    return;
  }
  if (focus.wasPinnedToBottom) {
    lockTimelineToBottom(timeline);
    queueTimelineBottomRestore();
    return;
  }
  timeline.scrollTop = focus.previousScrollTop;
  setTimelineBottomLockPinned(false);
}

function restoreTimelineBottomAfterIslandHydration(focus: RenderFocusSnapshot): void {
  if (!focus.wasPinnedToBottom) {
    setTimelineBottomLockPinned(false);
    return;
  }
  const timeline = document.querySelector<HTMLElement>(".timeline");
  if (timeline) {
    lockTimelineToBottom(timeline);
    queueTimelineBottomRestore();
  }
}

function installTimelineBottomLock(): void {
  const timeline = document.querySelector<HTMLElement>(".timeline");
  const content = document.querySelector<HTMLElement>(".timeline-scroller-content");
  if (!timeline || !content || typeof ResizeObserver === "undefined") {
    cleanupTimelineBottomLock();
    return;
  }
  if (timelineBottomLockTimeline === timeline && timelineBottomLockContent === content) {
    return;
  }
  cleanupTimelineBottomLock();
  timelineBottomLockTimeline = timeline;
  timelineBottomLockContent = content;
  timelineBottomLockPinned = isTimelinePinnedToBottom(timeline);

  const markUserPause = (): void => {
    timelineBottomLockUserPauseUntil = Date.now() + 300;
  };
  const onWheel = (event: WheelEvent): void => {
    if (event.deltaY < 0) {
      markUserPause();
      timelineBottomLockPinned = false;
    }
  };
  const onPointerDown = (): void => {
    markUserPause();
  };
  const onKeyDown = (event: KeyboardEvent): void => {
    if (["ArrowUp", "PageUp", "Home"].includes(event.key)) {
      markUserPause();
      timelineBottomLockPinned = false;
    }
  };
  const onScroll = (): void => {
    if (isTimelinePinnedToBottom(timeline)) {
      timelineBottomLockPinned = true;
      return;
    }
    if (Date.now() < timelineBottomLockUserPauseUntil) {
      timelineBottomLockPinned = false;
    }
  };

  timeline.addEventListener("wheel", onWheel, { passive: true });
  timeline.addEventListener("pointerdown", onPointerDown, { passive: true });
  timeline.addEventListener("touchstart", onPointerDown, { passive: true });
  timeline.addEventListener("keydown", onKeyDown);
  timeline.addEventListener("scroll", onScroll, { passive: true });
  timelineBottomLockObserver = new ResizeObserver(() => {
    queueTimelineBottomRestore();
  });
  timelineBottomLockObserver.observe(content);
  timelineBottomLockCleanup = () => {
    timeline.removeEventListener("wheel", onWheel);
    timeline.removeEventListener("pointerdown", onPointerDown);
    timeline.removeEventListener("touchstart", onPointerDown);
    timeline.removeEventListener("keydown", onKeyDown);
    timeline.removeEventListener("scroll", onScroll);
  };
}

function cleanupTimelineBottomLock(): void {
  timelineBottomLockObserver?.disconnect();
  timelineBottomLockObserver = undefined;
  timelineBottomLockCleanup?.();
  timelineBottomLockCleanup = undefined;
  timelineBottomLockTimeline = undefined;
  timelineBottomLockContent = undefined;
  if (timelineBottomLockFrameHandle !== undefined) {
    window.cancelAnimationFrame(timelineBottomLockFrameHandle);
    timelineBottomLockFrameHandle = undefined;
  }
}

function queueTimelineBottomRestore(): void {
  if (timelineBottomLockFrameHandle !== undefined) {
    return;
  }
  timelineBottomLockFrameHandle = window.requestAnimationFrame(() => {
    timelineBottomLockFrameHandle = undefined;
    restoreTimelineBottomFromResizeObserver();
  });
}

function restoreTimelineBottomFromResizeObserver(): void {
  const timeline = timelineBottomLockTimeline;
  if (!timeline?.isConnected || !timelineBottomLockPinned) {
    return;
  }
  if (Date.now() < timelineBottomLockUserPauseUntil && !isTimelinePinnedToBottom(timeline)) {
    return;
  }
  timeline.scrollTop = timeline.scrollHeight;
}

function lockTimelineToBottom(timeline: HTMLElement): void {
  installTimelineBottomLock();
  setTimelineBottomLockPinned(true);
  timeline.scrollTop = timeline.scrollHeight;
}

function setTimelineBottomLockPinned(pinned: boolean): void {
  timelineBottomLockPinned = pinned;
}

function refreshTimelineGroupsForPatchedNodes(nodeIds: string[]): PatchedTimelineHydrationTargets {
  const changedIds = new Set(nodeIds);
  const groups = timelineGroups(visibleTimelineNodes());
  const previousDiffPreviewCount = pendingDiffPreviews.length;
  const groupIds = new Set<string>();
  const inlineActionIds = new Set<string>();
  const payloadDisclosureIds = new Set<string>();
  const toolGroupIds = new Set<string>();
  groups.forEach((group, index) => {
    if (group.nodes.some((node) => changedIds.has(node.id))) {
      const item = renderTimelineGroup(group, index, groups.length, groups);
      groupIds.add(item.id);
      collectHelperIslandIdsFromHtml(item.html, { inlineActionIds, payloadDisclosureIds });
      const props = timelineGroupProps.get(item.id);
      for (const bodyItem of props?.bodyItems || []) {
        collectHelperIslandIdsFromHtml(bodyItem.html, { inlineActionIds, payloadDisclosureIds });
        if (bodyItem.id.startsWith("tool-group:")) {
          toolGroupIds.add(bodyItem.id);
        }
      }
    }
  });
  return {
    groupIds,
    inlineActionIds,
    nodeIds: changedIds,
    payloadDisclosureIds,
    toolGroupIds,
    needsDiffHydration: pendingDiffPreviews.length > previousDiffPreviewCount
  };
}

function collectHelperIslandIdsFromHtml(
  html: string,
  ids: { inlineActionIds: Set<string>; payloadDisclosureIds: Set<string> }
): void {
  const template = document.createElement("template");
  template.innerHTML = html;
  template.content.querySelectorAll<HTMLElement>("[data-payload-disclosure-id]").forEach((element) => {
    const id = element.dataset.payloadDisclosureId;
    if (id) {
      ids.payloadDisclosureIds.add(id);
    }
  });
  template.content.querySelectorAll<HTMLElement>("[data-inline-actions-id]").forEach((element) => {
    const id = element.dataset.inlineActionsId;
    if (id) {
      ids.inlineActionIds.add(id);
    }
  });
}

function cancelPatchedNodeHydration(): void {
  pendingPatchedNodeIds.clear();
  pendingPatchedNodeBottomLock = false;
  if (patchedNodeHydrationFrameHandle !== undefined) {
    window.cancelAnimationFrame(patchedNodeHydrationFrameHandle);
    patchedNodeHydrationFrameHandle = undefined;
  }
}

function cancelTimelineStructureHydration(): void {
  pendingTimelineStructureChromeHydration = false;
  if (timelineStructureHydrationFrameHandle !== undefined) {
    window.cancelAnimationFrame(timelineStructureHydrationFrameHandle);
    timelineStructureHydrationFrameHandle = undefined;
  }
}

function cancelChromeHydration(): void {
  if (chromeHydrationFrameHandle !== undefined) {
    window.cancelAnimationFrame(chromeHydrationFrameHandle);
    chromeHydrationFrameHandle = undefined;
  }
}

function captureRenderFocus(): RenderFocusSnapshot {
  const active = document.activeElement as HTMLTextAreaElement | HTMLInputElement | null;
  const composerFocused = Boolean(active?.classList.contains("composer-input"));
  const timelineSearchFocused = Boolean(active?.classList.contains("timeline-search-input"));
  const selectionStart = composerFocused ? active?.selectionStart : undefined;
  const selectionEnd = composerFocused ? active?.selectionEnd : undefined;
  const searchSelectionStart = timelineSearchFocused ? active?.selectionStart : undefined;
  const searchSelectionEnd = timelineSearchFocused ? active?.selectionEnd : undefined;
  const oldTimeline = document.querySelector<HTMLElement>(".timeline");
  const hadTimeline = Boolean(oldTimeline);
  const wasPinnedToBottom = shouldKeepTimelinePinnedToBottom(oldTimeline);
  const previousScrollTop = oldTimeline?.scrollTop ?? 0;

  return {
    composerFocused,
    timelineSearchFocused,
    selectionStart,
    selectionEnd,
    searchSelectionStart,
    searchSelectionEnd,
    hadTimeline,
    wasPinnedToBottom,
    previousScrollTop
  };
}

function prepareRenderProps(visibleNodes: RenderNode[]): RenderPass {
  pendingDiffPreviews = [];
  composerCardProps = undefined;
  diffCardProps = new Map<string, DiffCardProps>();
  emptyStateCardProps = new Map<string, EmptyStateCardProps>();
  eventCardProps = new Map<string, EventCardProps>();
  headerBarProps = undefined;
  inlineActionsProps = new Map<string, InlineActionsProps>();
  messageCardProps = new Map<string, MessageCardProps>();
  const lastUserMessage = visibleNodes.filter((n): n is Extract<RenderNode, { kind: "message" }> => n.kind === "message" && (n as Extract<RenderNode, { kind: "message" }>).role === "user").at(-1);
  lastUserMessageNodeId = lastUserMessage?.id;
  payloadDisclosureProps = new Map<string, PayloadDisclosureProps>();
  planCardProps = new Map<string, PlanCardProps>();
  recoveryBannerProps = new Map<string, RecoveryBannerProps>();
  reviewDrawerProps = undefined;
  terminalCardProps = new Map<string, TerminalCardProps>();
  thoughtCardProps = new Map<string, ThoughtCardProps>();
  timelineGroupProps = new Map<string, TimelineGroupCardProps>();
  timelineNavigationProps = undefined;
  toolCallCardProps = new Map<string, ToolCallCardProps>();
  toolCallGroupCardProps = new Map<string, ToolCallGroupCardProps>();
  diffPreviewCounter = 0;
  timelineScrollerProps = {
    items: timelineScrollerItems(visibleNodes)
  };
  cleanupDetachedDiffEnhancements();
  return {
    renderEpoch: ++renderEpoch,
    diffEpoch: ++diffRenderEpoch
  };
}

function render(): void {
  clearScheduledRender();
  lastRenderAt = Date.now();
  if (!app) {
    return;
  }

  const task = state.task;
  const visibleNodes = visibleTimelineNodes();
  const focus = captureRenderFocus();
  const pass = prepareRenderProps(visibleNodes);
  const timelineNavigationHtml = timelineNavigation(visibleNodes);
  const headerHtml = header(task, timelineNavigationHtml);
  const composerHtml = composer();
  const reviewHtml = reviewVisible ? reviewDrawer(task) : "";
  syncAppShell({ headerHtml, composerHtml, reviewHtml });
  hydrateExistingShell(pass, focus);
}

function renderStateUpdate(): void {
  clearScheduledRender();
  lastRenderAt = Date.now();
  if (!app || !existingShellCanHydrate()) {
    render();
    return;
  }

  const task = state.task;
  const visibleNodes = visibleTimelineNodes();
  const focus = captureRenderFocus();
  const pass = prepareRenderProps(visibleNodes);
  timelineNavigation(visibleNodes);
  header(task);
  composer();
  if (reviewVisible) {
    reviewDrawer(task);
  }
  hydrateExistingShell(pass, focus);
}

function syncAppShell({
  composerHtml,
  headerHtml,
  reviewHtml
}: {
  composerHtml: string;
  headerHtml: string;
  reviewHtml: string;
}): void {
  if (!app) {
    return;
  }
  let shell = app.querySelector<HTMLElement>(":scope > .shell");
  if (!shell) {
    app.innerHTML = initialShellHtml({ headerHtml, composerHtml, reviewHtml });
    return;
  }

  shell.className = `shell ${reviewVisible ? "review-open" : ""}`.trim();
  ensureLiveRegion(shell);
  syncHeaderShell(shell, headerHtml);
  syncTimelineShell(shell);
  syncComposerShell(shell, composerHtml);
  syncReviewShell(shell, reviewHtml);
}

function initialShellHtml({
  composerHtml,
  headerHtml,
  reviewHtml
}: {
  composerHtml: string;
  headerHtml: string;
  reviewHtml: string;
}): string {
  return `
    <section class="shell ${reviewVisible ? "review-open" : ""}">
      <div class="sr-only" role="status" aria-live="polite" aria-atomic="true" data-live-announcement>${escapeHtml(announcement)}</div>
      ${headerHtml}
      <section class="timeline-shell" aria-label="Transcript workspace">
        <div class="timeline-scroller-root" data-timeline-scroller-root></div>
      </section>
      ${composerHtml}
      ${reviewHtml}
    </section>
  `;
}

function ensureLiveRegion(shell: HTMLElement): void {
  if (shell.querySelector("[data-live-announcement]")) {
    return;
  }
  const liveRegion = document.createElement("div");
  liveRegion.className = "sr-only";
  liveRegion.setAttribute("role", "status");
  liveRegion.setAttribute("aria-live", "polite");
  liveRegion.setAttribute("aria-atomic", "true");
  liveRegion.setAttribute("data-live-announcement", "");
  liveRegion.textContent = announcement;
  shell.prepend(liveRegion);
}

function syncHeaderShell(shell: HTMLElement, headerHtml: string): void {
  let header = shell.querySelector<HTMLElement>(":scope > .chat-header");
  if (!header) {
    const nextHeader = htmlElement(headerHtml);
    if (nextHeader) {
      shell.insertBefore(nextHeader, shell.querySelector(":scope > .timeline-shell"));
    }
    return;
  }

  if (!header.querySelector("[data-header-bar-root]")) {
    const headerRoot = document.createElement("div");
    headerRoot.className = "header-bar-react-root";
    headerRoot.setAttribute("data-header-bar-root", "");
    headerRoot.setAttribute("data-header-bar-id", "header");
    const beforeNavigation = header.querySelector("[data-timeline-navigation-root]");
    header.insertBefore(headerRoot, beforeNavigation);
  }

  if (!header.querySelector("[data-timeline-navigation-root]")) {
    const navigationRoot = htmlElement(headerHtml)?.querySelector<HTMLElement>("[data-timeline-navigation-root]");
    if (navigationRoot) {
      header.append(navigationRoot);
    }
  }
}

function syncTimelineShell(shell: HTMLElement): void {
  let timelineShell = shell.querySelector<HTMLElement>(":scope > .timeline-shell");
  if (!timelineShell) {
    timelineShell = document.createElement("section");
    timelineShell.className = "timeline-shell";
    timelineShell.setAttribute("aria-label", "Transcript workspace");
    const composerElement = shell.querySelector(":scope > .composer");
    shell.insertBefore(timelineShell, composerElement);
  }
  if (!timelineShell.querySelector("[data-timeline-scroller-root]")) {
    const scrollerRoot = document.createElement("div");
    scrollerRoot.className = "timeline-scroller-root";
    scrollerRoot.setAttribute("data-timeline-scroller-root", "");
    timelineShell.append(scrollerRoot);
  }
}

function syncComposerShell(shell: HTMLElement, composerHtml: string): void {
  let composerElement = shell.querySelector<HTMLElement>(":scope > .composer");
  if (!composerElement) {
    const nextComposer = htmlElement(composerHtml);
    if (nextComposer) {
      shell.insertBefore(nextComposer, shell.querySelector(":scope > .review-drawer"));
    }
    return;
  }
  if (!composerElement.querySelector("[data-composer-card-root]")) {
    const composerRoot = document.createElement("div");
    composerRoot.className = "composer-card-react-root";
    composerRoot.setAttribute("data-composer-card-root", "");
    composerRoot.setAttribute("data-composer-id", "composer");
    composerElement.append(composerRoot);
  }
}

function syncReviewShell(shell: HTMLElement, reviewHtml: string): void {
  const reviewElement = shell.querySelector<HTMLElement>(":scope > .review-drawer");
  if (!reviewHtml) {
    reviewElement?.remove();
    return;
  }
  if (!reviewElement) {
    const nextReview = htmlElement(reviewHtml);
    if (nextReview) {
      shell.append(nextReview);
    }
  }
}

function htmlElement(html: string): HTMLElement | undefined {
  const template = document.createElement("template");
  template.innerHTML = html.trim();
  const element = template.content.firstElementChild;
  return element instanceof HTMLElement ? element : undefined;
}

function existingShellCanHydrate(): boolean {
  return Boolean(
    document.querySelector("[data-header-bar-root]") &&
      document.querySelector("[data-timeline-navigation-root]") &&
      document.querySelector("[data-timeline-scroller-root]") &&
      document.querySelector("[data-composer-card-root]")
  );
}

function isCurrentRender(epoch: number): boolean {
  return epoch === renderEpoch;
}

function hydrateExistingShell(pass: RenderPass, focus: RenderFocusSnapshot): void {
  const restoreTimelineSearchFocus = focus.timelineSearchFocused || pendingTimelineSearchFocus;
  pendingTimelineSearchFocus = false;
  syncLiveAnnouncement();
  void (async () => {
    await Promise.all([
      hydrateHeaderBars(),
      hydrateTimelineNavigation().then(() => {
        restoreTimelineSearchInput({
          searchFocused: restoreTimelineSearchFocus,
          selectionStart: focus.searchSelectionStart,
          selectionEnd: focus.searchSelectionEnd
        });
      }),
      hydrateComposerCards().then(() => {
        restoreComposerInput({
          composerFocused: focus.composerFocused,
          selectionStart: focus.selectionStart,
          selectionEnd: focus.selectionEnd
        });
      })
    ]);
    if (!isCurrentRender(pass.renderEpoch)) {
      return;
    }
    await hydrateTimelineScroller();
    if (!isCurrentRender(pass.renderEpoch)) {
      return;
    }
    restoreTimelineScrollFromFocus(focus);
    await hydrateReactIslands();
    if (!isCurrentRender(pass.renderEpoch)) {
      return;
    }
    restoreTimelineBottomAfterIslandHydration(focus);
    window.requestAnimationFrame(() => {
      if (!isCurrentRender(pass.renderEpoch)) {
        return;
      }
      void highlightCodeBlocks();
      void hydrateDiffPreviews(pass.diffEpoch);
    });
  })();
}

function syncLiveAnnouncement(): void {
  const liveRegion = document.querySelector<HTMLElement>("[data-live-announcement]");
  if (liveRegion) {
    liveRegion.textContent = announcement;
  }
}

function restoreComposerInput({
  composerFocused,
  selectionEnd,
  selectionStart
}: {
  composerFocused: boolean;
  selectionEnd?: number | null | undefined;
  selectionStart?: number | null | undefined;
}): void {
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  if (input) {
    input.value = composerDraft;
    resizeComposerInput(input);
    if (composerFocused) {
      input.focus();
      if (selectionStart !== undefined && selectionEnd !== undefined) {
        input.setSelectionRange(selectionStart, selectionEnd);
      }
    }
  }
  syncComposerAffordances();
}

async function hydrateComposerCards(): Promise<void> {
  if (!document.querySelector("[data-composer-card-root]")) {
    return;
  }
  composerCardModulePromise ??= import("./ComposerCard.js");
  const module = await composerCardModulePromise;
  module.mountComposerCards({
    getProps: (id) => (id === "composer" ? composerCardProps : undefined)
  });
}

async function hydrateHeaderBars(): Promise<void> {
  if (!document.querySelector("[data-header-bar-root]")) {
    return;
  }
  headerBarModulePromise ??= import("./HeaderBar.js");
  const module = await headerBarModulePromise;
  module.mountHeaderBars({
    getProps: (id) => (id === "header" ? headerBarProps : undefined)
  });
}

async function hydrateTimelineNavigation(): Promise<void> {
  if (!document.querySelector("[data-timeline-navigation-root]")) {
    return;
  }
  timelineNavigationModulePromise ??= import("./TimelineNavigation.js");
  const module = await timelineNavigationModulePromise;
  module.mountTimelineNavigation({
    getProps: (id) => (id === "timeline" ? timelineNavigationProps : undefined)
  });
}

async function hydrateToolCallCards(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-tool-call-card-root]")) {
    return;
  }
  toolCallCardModulePromise ??= import("./ToolCallCard.js");
  const module = await toolCallCardModulePromise;
  module.mountToolCallCards({
    ids: options.ids,
    getProps: (nodeId) => toolCallCardProps.get(nodeId),
    onOpenLocation: ({ path, line }) => {
      vscode.postMessage({
        type: "openLocation",
        path,
        line
      });
    }
  });
}

async function hydrateToolCallGroupCards(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-tool-call-group-root]")) {
    return;
  }
  toolCallGroupCardModulePromise ??= import("./ToolCallGroupCard.js");
  const module = await toolCallGroupCardModulePromise;
  module.mountToolCallGroupCards({
    ids: options.ids,
    getProps: (id) => toolCallGroupCardProps.get(id),
    onOpenLocation: ({ path, line }) => {
      vscode.postMessage({
        type: "openLocation",
        path,
        line
      });
    }
  });
}

async function hydrateMessageCards(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-message-card-root]")) {
    return;
  }
  messageCardModulePromise ??= import("./MessageCard.js");
  const module = await messageCardModulePromise;
  module.mountMessageCards({
    ids: options.ids,
    getProps: (nodeId) => messageCardProps.get(nodeId)
  });
}

async function hydratePayloadDisclosures(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-payload-disclosure-root]")) {
    payloadDisclosureModulePromise?.then((module) => module.cleanupDetachedPayloadDisclosures()).catch(() => undefined);
    return;
  }
  payloadDisclosureModulePromise ??= import("./PayloadDisclosure.js");
  const module = await payloadDisclosureModulePromise;
  module.mountPayloadDisclosures({
    ids: options.ids,
    getProps: (id) => payloadDisclosureProps.get(id)
  });
  window.requestAnimationFrame(() => {
    module.mountPayloadDisclosures({
      ids: options.ids,
      getProps: (id) => payloadDisclosureProps.get(id)
    });
  });
}

async function hydrateInlineActions(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-inline-actions-root]")) {
    inlineActionsModulePromise?.then((module) => module.cleanupDetachedInlineActions()).catch(() => undefined);
    return;
  }
  inlineActionsModulePromise ??= import("./InlineActions.js");
  const module = await inlineActionsModulePromise;
  module.mountInlineActions({
    ids: options.ids,
    getProps: (id) => inlineActionsProps.get(id)
  });
  window.requestAnimationFrame(() => {
    module.mountInlineActions({
      ids: options.ids,
      getProps: (id) => inlineActionsProps.get(id)
    });
  });
}

async function hydratePlanCards(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-plan-card-root]")) {
    return;
  }
  planCardModulePromise ??= import("./PlanCard.js");
  const module = await planCardModulePromise;
  module.mountPlanCards({
    ids: options.ids,
    getProps: (nodeId) => planCardProps.get(nodeId)
  });
}

async function hydrateEmptyStateCards(): Promise<void> {
  if (!document.querySelector("[data-empty-state-card-root]")) {
    return;
  }
  emptyStateCardModulePromise ??= import("./EmptyStateCard.js");
  const module = await emptyStateCardModulePromise;
  module.mountEmptyStateCards({
    getProps: (id) => emptyStateCardProps.get(id)
  });
}

async function hydrateDiffCards(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-diff-card-root]")) {
    return;
  }
  diffCardModulePromise ??= import("./DiffCard.js");
  const module = await diffCardModulePromise;
  module.mountDiffCards({
    ids: options.ids,
    getProps: (nodeId) => diffCardProps.get(nodeId)
  });
}

async function hydrateEventCards(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-event-card-root]")) {
    return;
  }
  eventCardModulePromise ??= import("./EventCard.js");
  const module = await eventCardModulePromise;
  module.mountEventCards({
    ids: options.ids,
    getProps: (nodeId) => eventCardProps.get(nodeId)
  });
}

async function hydrateTerminalCards(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-terminal-card-root]")) {
    return;
  }
  terminalCardModulePromise ??= import("./TerminalCard.js");
  const module = await terminalCardModulePromise;
  module.mountTerminalCards({
    ids: options.ids,
    getProps: (nodeId) => terminalCardProps.get(nodeId)
  });
}

async function hydrateThoughtCards(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-thought-card-root]")) {
    return;
  }
  thoughtCardModulePromise ??= import("./ThoughtCard.js");
  const module = await thoughtCardModulePromise;
  module.mountThoughtCards({
    ids: options.ids,
    getProps: (nodeId) => thoughtCardProps.get(nodeId)
  });
}

async function hydrateTimelineGroups(options: { ids?: ReadonlySet<string> | undefined } = {}): Promise<void> {
  if (!document.querySelector("[data-timeline-group-root]")) {
    return;
  }
  timelineGroupModulePromise ??= import("./TimelineGroup.js");
  const module = await timelineGroupModulePromise;
  module.mountTimelineGroups({
    ids: options.ids,
    getProps: (id) => timelineGroupProps.get(id)
  });
}

async function hydrateRecoveryBanners(): Promise<void> {
  if (!document.querySelector("[data-recovery-banner-root]")) {
    return;
  }
  recoveryBannerModulePromise ??= import("./RecoveryBanner.js");
  const module = await recoveryBannerModulePromise;
  module.mountRecoveryBanners({
    getProps: (id) => recoveryBannerProps.get(id)
  });
}

async function hydrateReviewDrawers(): Promise<void> {
  if (!document.querySelector("[data-review-drawer-root]")) {
    return;
  }
  reviewDrawerModulePromise ??= import("./ReviewDrawer.js");
  const module = await reviewDrawerModulePromise;
  module.mountReviewDrawers({
    getProps: (id) => (id === "review" ? reviewDrawerProps : undefined)
  });
}

async function hydrateTimelineScroller(): Promise<void> {
  const element = document.querySelector<HTMLElement>("[data-timeline-scroller-root]");
  if (!element || !timelineScrollerProps) {
    return;
  }
  timelineScrollerModulePromise ??= import("./TimelineScroller.js");
  const module = await timelineScrollerModulePromise;
  module.cleanupTimelineScroller();
  module.mountTimelineScroller(element, timelineScrollerProps);
}

async function hydrateReactIslands(): Promise<void> {
  await hydrateTimelineGroups();
  await Promise.all([
    hydrateDiffCards(),
    hydrateEmptyStateCards(),
    hydrateEventCards(),
    hydrateMessageCards(),
    hydratePlanCards(),
    hydrateRecoveryBanners(),
    hydrateReviewDrawers(),
    hydrateTerminalCards(),
    hydrateThoughtCards(),
    hydrateToolCallCards(),
    hydrateToolCallGroupCards()
  ]);
  await hydratePayloadDisclosures();
  await hydrateInlineActions();
}

async function hydratePatchedTimelineIslands(targets: PatchedTimelineHydrationTargets): Promise<void> {
  await hydrateTimelineGroups({ ids: targets.groupIds });
  await Promise.all([
    hydrateDiffCards({ ids: targets.nodeIds }),
    hydrateEventCards({ ids: targets.nodeIds }),
    hydrateMessageCards({ ids: targets.nodeIds }),
    hydratePlanCards({ ids: targets.nodeIds }),
    hydrateTerminalCards({ ids: targets.nodeIds }),
    hydrateThoughtCards({ ids: targets.nodeIds }),
    hydrateToolCallCards({ ids: targets.nodeIds }),
    hydrateToolCallGroupCards({ ids: targets.toolGroupIds })
  ]);
  collectPatchedTimelinePayloadDisclosureIds(targets);
  await hydratePayloadDisclosures({ ids: targets.payloadDisclosureIds });
  collectPatchedTimelineInlineActionIds(targets);
  await hydrateInlineActions({ ids: targets.inlineActionIds });
}

function collectPatchedTimelinePayloadDisclosureIds(targets: PatchedTimelineHydrationTargets): void {
  for (const scope of patchedTimelineHydrationScopes(targets)) {
    scope.querySelectorAll<HTMLElement>("[data-payload-disclosure-id]").forEach((element) => {
      const id = element.dataset.payloadDisclosureId;
      if (id) {
        targets.payloadDisclosureIds.add(id);
      }
    });
  }
}

function collectPatchedTimelineInlineActionIds(targets: PatchedTimelineHydrationTargets): void {
  for (const scope of patchedTimelineHydrationScopes(targets)) {
    scope.querySelectorAll<HTMLElement>("[data-inline-actions-id]").forEach((element) => {
      const id = element.dataset.inlineActionsId;
      if (id) {
        targets.inlineActionIds.add(id);
      }
    });
  }
}

function patchedTimelineHydrationScopes(targets: PatchedTimelineHydrationTargets): HTMLElement[] {
  const scopes = new Set<HTMLElement>();
  const addScope = (id: string): void => {
    const element = document.getElementById(id);
    if (element) {
      scopes.add(element);
    }
  };
  targets.groupIds.forEach(addScope);
  targets.nodeIds.forEach((id) => addScope(nodeDomId(id)));
  targets.toolGroupIds.forEach((id) => addScope(nodeDomId(id)));
  return [...scopes];
}

function providerFailureBanner(failure: NonNullable<WebviewState["providerFailure"]>): string {
  const when = failure.occurredAt ? new Date(failure.occurredAt).toLocaleTimeString() : "";
  const id = "provider-failure";
  recoveryBannerProps.set(id, {
    id,
    kind: "failure",
    role: "alert",
    ariaLive: "assertive",
    eyebrow: "Agent interrupted",
    title: failure.message,
    description: "Partial transcript and lane changes remain in CrabDB. Review the task or start a follow-up from the latest checkpoint.",
    detail: failure.detail,
    badges: [
      failure.code !== undefined && failure.code !== null ? `exit ${failure.code}` : "",
      when
    ].filter(Boolean),
    actions: [
      { action: "focusReview", label: "Open review", tone: "review" },
      { action: "startFollowUp", label: "Start follow-up", tone: "primary" },
      { action: "showAcpLogs", label: "Show logs", tone: "provider" }
    ],
    paths: []
  });
  return recoveryBannerRoot(id);
}

function overlapWarningBanner(): string {
  const overlaps = state.taskOverlaps || [];
  if (!overlaps.length) {
    return "";
  }
  const sharedCount = uniqueStrings(overlaps.flatMap((overlap) => overlap.sharedPaths)).length;
  const top = overlaps[0];
  const id = "task-overlap";
  recoveryBannerProps.set(id, {
    id,
    kind: "overlap",
    role: "status",
    ariaLive: "polite",
    eyebrow: "Parallel work overlap",
    title: top ? `${top.title} also changes ${top.sharedPaths[0] || "this task's files"}` : "Another task changes the same files",
    description: "Compare tasks or refresh CrabDB state before applying this lane.",
    badges: [
      `${overlaps.length} task${overlaps.length === 1 ? "" : "s"}`,
      `${sharedCount} shared path${sharedCount === 1 ? "" : "s"}`
    ],
    actions: [
      { action: "compareTasks", label: "Compare tasks", tone: "provider" },
      { action: "refresh", label: "Refresh", tone: "lane" },
      { action: "queueMerge", label: "Queue merge", tone: "lane" }
    ],
    paths: overlaps.slice(0, 3).map((overlap) => ({
      id: overlap.taskId,
      title: shortLabel(overlap.title),
      labels: overlap.sharedPaths.slice(0, 3).map(shortLabel).join(", ")
    }))
  });
  return recoveryBannerRoot(id);
}

function recoveryBannerRoot(id: string): string {
  return `<div class="recovery-banner-react-root" data-recovery-banner-root data-recovery-banner-id="${escapeHtml(id)}"></div>`;
}

function timelineScrollerItems(nodes: RenderNode[]): TimelineScrollerItemView[] {
  const items: TimelineScrollerItemView[] = [];
  if (state.providerFailure) {
    items.push({
      id: "provider-failure",
      className: "timeline-scroller-row-recovery",
      html: providerFailureBanner(state.providerFailure)
    });
  }
  const overlap = overlapWarningBanner();
  if (overlap) {
    items.push({
      id: "task-overlap",
      className: "timeline-scroller-row-recovery",
      html: overlap
    });
  }
  if (!nodes.length) {
    items.push({
      id: "timeline-empty",
      className: "timeline-scroller-row-empty",
      html: emptyTimeline()
    });
    return items;
  }
  items.push(...renderTimeline(nodes));
  return items;
}

function header(task: WebviewState["task"], timelineNavigationHtml = ""): string {
  const status = task?.status || "new";
  const showStatusPill = !["new", "ready"].includes(status);
  const changed = task?.changedPaths?.length || 0;
  const usage = currentUsageNode();
  const modeLabel = currentModeLabel();
  const configCount = currentConfigOptions().length;
  const sessionState = sessionStateLabel();
  const coordination = coordinationSummaryFromSources(task, state.taskView);
  const currentProvider = currentProviderProfile();
  const toolbar = buildToolbarModel({
    taskStatus: status,
    lane: task?.lane,
    changedPaths: changed,
    providerLabel: state.provider || currentProvider?.label || task?.provider,
    providerCrabdbBacked: currentProvider?.crabdbBacked,
    sending: state.sending,
    permissionPending: state.permissionPending,
    providerFailure: Boolean(state.providerFailure),
    supportsFromRef: currentProvider?.supportsFromRef,
    reviewVisible,
    sessionLabel: sessionState?.label,
    sessionTone: sessionState?.tone,
    acpSessionId: state.acpSessionId || state.persistedAcpSessionId,
    nextAction: task?.nextAction,
    modeLabel,
    configCount,
    commandCount: currentCommands().length,
    coordinationLabels: coordination.labels,
    coordinationSeverity: coordination.severity,
    capabilities: state.capabilities?.promptCapabilities
  });
  const reviewLabel = reviewVisible ? "Hide review" : "Open review";
  headerBarProps = {
    id: "header",
    title: task?.title || "New agent task",
    status,
    showStatusPill,
    toolbar,
    usage: usage ? { used: usage.used, size: usage.size } : undefined,
    detailsIconHtml: iconSvg("tool"),
    capabilitiesIconHtml: iconSvg("tool"),
    primaryActionIconHtml: iconSvg(toolbarActionIcon(toolbar.primaryAction.action)),
    laneMap: timelineNavigationProps,
    inspectActions: [
      {
        action: "toggleReview",
        label: reviewLabel,
        iconHtml: iconSvg("review"),
        active: reviewVisible,
        ariaPressed: reviewVisible,
        ariaExpanded: reviewVisible,
        ariaControls: "review"
      },
      { action: "openDiff", label: "Open diff", iconHtml: iconSvg("diff") },
      { action: "openSettings", label: "Open CrabDB settings", iconHtml: iconSvg("settings") }
    ],
    runActions: [
      { action: "refresh", label: "Refresh task", iconHtml: iconSvg("refresh") },
      { action: "cancel", label: "Cancel current turn", iconHtml: iconSvg("stop"), disabled: !state.sending && !state.permissionPending }
    ]
  };
  return `
    <header class="chat-header">
      <div class="header-bar-react-root" data-header-bar-root data-header-bar-id="header"></div>
      ${timelineNavigationHtml}
    </header>
  `;
}

function toolbarActionIcon(action: ToolbarAction["action"]): IconName {
  switch (action) {
    case "cancel":
      return "stop";
    case "dryRunApply":
      return "check";
    case "focusReview":
      return "review";
    case "focusTranscript":
      return "tree";
    case "refresh":
      return "refresh";
    case "startFollowUp":
      return "message";
    default:
      return "message";
  }
}

function currentUsageNode(): Extract<RenderNode, { kind: "usage" }> | undefined {
  return state.nodes.find((node) => node.kind === "usage") as Extract<RenderNode, { kind: "usage" }> | undefined;
}

function contextUsageGauge(usage: Extract<RenderNode, { kind: "usage" }> | undefined): string {
  if (!usage) {
    return "";
  }
  const pct = usage.size > 0 ? Math.min(100, Math.round((usage.used / usage.size) * 100)) : 0;
  const tone = pct >= 90 ? "risk" : pct >= 70 ? "warning" : "ok";
  const detail = `${pct}% context (${usage.used}/${usage.size} tokens)`;
  return `<span class="composer-context-gauge composer-context-gauge-${tone}" role="meter" aria-label="Context usage" aria-valuenow="${pct}" title="${detail}" style="--context-pct:${pct}%"><span>${pct}%</span></span>`;
}

function timelineNavigation(visibleNodes: RenderNode[]): string {
  const transcriptNodes = chatNodes(state.nodes);
  const counts = timelineFilterCounts(transcriptNodes);
  const queryTokens = timelineSearchTokens(timelineQuery);
  const filtered = timelineFilter !== "all" || queryTokens.length > 0;
  const queryDetail = queryTokens.length ? ` matching ${queryTokens.join(" + ")}` : "";
  const allGroups = timelineGroups(transcriptNodes);
  const visibleGroups = timelineGroups(visibleNodes);
  const task = state.task;
  const lane = task?.lane || visibleNodes[0]?.lane || "pending";
  const sessionId = state.acpSessionId || state.persistedAcpSessionId || visibleNodes.find((node) => node.acpSessionId)?.acpSessionId;
  const messageCount = transcriptNodes.filter((node) => node.kind === "message").length;
  const toolCount = transcriptNodes.filter((node) => node.kind === "tool").length;
  const activity = buildToolActivitySummary(visibleNodes);
  const turnGroups = allGroups.filter((group) => group.turnId);
  const visibleTurnGroups = visibleGroups.filter((group) => group.turnId);
  const lifecycleChip = timelineLifecycleChip(transcriptNodes);
  timelineNavigationProps = {
    id: "timeline",
    filters: TIMELINE_FILTERS.map((filter) => ({
      id: filter.id,
      label: filter.label,
      count: counts[filter.id],
      active: timelineFilter === filter.id
    })),
    query: timelineQuery,
    queryDetail,
    filtered,
    visibleCount: visibleNodes.length,
    searchIconHtml: iconSvg("search"),
    mapIconHtml: iconSvg("tree"),
    activityIconHtml: iconSvg(activity.tone === "risk" ? "diagnostics" : "tool"),
    visibleGroups: visibleGroups.length,
    chips: [
      { id: "lane", label: `Lane ${shortLabel(lane)}`, iconHtml: iconSvg("lane"), active: true },
      { id: "session", label: `Session ${sessionId ? shortLabel(sessionId) : "new"}`, iconHtml: iconSvg("session") },
      ...(lifecycleChip ? [lifecycleChip] : []),
      { id: "turns", label: `${turnGroups.length} turn${turnGroups.length === 1 ? "" : "s"}`, iconHtml: iconSvg("turn") },
      { id: "messages", label: `${messageCount} message${messageCount === 1 ? "" : "s"}`, iconHtml: iconSvg("message") },
      { id: "tools", label: `${toolCount} tool${toolCount === 1 ? "" : "s"}`, iconHtml: iconSvg("tool") }
    ],
    activity,
    turnLinks: visibleTurnGroups.slice(-8).map((group) => {
      const target = timelineGroupDomId(group);
      return {
        id: group.key,
        href: `#${target}`,
        label: group.label,
        detail: group.detail
      };
    })
  };
  return `<div class="timeline-navigation-react-root" data-timeline-navigation-root data-timeline-navigation-id="timeline"></div>`;
}

function timelineLifecycleChip(nodes: RenderNode[]): { id: string; label: string; iconHtml: string; active?: boolean } | undefined {
  const liveCount = nodes.filter((node) => node.status === "pending" || node.status === "in_progress").length;
  if (state.sending || liveCount) {
    const checkpointPending = nodes.some((node) => node.kind === "completion" && node.checkpointPending);
    if (!state.sending && checkpointPending) {
      return { id: "stream", label: "Checkpoint pending", iconHtml: iconSvg("check") };
    }
    return {
      id: "stream",
      label: liveCount ? `Streaming ${liveCount}` : "Streaming",
      iconHtml: iconSvg("message"),
      active: true
    };
  }
  const completion = [...nodes].reverse().find((node) => node.kind === "completion");
  if (completion) {
    return {
      id: "stream",
      label: completion.status === "cancelled" ? "Cancelled" : completion.status === "failed" ? "Stopped" : "Completed",
      iconHtml: completion.status === "completed" ? iconSvg("check") : iconSvg("diagnostics")
    };
  }
  return undefined;
}

function emptyTimeline(): string {
  if (state.nodes.length) {
    const activeFilter = TIMELINE_FILTERS.find((filter) => filter.id === timelineFilter);
    const filterLabel = activeFilter && activeFilter.id !== "all" ? activeFilter.label : "";
    const queryTokens = timelineSearchTokens(timelineQuery);
    const constraints = [filterLabel ? `${filterLabel} items` : "", queryTokens.length ? queryTokens.join(" + ") : ""].filter(Boolean);
    const detail = constraints.length
      ? `No transcript items matched ${constraints.join(" and ")}. Clear filters to return to the full run.`
      : "Clear the search or filter to see the full agent run.";
    const id = "filtered";
    emptyStateCardProps.set(id, {
      id,
      variant: "filtered",
      ariaLabel: "No transcript items match the active filters.",
      iconHtml: iconSvg("search"),
      roleLabel: "Transcript filter",
      title: "No matching transcript items",
      description: detail,
      actions: [emptyStateAction("clearTimelineSearch", "Clear filters", "search", "primary", false)]
    });
    return `<div class="empty-state-root" data-empty-state-card-root data-empty-state-id="${id}"></div>`;
  }
  const blocked = Boolean(state.permissionPending);
  const running = Boolean(state.sending);
  const id = "ready";
  emptyStateCardProps.set(id, {
    id,
    variant: "ready",
    ariaLabel: "Empty transcript",
    iconHtml: iconSvg(blocked ? "review" : "message"),
    roleLabel: "CrabDB workspace",
    title: blocked ? "Permission needed before the next turn" : "Ready for a CrabDB turn",
    description: blocked
      ? "Resolve the pending tool request, then continue from the preserved transcript."
      : "Message the agent or attach editor context. CrabDB will record checkpoints, tool evidence, and review state.",
    actions: [
      emptyStateAction(blocked ? "focusReview" : "focusComposer", blocked ? "Open review" : "Write a message", blocked ? "review" : "message", "primary", running),
      emptyStateAction("attachSelection", "Attach selection", "selection", "secondary", blocked || running),
      emptyStateAction("attachFile", "Attach file", "file", "secondary", blocked || running)
    ]
  });
  return `<div class="empty-state-root" data-empty-state-card-root data-empty-state-id="${id}"></div>`;
}

function clearTimelineSearch(refocusSearch = false): void {
  timelineQuery = "";
  timelineFilter = "all";
  persistWebviewState();
  pendingTimelineSearchFocus = refocusSearch;
  render();
}

function restoreTimelineSearchInput({
  searchFocused,
  selectionEnd,
  selectionStart
}: {
  searchFocused: boolean;
  selectionEnd?: number | null | undefined;
  selectionStart?: number | null | undefined;
}): void {
  const search = document.querySelector<HTMLInputElement>(".timeline-search-input");
  if (!search) {
    return;
  }
  search.value = timelineQuery;
  if (searchFocused) {
    search.focus();
    if (selectionStart !== undefined && selectionEnd !== undefined) {
      search.setSelectionRange(selectionStart, selectionEnd);
    }
  }
}

function emptyStateAction(name: string, label: string, icon: IconName, tone: "primary" | "secondary", disabled: boolean): EmptyStateAction {
  return {
    action: name,
    label,
    iconHtml: iconSvg(icon),
    tone,
    disabled
  };
}

interface TimelineGroup extends TimelineConversationGroup {
  label: string;
  detail: string;
}

function renderTimeline(nodes: RenderNode[]): TimelineScrollerItemView[] {
  const groups = timelineGroups(nodes);
  if (!groups.length) {
    return [];
  }
  return groups.map((group, index) => renderTimelineGroup(group, index, groups.length, groups));
}

function renderTimelineGroup(group: TimelineGroup, index: number, total: number, groups: TimelineGroup[]): TimelineScrollerItemView {
  const open = shouldOpenTimelineGroup(group, index, total, groups);
  const id = timelineGroupDomId(group);
  const bodyItems = renderTimelineGroupBodyItems(group.nodes);
  timelineGroupProps.set(id, {
    id,
    label: group.label,
    detail: group.detail,
    status: group.status,
    statusLabel: toolStatusLabel(group.status),
    laneId: group.lane,
    iconHtml: iconSvg(group.turnId ? "turn" : "session"),
    bodyItems,
    open
  });
  return {
    id,
    className: `timeline-scroller-row-group timeline-scroller-row-${escapeClass(group.status)}`,
    scrollAnchor: Boolean(group.turnId),
    preserveDom: true,
    html: `<div id="${id}" class="timeline-group" data-timeline-group-root data-timeline-group-id="${escapeHtml(id)}"></div>`
  };
}

function renderTimelineGroupBodyItems(nodes: RenderNode[]): TimelineGroupCardProps["bodyItems"] {
  const items: TimelineGroupCardProps["bodyItems"] = [];
  for (let index = 0; index < nodes.length; index += 1) {
    const node = nodes[index];
    if (!node) {
      continue;
    }
    if (isInlineToolDiffNode(nodes, node)) {
      continue;
    }
    const toolRun = collectGroupableToolRun(nodes, index);
    if (toolRun.length >= 2) {
      items.push(toolCallGroupBodyItem(toolRun));
      index += toolRun.length - 1;
      continue;
    }
    items.push({
      id: node.id,
      className: `timeline-group-body-item timeline-group-body-item-${escapeClass(node.kind)}`,
      html: renderNode(node),
      preserveDom: true
    });
  }
  return items;
}

function collectGroupableToolRun(nodes: RenderNode[], startIndex: number): ToolNodeView[] {
  if (!shouldGroupCompletedToolCalls()) {
    return [];
  }
  const run: ToolNodeView[] = [];
  for (let index = startIndex; index < nodes.length; index += 1) {
    const node = nodes[index];
    if (!isGroupableToolNode(node)) {
      break;
    }
    run.push(node);
  }
  return run;
}

function shouldGroupCompletedToolCalls(): boolean {
  return timelineFilter === "all" && timelineSearchTokens(timelineQuery).length === 0;
}

function isGroupableToolNode(node: RenderNode | undefined): node is ToolNodeView {
  if (!node || node.kind !== "tool") {
    return false;
  }
  if (node.permission?.status === "pending" || node.permission?.status === "in_progress") {
    return false;
  }
  const status = toolDisplayStatus(node);
  return status !== "pending" && status !== "in_progress";
}

function timelineGroupDomId(group: TimelineGroup): string {
  return nodeDomId(`group:${group.key}`);
}

function timelineGroups(nodes: RenderNode[]): TimelineGroup[] {
  const groups = buildTimelineConversationGroups(nodes).map((group): TimelineGroup => ({
    ...group,
    label: "",
    detail: ""
  }));
  groups.forEach((group) => {
    group.label = group.turnId ? `Turn ${turnSequenceLabel(group.index, groups)}` : "Task updates";
    group.detail = timelineGroupDetail(group);
  });
  return groups;
}

function timelineGroupDetail(group: TimelineGroup): string {
  const messages = group.nodes.filter((node) => node.kind === "message").length;
  const tools = group.nodes.filter((node) => node.kind === "tool").length;
  const diffs = group.nodes.filter((node) => node.kind === "diff").length;
  const approvals = group.nodes.filter((node) => node.kind === "approval" || (node.kind === "tool" && node.permission)).length;
  const events = group.nodes.filter((node) => node.kind !== "message" && node.kind !== "tool" && node.kind !== "diff" && node.kind !== "approval").length;
  const parts = [
    countLabel(messages, "message"),
    countLabel(tools, "tool"),
    countLabel(diffs, "diff"),
    countLabel(approvals, "approval"),
    countLabel(events, "event")
  ].filter(Boolean);
  return parts.length ? parts.join(" / ") : `${group.nodes.length} item${group.nodes.length === 1 ? "" : "s"}`;
}

function turnSequenceLabel(groupIndex: number, groups: TimelineGroup[]): string {
  let turn = 0;
  for (let index = 0; index <= groupIndex; index += 1) {
    if (groups[index]?.turnId) {
      turn += 1;
    }
  }
  return String(turn || groupIndex + 1);
}

function countLabel(count: number, label: string): string {
  if (!count) {
    return "";
  }
  return `${count} ${label}${count === 1 ? "" : "s"}`;
}

function shouldOpenTimelineGroup(group: TimelineGroup, index: number, total: number, groups: TimelineGroup[]): boolean {
  if (timelineFilter !== "all" || timelineSearchTokens(timelineQuery).length) {
    return true;
  }
  if (group.status !== "completed") {
    return true;
  }
  return total <= 2 || index === total - 1 || isLatestTurnGroup(group, index, groups);
}

function isLatestTurnGroup(group: TimelineGroup, index: number, groups: TimelineGroup[]): boolean {
  if (!group.turnId) {
    return false;
  }
  for (let nextIndex = index + 1; nextIndex < groups.length; nextIndex += 1) {
    if (groups[nextIndex]?.turnId) {
      return false;
    }
  }
  return true;
}

function renderNode(node: RenderNode): string {
  switch (node.kind) {
    case "message":
      return messageNode(node);
    case "thought":
      return thoughtNode(node);
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
      return modeNode(node);
    case "config":
      return configNode(node);
    case "commands":
      return commandsNode(node);
    case "session":
      return sessionNode(node);
    case "usage":
      return usageNode(node);
    case "resource":
      return resourceBlock(node);
    case "unknown":
      return unknownNode(node);
    default:
      return "";
  }
}

function messageNode(node: Extract<RenderNode, { kind: "message" }>): string {
  const isSticky = node.role === "user" && node.id === lastUserMessageNodeId;
  const streamText = streamingTextForNode(node);
  messageCardProps.set(node.id, {
    nodeId: node.id,
    role: node.role,
    streaming: node.streaming,
    contentHtml: streamText !== undefined ? "" : renderContentBlocks(node.content),
    contentMode: streamText !== undefined ? "stream-text" : "html",
    contentText: streamText,
    isSticky
  });
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card message ${node.role}${isSticky ? " sticky-last-user" : ""}">
      <div class="rail"></div>
      <div class="card-body" data-message-card-root data-message-node-id="${escapeHtml(node.id)}"></div>
    </article>
  `;
}

function planNode(node: Extract<RenderNode, { kind: "plan" }>): string {
  const entries = node.entries.map((entry, index) => {
    const status = String(entry.status || "pending");
    return {
      id: `${node.id}-${index}`,
      title: String(entry.title || entry.content || "Task"),
      status,
      statusClass: escapeClass(status),
      priority: entry.priority ? String(entry.priority) : undefined
    };
  });
  planCardProps.set(node.id, {
    nodeId: node.id,
    title: "Plan",
    detail: entries.length ? `${entries.length} tracked ${entries.length === 1 ? "step" : "steps"}` : "No plan steps reported",
    entries,
    emptyText: "No plan steps reported."
  });
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card plan">
      <div class="rail"></div>
      <div class="plan-card-react-root" data-plan-card-root data-plan-node-id="${escapeHtml(node.id)}"></div>
    </article>
  `;
}

function thoughtNode(node: Extract<RenderNode, { kind: "thought" }>): string {
  const text = contentBlocksToPlainText(node.content);
  const statusLabel = node.status === "pending" || node.status === "in_progress" ? "live" : node.status === "completed" ? "done" : node.status;
  const streamText = streamingTextForNode(node);
  thoughtCardProps.set(node.id, {
    nodeId: node.id,
    title: "Thinking",
    detail: text ? shortLabel(text) : "Agent reasoning update",
    statusLabel,
    iconHtml: iconSvg("message"),
    contentHtml: streamText !== undefined ? "" : node.content.length ? renderContentBlocks(node.content) : "",
    contentMode: streamText !== undefined ? "stream-text" : "html",
    contentText: streamText,
    emptyText: "No thought content reported."
  });
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card thought activity">
      <div class="rail"></div>
      <div class="thought-card-react-root" data-thought-card-root data-thought-node-id="${escapeHtml(node.id)}"></div>
    </article>
  `;
}

function isLiveStatus(status: RenderNode["status"]): boolean {
  return status === "pending" || status === "in_progress";
}

function streamingTextForNode(
  node: Extract<RenderNode, { kind: "message" }> | Extract<RenderNode, { kind: "thought" }>
): string | undefined {
  if (!isLiveStatus(node.status)) {
    return undefined;
  }
  if (node.kind === "message" && !node.streaming) {
    return undefined;
  }
  return textOnlyContent(node.content);
}

type ToolNodeView = Extract<RenderNode, { kind: "tool" }>;
type ApprovalNodeView = Extract<RenderNode, { kind: "approval" }>;

function toolCallGroupBodyItem(nodes: ToolNodeView[]): TimelineGroupCardProps["bodyItems"][number] {
  const id = toolCallGroupId(nodes);
  const props = toolCallGroupPropsFromNodes(id, nodes);
  toolCallGroupCardProps.set(id, props);
  return {
    id,
    className: "timeline-group-body-item timeline-group-body-item-tool-group",
    html: `<article id="${nodeDomId(id)}" class="turn-card tool-call-group" data-tool-call-count="${nodes.length}"><div class="rail"></div><div class="tool-call-group-react-root" data-tool-call-group-root data-tool-call-group-id="${escapeHtml(id)}"></div></article>`,
    preserveDom: true
  };
}

function toolCallGroupId(nodes: ToolNodeView[]): string {
  return `tool-group:${nodes[0]?.id || "empty"}`;
}

function toolCallGroupPropsFromNodes(id: string, nodes: ToolNodeView[]): ToolCallGroupCardProps {
  const items = nodes.map(toolCallCardPropsFromNode);
  const status = combinedToolCallStatus(items.map((item) => item.status));
  const summary = summarizeToolCallGroup(items);
  return {
    id,
    title: summary.title,
    detail: summary.detail,
    status,
    statusLabel: toolStatusLabel(status),
    items
  };
}

function combinedToolCallStatus(statuses: string[]): string {
  const priority: Record<string, number> = {
    failed: 5,
    cancelled: 4,
    pending: 3,
    in_progress: 3,
    completed: 1
  };
  return statuses.reduce((current, next) => (priority[next] || 0) > (priority[current] || 0) ? next : current, "completed");
}

function toolNode(node: ToolNodeView): string {
  const props = toolCallCardPropsFromNode(node);
  toolCallCardProps.set(node.id, props);
  return toolNodeShell(node, props);
}

function toolCallCardPropsFromNode(node: ToolNodeView): ToolCallCardProps {
  const model = buildToolPresentation({
    title: node.title,
    toolKind: node.toolKind,
    toolStatus: node.toolStatus,
    locations: node.locations,
    content: node.content,
    rawInput: node.rawInput,
    rawOutput: node.rawOutput,
    source: node.source
  });
  const approval = toolApprovalProps(node, model);
  const cardModel = approval ? toolCardModelWithApproval(model, node.permission) : model;
  const displayStatus = toolDisplayStatus(node);
  const terminal = model.kind === "execute";
  const title = terminal ? "Bash" : model.title;
  const subtitle = terminal ? terminalToolIntent(node, model) : model.summary;
  const readPreview = model.kind === "read" && node.content.some((content) => renderedToolContentType(content) === "text");
  const details = toolStructuredDetails(node, model);
  const renderGenericEmptyState = !node.permission && !details && model.kind !== "edit" && model.kind !== "think";
  const contentHtml = terminal
    ? terminalToolPreview(node, model)
    : node.content.length
      ? node.content.map((item) => renderToolContent(item, node, model.kind)).join("")
      : renderGenericEmptyState
        ? `<p class="muted tool-empty-state">${escapeHtml(model.emptyText)}</p>`
        : "";
  return {
    nodeId: node.id,
    rawToolKind: node.toolKind,
    title,
    subtitle,
    status: displayStatus,
    terminal,
    readPreview,
    model: cardModel,
    stats: model.stats,
    facts: model.facts,
    actions: model.actions,
    locations: node.locations.map(toolCallCardLocation),
    contentHtml,
    approval,
    details
  };
}

const BACKGROUND_PROCESS_STRUCTURED_KEYS = new Set(["id", "status", "pid", "cwd", "command", "last_output"]);
const BACKGROUND_PROCESS_LABEL: Record<string, string> = {
  command: "Command",
  cwd: "Cwd",
  id: "Process id",
  last_output: "Last output",
  pid: "PID",
  status: "Status"
};
const BACKGROUND_PROCESS_TITLE: Record<string, string> = {
  list: "List background processes",
  logs: "View background logs",
  restart: "Restart background process",
  start: "Start background process",
  status: "Check background process",
  stop: "Stop background process"
};

function toolStructuredDetails(
  node: ToolNodeView,
  model: ToolPresentation
): ToolCallStructuredDetails | undefined {
  if (model.kind === "background_process") {
    return backgroundProcessDetails(node);
  }
  if (model.kind === "task") {
    return taskToolDetails(node, model);
  }
  return undefined;
}

function backgroundProcessDetails(node: ToolNodeView): ToolCallStructuredDetails | undefined {
  const input = toolArgumentRecord(asRecord(node.rawInput));
  const output = asRecord(node.rawOutput);
  const action = textValue(input.action) || "status";
  const parsedOutput = backgroundProcessStructuredOutput(toolOutputText(node, output));
  const rows: ToolCallStructuredDetails["rows"] = [];
  const command = terminalCommand(input) || terminalCommand(output) || findStructuredRowValue(parsedOutput.rows, "Command");
  const processId =
    textValue(output.id) ||
    textValue(output.processID) ||
    textValue(output.processId) ||
    textValue(output.process_id) ||
    textValue(input.id) ||
    textValue(input.processID) ||
    textValue(input.processId) ||
    textValue(input.process_id);
  const status = textValue(output.status) || textValue(output.state) || textValue(input.status) || textValue(input.state);
  const pid = textValue(output.pid) || textValue(input.pid);
  const cwd =
    textValue(output.cwd) ||
    textValue(output.workingDirectory) ||
    textValue(output.working_directory) ||
    textValue(input.cwd) ||
    textValue(input.workingDirectory) ||
    textValue(input.working_directory);
  const workdir = cwd ? undefined : textValue(input.workdir);
  const ready = asRecord(input.ready);

  addStructuredRow(rows, "Command", command);
  addStructuredRow(rows, "Description", textValue(input.description));
  addStructuredRow(rows, "Process id", processId);
  addStructuredRow(rows, "Status", status);
  addStructuredRow(rows, "PID", pid);
  addStructuredRow(rows, "Cwd", cwd);
  addStructuredRow(rows, "Workdir", workdir);
  addStructuredRow(rows, "Ports", textValue(ready.port));
  for (const row of parsedOutput.rows) {
    addStructuredRow(rows, row.label, row.value);
  }

  const details: ToolCallStructuredDetails = {
    kind: "background_process",
    title: BACKGROUND_PROCESS_TITLE[action] || "Background process",
    description: textValue(input.description),
    rows,
    output: parsedOutput.output,
    outputLabel: "Output"
  };
  return rows.length || details.output ? details : undefined;
}

function taskToolDetails(node: ToolNodeView, model: ToolPresentation): ToolCallStructuredDetails | undefined {
  const input = toolArgumentRecord(asRecord(node.rawInput));
  const output = asRecord(node.rawOutput);
  const agent = textValue(input.subagent_type) || textValue(input.subagentType) || textValue(input.agent) || textValue(input.type);
  const description = textValue(input.description) || textValue(input.prompt);
  const sessionId =
    textValue(asRecord(node.rawInput)._sessionId) ||
    textValue(input.sessionId) ||
    textValue(input.sessionID) ||
    textValue(output.sessionId) ||
    textValue(output.sessionID);
  const result = taskResultText(output, node.content.length ? [] : terminalContentTexts(node.content));
  const rows: ToolCallStructuredDetails["rows"] = [];

  addStructuredRow(rows, "Agent", agent);
  addStructuredRow(rows, "Session", sessionId);
  addStructuredRow(rows, "Status", model.statusLabel);

  return {
    kind: "task",
    title: agent ? `Agent task (${agent})` : "Delegated agent task",
    description,
    rows,
    output: result,
    outputLabel: "Result"
  };
}

function backgroundProcessStructuredOutput(value: string | undefined): {
  rows: ToolCallStructuredDetails["rows"];
  output?: string | undefined;
} {
  if (!value) {
    return { rows: [] };
  }
  const rows: ToolCallStructuredDetails["rows"] = [];
  const rest: string[] = [];
  for (const line of value.trimEnd().split("\n")) {
    const match = line.match(/^([a-z_]+):\s*(.*)$/);
    const key = match?.[1];
    if (!key || !BACKGROUND_PROCESS_STRUCTURED_KEYS.has(key)) {
      rest.push(line);
      continue;
    }
    addStructuredRow(rows, BACKGROUND_PROCESS_LABEL[key] || key, match?.[2] || "");
  }
  return {
    rows,
    output: compactStructuredOutput(rest.join("\n"))
  };
}

function taskResultText(output: Record<string, unknown>, contentTexts: string[]): string | undefined {
  const candidates = [
    textValue(output.result),
    textValue(output.output),
    textValue(output.text),
    ...contentTexts
  ].filter((value): value is string => Boolean(value));
  for (const candidate of candidates) {
    const match = /<task_result>\s*([\s\S]*?)\s*<\/task_result>/i.exec(candidate);
    return compactStructuredOutput((match?.[1] || candidate).trim());
  }
  return undefined;
}

function toolOutputText(node: ToolNodeView, rawOutput: Record<string, unknown>): string | undefined {
  return (
    textValue(rawOutput.output) ||
    textValue(rawOutput.stdout) ||
    textValue(rawOutput.stderr) ||
    textValue(rawOutput.last_output) ||
    textValue(rawOutput.lastOutput) ||
    terminalContentTexts(node.content).join("\n")
  );
}

function findStructuredRowValue(rows: ToolCallStructuredDetails["rows"], label: string): string | undefined {
  return rows.find((row) => row.label === label)?.value;
}

function addStructuredRow(rows: ToolCallStructuredDetails["rows"], label: string, value: string | undefined): void {
  const text = compactStructuredValue(value);
  if (!text || rows.some((row) => row.label === label)) {
    return;
  }
  rows.push({ label, value: text });
}

function compactStructuredValue(value: string | undefined): string | undefined {
  const text = value?.trim();
  return text ? redactString(shortLabel(text)) : undefined;
}

function compactStructuredOutput(value: string | undefined): string | undefined {
  const text = value?.trimEnd();
  if (!text?.trim()) {
    return undefined;
  }
  const redacted = redactString(text);
  if (redacted.length <= 4_000) {
    return redacted;
  }
  return `${redacted.slice(0, 2_000)}\n...\n${redacted.slice(-1_800)}`;
}

function textValue(value: unknown): string | undefined {
  if (typeof value === "string") {
    const text = value.trim();
    return text ? text : undefined;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return undefined;
}

function toolNodeShell(node: ToolNodeView, props: ToolCallCardProps): string {
  return `<article id="${nodeDomId(node.id)}" class="turn-card tool tool-${escapeClass(props.model.kind)} tool-risk-${escapeClass(props.model.riskTone)}${props.approval ? " tool-permission" : ""}${props.terminal ? " terminal-tool" : ""}" data-raw-tool-kind="${escapeHtml(node.toolKind)}"><div class="rail"></div><div class="tool-call-react-root" data-tool-call-card-root data-tool-node-id="${escapeHtml(node.id)}"></div></article>`;
}

function toolDisplayStatus(node: ToolNodeView): string {
  if (node.permission?.status === "cancelled" || node.permission?.status === "failed") {
    return node.permission.status;
  }
  return node.toolStatus;
}

function toolCardModelWithApproval(
  model: ToolPresentation,
  permission: ToolPermissionRequest | undefined
): ToolPresentation {
  if (!permission) {
    return model;
  }
  if (permission.status === "pending") {
    return {
      ...model,
      riskTone: model.riskTone === "risk" ? "risk" : "warning",
      riskLabel: "Needs approval",
      statusLabel: approvalStateLabel(permission.status)
    };
  }
  if (permission.status === "cancelled" || permission.status === "failed") {
    return {
      ...model,
      riskTone: "risk",
      riskLabel: approvalStateLabel(permission.status),
      statusLabel: approvalStateLabel(permission.status)
    };
  }
  return {
    ...model,
    statusLabel: approvalStateLabel(permission.status)
  };
}

function toolApprovalProps(
  node: ToolNodeView,
  model: ToolPresentation
): ToolCallApprovalProps | undefined {
  const permission = node.permission;
  if (!permission) {
    return undefined;
  }
  const resolved = permission.status !== "pending";
  const locationCount = (node.locations || []).length;
  const tone = approvalTone({ status: permission.status, toolKind: model.kind });
  const decisionOptions = resolved
    ? []
    : permission.options
        .filter((option) => !isRejectPermissionOption(option))
        .sort((a, b) => approvalOptionSafetyRank(a) - approvalOptionSafetyRank(b));
  const resolvedNote =
    permission.status === "completed"
      ? "Allowed."
      : permission.status === "failed"
        ? "Permission failed."
        : "Rejected.";

  return {
    requestId: permission.requestId,
    status: permission.status,
    statusLabel: approvalStateLabel(permission.status),
    tone,
    resolved,
    title: permission.title || "Permission required",
    resolvedNote,
    impactText: approvalImpactText(model.kind, locationCount),
    actions: resolved
      ? []
      : [
          ...decisionOptions.map((option) => approvalOptionAction(option, permission.status, model.kind)),
          approvalRejectAction(permission.status)
        ]
  };
}

function toolCallCardLocation(location: { path?: string | undefined; line?: number | null | undefined }): ToolCallCardLocation {
  const result: ToolCallCardLocation = { path: String(location.path || "") };
  if (typeof location.line === "number") {
    result.line = location.line;
  }
  return result;
}

function renderedToolContentType(content: ToolCallContent): string {
  const record = asRecord(content);
  if (record.type === "content") {
    return String(asRecord(record.content).type || "");
  }
  return String(record.type || "");
}

function diffNode(node: Extract<RenderNode, { kind: "diff" }>): string {
  const stats = diffStats(node.oldText || "", node.newText);
  diffCardProps.set(node.id, {
    nodeId: node.id,
    path: node.path,
    subtitle: diffSummaryText(stats),
    iconHtml: iconSvg("diff"),
    stats: [{ label: `${stats.oldLineCount} old` }, { label: `${stats.newLineCount} new` }],
    previewHtml: diffPreview({
      path: node.path,
      oldText: node.oldText || "",
      newText: node.newText,
      nodeId: node.id,
      title: "Agent file diff"
    })
  });
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card diff">
      <div class="rail"></div>
      <div class="diff-card-react-root" data-diff-card-root data-diff-node-id="${escapeHtml(node.id)}"></div>
    </article>
  `;
}

function terminalNode(node: Extract<RenderNode, { kind: "terminal" }>): string {
  const model = buildTerminalPresentation(node, MAX_TERMINAL_CHARS);
  terminalCardProps.set(node.id, {
    nodeId: node.id,
    status: node.status,
    tone: model.tone,
    title: model.title,
    subtitle: model.command || "Terminal output",
    statusLabel: model.statusLabel,
    iconHtml: iconSvg("terminal"),
    openIconHtml: iconSvg("terminal"),
    rows: terminalTranscriptRows(model)
  });
  return `
    <article id="${nodeDomId(node.id)}" class="turn-card terminal terminal-${escapeClass(node.status)} terminal-tone-${escapeClass(model.tone)}">
      <div class="rail"></div>
      <div class="terminal-card-react-root" data-terminal-card-root data-terminal-node-id="${escapeHtml(node.id)}"></div>
    </article>
  `;
}

function approvalNode(node: Extract<RenderNode, { kind: "approval" }>): string {
  return toolNode(approvalAsToolNode(node));
}

function approvalAsToolNode(node: ApprovalNodeView): ToolNodeView {
  const permission = permissionFromApproval(node);
  const toolStatus =
    node.status === "completed" || node.status === "cancelled" || node.status === "failed"
      ? node.status
      : node.tool.toolStatus;
  return {
    ...node.tool,
    id: node.id,
    taskId: node.taskId,
    lane: node.lane,
    turnId: node.turnId,
    acpSessionId: node.acpSessionId,
    provider: node.provider,
    source: node.source,
    status: node.status,
    createdAt: node.createdAt || node.tool.createdAt,
    updatedAt: node.updatedAt,
    toolStatus,
    permission
  };
}

function permissionFromApproval(node: ApprovalNodeView): ToolPermissionRequest {
  const permission: ToolPermissionRequest = {
    requestId: node.requestId,
    title: node.title,
    status: node.status,
    options: node.options,
    raw: node.raw
  };
  if (node.provider) {
    permission.provider = node.provider;
  }
  if (node.createdAt) {
    permission.createdAt = node.createdAt;
  }
  if (node.updatedAt) {
    permission.updatedAt = node.updatedAt;
  }
  return permission;
}

function isRejectPermissionOption(option: { optionId: string; label: string }): boolean {
  const value = `${option.optionId} ${option.label}`.toLowerCase();
  return /\b(reject|deny|decline|cancel|refuse|disallow)\b/.test(value);
}

function approvalOptionSafetyRank(option: { optionId: string; label: string }): number {
  const value = `${option.optionId} ${option.label}`.toLowerCase();
  if (/\b(always|forever|persist)\b/.test(value)) {
    return 1;
  }
  if (/\b(allow|approve|once)\b/.test(value)) {
    return 0;
  }
  return 2;
}

function approvalOptionAction(
  option: { optionId: string; label: string; description?: string | undefined },
  status: string,
  toolKind: ToolPresentation["kind"]
): ToolCallApprovalAction {
  const rawLabel = String(option.label || option.optionId || "Allow").trim();
  const label = approvalActionLabel(rawLabel, option.optionId);
  const description = String(option.description || rawLabel || "").trim() || approvalDecisionDescription(toolKind);
  const tone = approvalDecisionTone({ status, toolKind });
  return {
    kind: "approve",
    optionId: option.optionId,
    label,
    description,
    tone,
    disabled: status !== "pending"
  };
}

function approvalRejectAction(status: string): ToolCallApprovalAction {
  return {
    kind: "reject",
    label: "Reject",
    description: "Do not allow this action.",
    tone: "risk",
    disabled: status !== "pending"
  };
}

function checkpointNode(node: Extract<RenderNode, { kind: "checkpoint" }>): string {
  return auditEventNode({
    id: node.id,
    className: "checkpoint",
    rail: "rail-dot",
    variant: "checkpoint",
    presentation: buildEventPresentation({
      kind: "checkpoint",
      label: node.label,
      checkpointId: node.checkpointId,
      updatedAt: node.updatedAt ? formatTimestamp(node.updatedAt) : undefined
    })
  });
}

function completionNode(node: Extract<RenderNode, { kind: "completion" }>): string {
  return auditEventNode({
    id: node.id,
    className: `completion completion-${escapeClass(node.status)}`,
    rail: node.status === "pending" ? "" : "rail-dot",
    presentation: buildEventPresentation({
      kind: "completion",
      label: node.label,
      status: node.status,
      stopReason: node.stopReason,
      checkpointPending: node.checkpointPending
    })
  });
}

function activityNode(title: string, detail: string, kind: string): string {
  return `
    <article class="turn-card activity ${escapeClass(kind)}">
      <div class="rail"></div>
      <div class="card-body compact event-card event-info">
        <span class="activity-title">${escapeHtml(title)}</span>
        <span class="event-detail">${escapeHtml(detail)}</span>
      </div>
    </article>
  `;
}

function usageNode(node: Extract<RenderNode, { kind: "usage" }>): string {
  const pct = node.size > 0 ? Math.min(100, Math.round((node.used / node.size) * 100)) : 0;
  const tone = pct >= 90 ? "risk" : pct >= 70 ? "warning" : "info";
  const cost = node.cost ? eventCostLabel(node.cost) : "";
  const event = buildEventPresentation({
    kind: "usage",
    used: node.used,
    size: node.size,
    costLabel: cost
  });
  return auditEventNode({
    id: node.id,
    className: "usage activity",
    presentation: event,
    meterHtml: `<div class="event-meter">
      <progress class="meter ${tone === "risk" ? "risk" : tone === "warning" ? "review" : "ok"}" value="${pct}" max="100" aria-label="Context usage ${pct}%"></progress>
      ${cost ? `<span>${escapeHtml(cost)}</span>` : ""}
    </div>`
  });
}

function modeNode(node: Extract<RenderNode, { kind: "mode" }>): string {
  const active = node.availableModes.find((mode) => mode.id === node.modeId);
  const detail = active?.description || active?.name || node.modeId;
  const chips = [
    `<span class="event-chip active">Mode ${escapeHtml(active?.name || node.modeId)}</span>`,
    ...node.availableModes
      .filter((mode) => mode.id !== node.modeId)
      .slice(0, 4)
      .map((mode) => `<span class="event-chip">${escapeHtml(mode.name || mode.id)}</span>`)
  ];
  return eventNode({
    id: node.id,
    className: "mode",
    presentation: buildEventPresentation({
      kind: "mode",
      label: detail,
      modeName: active?.name || node.modeId,
      modeCount: node.availableModes.length
    }),
    chips
  });
}

function configNode(node: Extract<RenderNode, { kind: "config" }>): string {
  const chips = node.configOptions.slice(0, 6).map((option) => {
    const value = option.currentValue === undefined || option.currentValue === null ? "unset" : optionValueLabel(option, String(option.currentValue));
    return `<span class="event-chip"><b>${escapeHtml(option.name || option.id)}</b>${escapeHtml(shortLabel(String(value)))}</span>`;
  });
  if (node.configOptions.length > chips.length) {
    chips.push(`<span class="event-chip">${node.configOptions.length - chips.length} more</span>`);
  }
  return eventNode({
    id: node.id,
    className: "config",
    presentation: buildEventPresentation({
      kind: "config",
      configCount: node.configOptions.length
    }),
    chips
  });
}

function commandsNode(node: Extract<RenderNode, { kind: "commands" }>): string {
  const chips = node.availableCommands.slice(0, 8).map((command) => {
    const hint = asRecord(command.input).hint;
    return `<span class="event-chip" title="${escapeHtml(command.description || "")}"><b>/${escapeHtml(command.name)}</b>${hint ? escapeHtml(shortLabel(String(hint))) : ""}</span>`;
  });
  if (node.availableCommands.length > chips.length) {
    chips.push(`<span class="event-chip">${node.availableCommands.length - chips.length} more</span>`);
  }
  return eventNode({
    id: node.id,
    className: "commands",
    presentation: buildEventPresentation({
      kind: "commands",
      commandCount: node.availableCommands.length
    }),
    chips
  });
}

function sessionNode(node: Extract<RenderNode, { kind: "session" }>): string {
  const when = node.sessionUpdatedAt ? formatTimestamp(node.sessionUpdatedAt) : node.updatedAt ? formatTimestamp(node.updatedAt) : "";
  return eventNode({
    id: node.id,
    className: "session",
    presentation: buildEventPresentation({
      kind: "session",
      sessionId: node.acpSessionId,
      sessionTitle: node.title || undefined,
      updatedAt: when || undefined
    })
  });
}

function eventNode({
  id,
  className,
  presentation,
  chips
}: {
  id: string;
  className: string;
  presentation: EventPresentation;
  chips?: string[] | undefined;
}): string {
  return auditEventNode({ id, className: `activity ${escapeClass(className)}`, presentation, chips });
}

function auditEventNode({
  id,
  className,
  presentation,
  chips,
  rail = "",
  meterHtml = "",
  contentHtml = "",
  rawDetails,
  variant
}: {
  id: string;
  className: string;
  presentation: EventPresentation;
  chips?: string[] | undefined;
  rail?: string | undefined;
  meterHtml?: string | undefined;
  contentHtml?: string | undefined;
  rawDetails?: RawDetailsView | undefined;
  variant?: EventCardProps["variant"] | undefined;
}): string {
  eventCardProps.set(id, {
    nodeId: id,
    tone: presentation.tone,
    iconHtml: iconSvg(presentation.icon as IconName),
    title: presentation.title,
    detail: presentation.detail,
    statusLabel: presentation.statusLabel,
    facts: eventCardFacts(presentation.facts),
    chipsHtml: chips?.join("") || "",
    callout: presentation.callout,
    actions: eventCardActions(presentation.actions || []),
    meterHtml,
    contentHtml,
    rawDetails,
    variant,
    defaultOpen: presentation.openByDefault
  });
  return `<article id="${nodeDomId(id)}" class="turn-card ${escapeHtml(className)} audit-event-node"><div class="rail ${escapeHtml(rail)}"></div><div class="event-card-react-root" data-event-card-root data-event-node-id="${escapeHtml(id)}"></div></article>`;
}

function eventCardFacts(facts: EventFact[]): EventCardFact[] {
  return facts.map((fact) => ({
    label: fact.label,
    value: fact.value,
    shortValue: shortLabel(fact.value),
    active: fact.active === true
  }));
}

function eventCardActions(actions: EventAction[]): EventCardAction[] {
  return actions.map((action) => {
    const icon = action.action === "copyCheckpoint" ? "copy" : action.action === "rewind" ? "rewind" : "message";
    return {
      action: action.action,
      label: action.label,
      tone: action.tone,
      target: action.target,
      iconHtml: iconSvg(icon)
    };
  });
}

function unknownNode(node: Extract<RenderNode, { kind: "unknown" }>): string {
  const presentation = buildEventPresentation({ kind: "unknown", label: node.label });
  return auditEventNode({
    id: node.id,
    className: "unknown",
    presentation,
    rawDetails: rawDetailsView(`${node.id}-raw`, node.payload)
  });
}

function renderContentBlocks(blocks: unknown[]): string {
  return blocks.map((block) => renderContentBlock(block)).join("");
}

function contentBlocksToPlainText(blocks: unknown[]): string {
  return blocks
    .map((block) => {
      const record = asRecord(block);
      if (record.type === "text") {
        return textContentValue(record) ?? "";
      }
      if (record.type === "resource_link") {
        return String(record.title || record.name || record.uri || "resource");
      }
      if (record.type === "resource") {
        const resource = asRecord(record.resource);
        return String(resource.uri || resource.text || "resource");
      }
      return String(record.type || "");
    })
    .filter(Boolean)
    .join(" ");
}

function renderContentBlock(block: unknown): string {
  const record = asRecord(block);
  const type = typeof record.type === "string" ? record.type : "unknown";
  if (type === "text") {
    return markdownText(textContentValue(record) ?? "");
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
    return inlineActions({
      ariaLabel: "Resource link actions",
      className: "resource-link-actions",
      actions: [
        {
          action: "openResource",
          className: "resource-chip chip-button",
          data: { uri },
          detail,
          label,
          tone: "provider"
        }
      ]
    });
  }
  if (type === "resource") {
    const resource = asRecord(record.resource);
    const uri = String(resource.uri || "");
    const mime = String(resource.mimeType || "text/plain");
    const text = typeof resource.text === "string" ? truncateText(resource.text, MAX_TEXT_CHARS) : undefined;
    const blob = typeof resource.blob === "string" ? resource.blob : undefined;
    const summaryParts = [uri || "embedded resource", mime].filter(Boolean);
    return payloadDisclosure({
      className: "resource",
      label: summaryParts.join(" - "),
      bodyHtml: `${uri ? inlineActions({
        ariaLabel: "Resource actions",
        actions: [
          {
            action: "openResource",
            data: { uri },
            label: "Open source",
            tone: "provider"
          }
        ]
      }) : ""}${
        text
          ? `<p class="muted">${lineCount(text.text)} lines${text.truncated ? " - preview truncated" : ""}</p>${codeBlock(text.text, { language: languageForResource(uri, mime), title: uri || "Embedded resource" })}${text.truncated ? `<p class="muted">Truncated at ${MAX_TEXT_CHARS} chars.</p>` : ""}`
          : embeddedBinaryResourcePreview(uri, mime, blob)
      }`
    });
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
  return `<p class="muted">Binary resource, ${blob.length} encoded chars.</p>`;
}

function mediaPreviewBlock(kind: "image" | "audio", mime: string, data: string, label: string, open: boolean): string {
  const src = `data:${escapeHtml(mime)};base64,${escapeHtml(data)}`;
  const preview =
    kind === "image"
      ? `<img class="inline-image" src="${src}" alt="${escapeHtml(label)}">`
      : `<audio controls src="${src}"></audio>`;
  return payloadDisclosure({
    className: "media-preview",
    label,
    defaultOpen: open,
    bodyHtml: `
      ${inlineActions({
        ariaLabel: `${kind} preview actions`,
        actions: [
          {
            action: "openMediaPreview",
            ariaLabel: `Open ${kind} preview`,
            iconHtml: iconSvg("open"),
            iconOnly: true,
            label: `Open ${kind} preview`,
            tone: "provider"
          }
        ]
      })}
      ${preview}
    `
  });
}

function renderToolContent(content: unknown, tool?: ToolNodeView, effectiveKind?: ToolPresentation["kind"]): string {
  const record = asRecord(content);
  const type = typeof record.type === "string" ? record.type : "unknown";
  if (type === "content") {
    return renderToolContentBlock(record.content, tool, effectiveKind || tool?.toolKind);
  }
  if (type === "diff") {
    const path = String(record.path || primaryToolPath(tool) || "Tool diff");
    const oldText = String(record.oldText || "");
    const newText = String(record.newText || "");
    return diffPreview({
      path,
      oldText,
      newText,
      title: "Tool diff",
      compact: effectiveKind === "edit"
    });
  }
  if (type === "terminal") {
    return `<div class="terminal-inline">${terminalPreview(record)}</div>`;
  }
  return unsupportedContent(`Unsupported tool content: ${type}`, content);
}

function renderToolContentBlock(block: unknown, tool: ToolNodeView | undefined, effectiveKind: string | undefined): string {
  const record = asRecord(block);
  const type = typeof record.type === "string" ? record.type : "unknown";
  const path = primaryToolPath(tool);
  if ((effectiveKind === "read" || effectiveKind === "edit") && type === "text") {
    if (effectiveKind === "edit" && !path) {
      return renderContentBlock(block);
    }
    const title = path || tool?.title || "Read result";
    return filePreviewBlock(
      buildFilePreviewModel({
        path: title,
        language: languageForResource(title, "text/plain"),
        text: textContentValue(record) ?? "",
        maxChars: MAX_TEXT_CHARS
      })
    );
  }
  return renderContentBlock(block);
}

function filePreviewBlock(model: FilePreviewModel): string {
  const markdown = model.language === "markdown";
  const text = model.language === "text" || model.language === "plaintext";
  const body =
    markdown
      ? `<div class="file-document markdown" tabindex="0">${markdownText(model.text)}</div>`
      : text
        ? `<pre class="file-document" tabindex="0">${escapeHtml(model.text)}</pre>`
        : codeBlock(model.text, {
            language: model.language,
            title: model.title,
            meta: model.metaLabel,
            copyLabel: "Copy file preview",
            openPath: filePreviewOpenPath(model.title)
          });
  return `
    <section class="file-preview file-preview-${model.truncated ? "truncated" : "ready"}" data-highlight-capable="${model.highlightSupported ? "true" : "false"}" aria-label="${escapeHtml(model.accessibilityLabel)}">
      ${body}
      ${model.truncated ? `<p class="muted">Preview truncated before Shiki highlighting to keep the chat responsive.</p>` : ""}
    </section>
  `;
}

function filePreviewOpenPath(title: string): string {
  return /[/\\]/.test(title) || pathFromToolTitle(title) === title ? title : "";
}

function primaryToolPath(tool: ToolNodeView | undefined): string {
  const location = tool?.locations.find((candidate) => candidate.path);
  if (location?.path) {
    return location.path;
  }
  const rawInput = toolArgumentRecord(asRecord(tool?.rawInput));
  return (
    stringChoice(rawInput, ["path", "file", "filePath", "file_path", "filename", "target", "targetPath", "target_path"]) ||
    pathFromToolTitle(tool?.title)
  );
}

function pathFromToolTitle(title: string | undefined): string {
  return String(title || "").match(/[^\s()'"]+\.(?:bash|cjs|css|go|html?|jsx?|jsonc?|lock|mdx?|mjs|[mc]?ts|py|rs|sh|text|tsx?|txt|xml|ya?ml|zsh)\b/i)?.[0] || "";
}

function terminalToolIntent(tool: ToolNodeView, presentation: ToolPresentation): string {
  const data = terminalToolData(tool, presentation);
  const parsed = terminalCommandParts(data.command);
  const title = shellCommentText(tool.title);
  const genericTitle = /^(bash|shell|terminal|execute|command|run)$/i.test(String(tool.title || "").trim());
  return shortLabel(parsed.intent || title || (genericTitle ? "" : tool.title) || parsed.command || presentation.summary || "Run shell command");
}

function terminalToolPreview(tool: ToolNodeView, presentation: ToolPresentation): string {
  const data = terminalToolData(tool, presentation);
  const base: Record<string, unknown> = {
    ...data.rawInput,
    ...data.rawOutput,
    terminalId: tool.toolCallId,
    title: tool.title,
    status: tool.toolStatus,
    command: data.command
  };
  if (data.contentOutput && !terminalHasOutput(base)) {
    base.output = data.contentOutput;
  }
  if (data.terminalBlocks.length) {
    return data.terminalBlocks
      .map((block) => terminalPreviewFromModel(buildTerminalPresentation(terminalBlockPreviewInput(base, block, data), MAX_TERMINAL_CHARS)))
      .join("");
  }
  return terminalPreviewFromModel(buildTerminalPresentation(base, MAX_TERMINAL_CHARS));
}

function terminalBlockPreviewInput(
  base: Record<string, unknown>,
  block: Record<string, unknown>,
  data: Pick<ReturnType<typeof terminalToolData>, "command" | "contentOutput">
): Record<string, unknown> {
  const input = { ...base, ...block };
  const command = terminalCommand(input) || "";
  if (!command || isCountSummary(command)) {
    input.command = data.command;
  }
  if (data.contentOutput && !terminalHasOutput(input)) {
    input.output = data.contentOutput;
  }
  return input;
}

function terminalToolData(tool: ToolNodeView, presentation: ToolPresentation): {
  rawInput: Record<string, unknown>;
  rawOutput: Record<string, unknown>;
  terminalBlocks: Record<string, unknown>[];
  command: string;
  contentOutput: string;
} {
  const terminalBlocks = tool.content.map(asRecord).filter((block) => block.type === "terminal");
  const rawInput = toolArgumentRecord(asRecord(tool.rawInput));
  const rawOutput = asRecord(tool.rawOutput);
  const texts = terminalContentTexts(tool.content);
  const parsedTexts = texts.map(terminalCommandFromText);
  const textCommand = parsedTexts.find((parsed) => parsed.command) || parsedTexts[0] || { command: "", output: "", intent: "" };
  const textIntent = parsedTexts.find((parsed) => parsed.intent)?.intent || "";
  const command =
    terminalCommand(rawInput) ||
    terminalCommand(rawOutput) ||
    terminalBlocks.map(terminalCommand).find(Boolean) ||
    textCommand.command ||
    textIntent ||
    (isCountSummary(presentation.summary) ? "" : presentation.summary) ||
    (/^(bash|shell|terminal|execute|command|run)$/i.test(tool.title) ? "" : tool.title) ||
    "Command not provided";
  const outputParts = parsedTexts.map((parsed) => parsed.output).filter(Boolean);
  return { rawInput, rawOutput, terminalBlocks, command, contentOutput: outputParts.join("\n") };
}

function terminalContentTexts(content: unknown[]): string[] {
  return content
    .map((blockValue) => {
      const block = asRecord(blockValue);
      if (block.type !== "content") {
        return "";
      }
      const contentBlock = asRecord(block.content);
      if (contentBlock.type === "text") {
        return (textContentValue(contentBlock) ?? "").trim();
      }
      const resource = asRecord(contentBlock.resource);
      return typeof resource.text === "string" ? resource.text.trim() : "";
    })
    .filter(Boolean);
}

function terminalCommandFromText(text: string): { command: string; output: string; intent: string } {
  const fenced = terminalFenceBlock(text);
  if (fenced) {
    const preamble = terminalTextPreamble(fenced.before);
    const fencedBody = terminalCommandFromLines(fenced.body.split("\n"));
    return {
      command: preamble.command || fencedBody.command,
      output: fencedBody.output || fenced.body.trim(),
      intent: preamble.intent
    };
  }
  const lines = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
  while (lines.length && !lines[0]?.trim()) {
    lines.shift();
  }
  return terminalCommandFromLines(lines);
}

function terminalFenceBlock(text: string): { before: string; body: string } | undefined {
  const match = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").match(/```(?:console|terminal|shell|bash|sh|zsh|text)?\n([\s\S]*?)```/i);
  return match?.index === undefined ? undefined : { before: text.slice(0, match.index), body: match[1] || "" };
}

function terminalCommandFromLines(lines: string[]): { command: string; output: string; intent: string } {
  const working = [...lines];
  while (working.length && !working[0]?.trim()) {
    working.shift();
  }
  const first = working[0]?.trim() || "";
  const second = working[1]?.trim() || "";
  if (/^[$>]\s+/.test(first)) {
    return { command: first.replace(/^[$>]\s+/, ""), output: working.slice(1).join("\n").trim(), intent: "" };
  }
  if (shellCommentText(first) && second) {
    return { command: `${first}\n${second.replace(/^[$>]\s+/, "")}`, output: working.slice(2).join("\n").trim(), intent: shellCommentText(first) };
  }
  if (looksLikeShellCommand(first)) {
    return { command: first, output: working.slice(1).join("\n").trim(), intent: "" };
  }
  return { command: "", output: looksLikeTerminalOutput(working) ? working.join("\n").trim() : "", intent: terminalIntentFromLines(working) };
}

function terminalTextPreamble(text: string): { command: string; intent: string } {
  const parsed = terminalCommandFromLines(text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n"));
  return { command: parsed.command, intent: parsed.intent || terminalIntentFromLines(text.split(/\r?\n/)) };
}

function terminalIntentFromLines(lines: string[]): string {
  const seen = new Set<string>();
  const cleaned = lines
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("```"))
    .filter((line) => {
      const key = line.toLowerCase();
      if (seen.has(key)) {
        return false;
      }
      seen.add(key);
      return true;
    });
  const first = cleaned.find((line) => !looksLikeShellCommand(line));
  return first ? shellCommentText(first) || first : "";
}

function looksLikeTerminalOutput(lines: string[]): boolean {
  const text = lines.join("\n").trim();
  return (
    lines.length > 2 ||
    /^\s/.test(lines[0] || "") ||
    /(?:^|\n)\s*(?:error|warning|failed|passed|success|total)\b/i.test(text) ||
    /(?:https?:\/\/|[│┌└├─]{2,}|^\d+\s+\w+)/m.test(text)
  );
}

function looksLikeShellCommand(value: string): boolean {
  return /^(?:\.\/|\/|[A-Z_][A-Z0-9_]*=|(?:npm|pnpm|yarn|bun|npx|node|git|rg|grep|find|ls|cat|sed|awk|python3?|pytest|go|cargo|make|bash|sh|zsh|cd|mkdir|rm|cp|mv|curl|docker|kubectl|tsc|crabdb)\b)/.test(value) || /\s(?:--?[A-Za-z0-9]|&&|\|\s|2>)/.test(value);
}

function terminalHasOutput(record: Record<string, unknown>): boolean {
  return Boolean(stringChoice(record, ["output", "stdout", "stdoutPreview", "stdout_preview", "stderr", "stderrPreview", "stderr_preview"]));
}

function isCountSummary(value: string): boolean {
  return /^\d+\s+(?:output|outputs|terminal preview|terminal previews)$/.test(value);
}

function terminalPreview(value: unknown, nodeId?: string): string {
  return terminalPreviewFromModel(buildTerminalPresentation(asRecord(value), MAX_TERMINAL_CHARS), nodeId);
}

function terminalTranscriptRows(model: TerminalPresentation): TerminalTranscriptRow[] {
  const command = terminalCommandParts(model.command || model.title).command || model.title;
  const rows: TerminalTranscriptRow[] = [
    {
      id: "command",
      kind: "in",
      label: "IN",
      title: "Command",
      detail: model.cwd || "shell",
      textHtml: escapeHtml(command),
      language: "shellscript",
      meta: model.cwd,
      tone: "muted",
      truncated: false,
      empty: false,
      openByDefault: true
    }
  ];

  if (!model.sections.length) {
    rows.push({
      id: "empty-output",
      kind: "out",
      label: "OUT",
      title: "Output",
      detail: "empty",
      textHtml: escapeHtml(model.emptyText),
      tone: "muted",
      truncated: false,
      empty: true,
      openByDefault: true
    });
    return rows;
  }

  for (const section of model.sections) {
    rows.push({
      id: `section-${section.id}`,
      kind: section.id === "stderr" ? "err" : "out",
      label: section.id === "stderr" ? "ERR" : "OUT",
      title: section.label,
      detail: `${new Intl.NumberFormat("en-US").format(section.lineCount)} line${section.lineCount === 1 ? "" : "s"}`,
      textHtml: renderAnsiText(section.text),
      tone: section.tone,
      truncated: section.truncated,
      empty: false,
      openByDefault: section.openByDefault
    });
  }
  return rows;
}

function terminalPreviewFromModel(model: TerminalPresentation, nodeId?: string): string {
  const openTerminal = nodeId
    ? inlineActions({
        ariaLabel: "Terminal transcript actions",
        className: "terminal-transcript-actions",
        actions: [
          {
            action: "openTerminal",
            ariaLabel: "Open terminal",
            data: { "node-id": nodeId },
            iconHtml: iconSvg("terminal"),
            iconOnly: true,
            label: "Open terminal",
            tone: "provider"
          }
        ]
      })
    : "";
  const command = terminalCommandParts(model.command || model.title).command || model.title;
  const rows = [
    terminalTranscriptRow("in", "IN", command, { language: "shellscript", meta: model.cwd }),
    ...(model.sections.length
      ? model.sections.map((section) =>
          terminalTranscriptRow(section.id === "stderr" ? "err" : "out", section.id === "stderr" ? "ERR" : "OUT", section.text, {
            truncated: section.truncated,
            tone: section.tone
          })
        )
      : [terminalTranscriptRow("out", "OUT", model.emptyText, { empty: true })])
  ].join("");
  return `<div class="terminal-transcript terminal-tone-${escapeClass(model.tone)}">${rows}${openTerminal}</div>`;
}

function terminalTranscriptRow(
  kind: "in" | "out" | "err",
  label: string,
  text: string,
  options: { language?: string | undefined; meta?: string | undefined; tone?: string | undefined; truncated?: boolean | undefined; empty?: boolean | undefined } = {}
): string {
  const attrs = options.language ? ` data-highlight-language="${escapeHtml(options.language)}"` : "";
  const content = options.language ? escapeHtml(text) : renderAnsiText(text);
  const note = options.truncated ? `<small class="terminal-transcript-note">truncated at ${MAX_TERMINAL_CHARS.toLocaleString()} chars</small>` : "";
  const meta = options.meta ? `<small class="terminal-transcript-note">${escapeHtml(options.meta)}</small>` : "";
  return `<div class="terminal-transcript-row terminal-transcript-${kind} terminal-tone-${escapeClass(options.tone || "muted")}"><span class="terminal-transcript-label">${escapeHtml(label)}</span><div class="terminal-transcript-cell"><pre class="terminal-transcript-code code${options.empty ? " terminal-transcript-empty" : ""}"${attrs} tabindex="0">${content}</pre>${meta}${note}</div></div>`;
}

function terminalCommandParts(value: string): { intent: string; command: string } {
  const lines = value.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
  const intentLines: string[] = [];
  while (lines.length && shellCommentText(lines[0])) {
    intentLines.push(shellCommentText(lines.shift()) || "");
  }
  const command = lines.join("\n").trim() || value.trim();
  return { intent: intentLines.join(" ").trim(), command };
}

function shellCommentText(value: string | undefined): string {
  const match = String(value || "").trim().match(/^#\s+(.+)$/);
  return match?.[1]?.trim() || "";
}

function renderAnsiText(value: string): string {
  const input = value.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const pattern = /\x1b\[([0-9;]*)m/g;
  let cursor = 0;
  let color = "";
  let html = "";
  const append = (text: string) => {
    if (!text) {
      return;
    }
    const escaped = escapeHtml(text);
    html += color ? `<span class="ansi-fg-${color}">${escaped}</span>` : escaped;
  };
  for (let match = pattern.exec(input); match; match = pattern.exec(input)) {
    append(input.slice(cursor, match.index));
    cursor = match.index + match[0].length;
    color = ansiColor(String(match[1] || "0"));
  }
  append(input.slice(cursor));
  return html;
}

function ansiColor(value: string): string {
  const code = Number(value.split(";").pop() || 0);
  if (code === 0 || code === 39) {
    return "";
  }
  const colors = ["muted", "red", "green", "yellow", "blue", "magenta", "cyan"];
  return (code >= 30 && code <= 36 ? colors[code - 30] : code >= 90 && code <= 96 ? colors[code - 90] : "") || "";
}

async function copyCheckpoint(action: HTMLElement): Promise<void> {
  await copyTextToClipboard(action.dataset.target || "", "checkpoint id", "Copied checkpoint id.");
}

async function copyTimelineGroupId(action: HTMLElement): Promise<void> {
  await copyTextToClipboard(action.dataset.target || "", "turn id", "Copied turn ID.");
}

function focusToolDiff(action: HTMLElement): void {
  const card = action.closest<HTMLElement>(".tool-card");
  if (!card) {
    return;
  }
  const diff = card.querySelector<HTMLElement>(".diff-preview");
  if (!diff) {
    return;
  }
  diff.setAttribute("tabindex", "-1");
  diff.scrollIntoView({ block: "nearest", inline: "nearest" });
  diff.focus();
}

async function copyTextToClipboard(text: string, label: string, successMessage: string): Promise<void> {
  if (!text.length) {
    announceToast(`No ${label} available to copy.`, "error");
    return;
  }
  try {
    if (!navigator.clipboard?.writeText) {
      throw new Error("Clipboard API unavailable");
    }
    await navigator.clipboard.writeText(text);
    announceToast(successMessage, "status");
  } catch {
    fallbackCopy(text, label);
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
  let copied = false;
  try {
    copied = document.execCommand("copy");
  } catch {
    copied = false;
  }
  textarea.remove();
  announceToast(copied ? `Copied ${label}.` : `Unable to copy ${label}.`, copied ? "status" : "error");
}

async function copyCode(action: HTMLElement): Promise<void> {
  const text = codeTextForAction(action);
  await copyTextToClipboard(text, "preview text", "Copied preview text.");
}

async function copyDiff(action: HTMLElement): Promise<void> {
  const text = diffTextForAction(action);
  await copyTextToClipboard(text, "unified diff", "Copied unified diff.");
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

function openDiffPreview(action: HTMLElement): void {
  const text = diffTextForAction(action);
  if (!text) {
    return;
  }
  vscode.postMessage({
    type: "openTextPreview",
    text,
    title: action.dataset.title || "CrabDB unified diff",
    language: action.dataset.language || "diff"
  });
}

function selectDiffReviewFile(path: string): void {
  if (!path) {
    return;
  }
  const drawer = document.querySelector<HTMLElement>(".diff-review-drawer");
  if (!drawer) {
    return;
  }
  drawer.querySelectorAll<HTMLElement>("[data-diff-review-path]").forEach((element) => {
    const active = element.dataset.diffReviewPath === path;
    element.classList.toggle("active", active);
    if (element instanceof HTMLButtonElement) {
      element.setAttribute("aria-pressed", active ? "true" : "false");
    }
  });
  drawer.querySelectorAll<HTMLElement>("[data-diff-review-file]").forEach((element) => {
    const active = element.dataset.diffReviewFile === path;
    element.hidden = !active;
    element.classList.toggle("active", active);
    if (active) {
      element.scrollIntoView({ block: "nearest", inline: "nearest" });
    }
  });
}

function insertDiffSuggestion(action: HTMLElement): void {
  const command = action.dataset.command || "";
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  if (!command || !input) {
    return;
  }
  insertComposerText(input, command);
  closeJsonDrawer({ restoreFocus: false });
  announceToast("Command inserted in composer.", "status");
}

function diffTextForAction(action: HTMLElement): string {
  const template = action.closest(".diff-preview")?.querySelector<HTMLTemplateElement>("template.diff-source");
  return template?.content.textContent || "";
}

function openMediaPreview(action: HTMLElement): void {
  const preview = action.closest(".media-preview");
  const image = preview?.querySelector<HTMLImageElement>("img.inline-image");
  const audio = preview?.querySelector<HTMLAudioElement>("audio");
  const media = image || audio;
  if (!media?.src) {
    announceToast("No media preview is available.", "status");
    return;
  }
  const label = preview?.querySelector(".payload-summary")?.textContent || "Media preview";
  prepareJsonDrawer();
  const drawer = document.createElement("section");
  drawer.className = "json-drawer media-drawer";
  configureJsonDrawer(drawer, label);
  const closeActions = inlineActions({
    ariaLabel: "Media preview drawer actions",
    className: "media-drawer-actions",
    actions: [
      {
        action: "closeDrawer",
        ariaLabel: "Close media preview",
        iconHtml: iconSvg("close"),
        iconOnly: true,
        label: "Close media preview",
        tone: "provider"
      }
    ]
  });
  drawer.innerHTML = `
    <div class="drawer-header">
      <h2>${escapeHtml(label)}</h2>
      ${closeActions}
    </div>
    ${
      image
        ? `<img class="media-full" src="${escapeHtml(media.src)}" alt="${escapeHtml(label)}">`
        : `<audio class="media-full-audio" controls src="${escapeHtml(media.src)}"></audio>`
    }
  `;
  mountJsonDrawer(drawer);
  void hydrateInlineActions().then(() => drawer.querySelector<HTMLElement>("[data-action='closeDrawer']")?.focus());
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

function resourceBlock(node: Extract<RenderNode, { kind: "resource" }>): string {
  const label = resourceBlockLabel(node.content);
  const presentation = buildEventPresentation({
    kind: "resource",
    resourceLabel: label
  });
  return auditEventNode({
    id: node.id,
    className: "resource-node",
    presentation,
    contentHtml: `<div class="event-content">${renderContentBlock(node.content)}</div>`
  });
}

function composer(): string {
  const attachments = state.attachments || [];
  const controlsDisabled = Boolean(state.sending || state.permissionPending);
  const disabled = controlsDisabled ? "disabled" : "";
  const usage = currentUsageNode();
  const status = composerStatus(attachments);
  const draftState = composerDraftState(composerDraft, attachments.length);
  const sendBlockedReason = composerSendBlockedReason(Boolean(composerDraft.trim()), attachments.length, draftState.chars);
  const placeholder = composerPlaceholder(status);
  const provider = currentProviderProfile();
  const railItems = composerRailItems({
    statusTone: status.tone,
    statusLabel: status.label,
    attachmentModes: attachments.map(attachmentMode),
    sendMode: composerSendMode,
    providerCrabdbBacked: provider?.crabdbBacked
  });
  composerCardProps = {
    id: "composer",
    status,
    draft: {
      tone: draftState.tone,
      label: draftState.label,
      detail: draftState.detail,
      maxChars: draftState.maxChars,
      meterValue: draftState.meterValue,
      meterPercent: draftState.meterPercent
    },
    draftValue: composerDraft,
    placeholder,
    keyShortcuts: composerKeyShortcuts(),
    maxChars: MAX_COMPOSER_DRAFT_CHARS,
    controlsDisabled,
    sendBlockedReason,
    metricsText: composerMetrics(composerDraft, attachments),
    attachments: attachments.map(composerAttachmentView),
    attachmentSummary: attachmentSummary(attachments),
    railItems,
    presets: COMPOSER_PROMPT_PRESETS.map((preset) => ({
      id: preset.id,
      label: preset.label,
      detail: preset.detail,
      iconHtml: iconSvg(preset.icon)
    })),
    sendMode: composerSendMode,
    contextUsageHtml: contextUsageGauge(usage),
    sessionControlsHtml: composerSessionControlsHtml(disabled),
    contextActions: [
      composerIconAction("attachSelection", "Attach the current editor selection", "selection", controlsDisabled),
      composerIconAction("attachFile", "Attach the active file", "file", controlsDisabled),
      composerIconAction("attachDiagnostics", "Attach diagnostics for the active file", "diagnostics", controlsDisabled),
      composerIconAction("attachTerminalOutput", "Attach the latest terminal output from this chat", "terminal", controlsDisabled),
      composerIconAction("attachChangedFiles", "Attach the changed file list for this task", "changed", controlsDisabled),
      composerIconAction("attachHistory", "Attach CrabDB history for the active file", "history", controlsDisabled)
    ],
    rewindIconHtml: iconSvg("rewind"),
    sendIconHtml: iconSvg("send"),
    clearIconHtml: iconSvg("close"),
    settingsIconHtml: iconSvg("settings")
  };
  return `
    <section id="composer" class="composer" aria-label="Prompt composer" tabindex="-1">
      <div class="composer-card-react-root" data-composer-card-root data-composer-id="composer"></div>
    </section>
  `;
}

function composerAttachmentView(attachment: PromptAttachmentView): ComposerCardProps["attachments"][number] {
  return {
    id: attachment.id,
    kind: attachment.kind,
    label: shortLabel(attachment.label),
    mode: attachmentMode(attachment),
    title: `${attachment.kind}: ${attachment.label}`
  };
}

function composerIconAction(
  action: string,
  label: string,
  icon: IconName,
  disabled: boolean
): ComposerCardProps["contextActions"][number] {
  return {
    action,
    label,
    iconHtml: iconSvg(icon),
    disabled
  };
}

function composerKeyShortcuts(): string {
  return composerSendMode === "fast" ? "Enter Control+Enter Meta+Enter" : "Control+Enter Meta+Enter";
}

function composerSessionControlsHtml(disabled: string): string {
  const controls = [providerSelector(disabled), sessionControlSelectors(disabled)].filter(Boolean).join("");
  return controls;
}

function closeComposerControls(): void {
  closeFloatingDetails();
}

function closeFloatingDetails(except?: HTMLElement, restoreFocus = false): boolean {
  let closed = false;
  document.querySelectorAll<HTMLElement>(`${FLOATING_DETAILS_SELECTOR}[data-floating-open="true"]`).forEach((menu) => {
    if (menu === except) {
      return;
    }
    closed = true;
  });
  if (closed) {
    dispatchFloatingMenuClose({ except, restoreFocus });
  }
  return closed;
}

function composerStatus(attachments: PromptAttachmentView[]): ComposerStatus {
  if (state.permissionPending) {
    return {
      tone: "waiting",
      label: "Permission required",
      detail: "Approve or reject the pending tool request before sending another prompt."
    };
  }
  if (state.sending) {
    return {
      tone: "running",
      label: "Agent is working",
      detail: "Watch the transcript, open review, or cancel the current turn from the toolbar."
    };
  }
  if (state.providerFailure) {
    return {
      tone: "warning",
      label: "Follow-up recommended",
      detail: "The last provider turn stopped early. Start a follow-up from CrabDB's latest checkpoint when ready."
    };
  }
  if (attachments.length) {
    return {
      tone: "context",
      label: "Context ready",
      detail: `${attachmentSummary(attachments)} attached to the next prompt.`
    };
  }
  return {
    tone: "ready",
    label: "Ready for the next turn",
    detail: "Ask for code, tests, review, or attach editor context before sending."
  };
}

function composerPlaceholder(status: ComposerStatus): string {
  if (status.tone === "waiting") {
    return "Permission pending";
  }
  if (status.tone === "running") {
    return "Prompt running";
  }
  if (status.tone === "warning") {
    return "Start a follow-up or describe what to do next";
  }
  return "Message agent";
}

function composerSendBlockedReason(hasDraft: boolean, attachmentCount: number, draftChars = Array.from(composerDraft).length): string | undefined {
  return blockedComposerSendReason({
    hasDraft,
    attachmentCount,
    draftChars,
    maxChars: MAX_COMPOSER_DRAFT_CHARS,
    sending: state.sending,
    permissionPending: state.permissionPending
  });
}

function composerMetrics(text: string, attachments: PromptAttachmentView[]): string {
  return formatComposerMetrics(text, attachments.length);
}

function attachmentSummary(attachments: PromptAttachmentView[]): string {
  return attachmentModeSummary(attachments.map(attachmentMode));
}

function reviewDrawer(task: WebviewState["task"]): string {
  const changed = task?.changedPaths || [];
  const taskView = asRecord(state.taskView);
  const review = asRecord(taskView.review);
  const readiness = asRecord(taskView.readiness || review.readiness);
  const turns = arrayField(taskView, "turns");
  const events = arrayField(taskView, "events");
  const changes = arrayField(taskView, "changes").concat(arrayField(review, "changed_paths"));
  const changedFiles = uniqueStrings(changed.concat(changes.map(reviewValueLabel)));
  const blockers = reviewStrings(readiness, ["blockers", "blocking", "failed_gates"]).concat(
    reviewStrings(review, ["blockers", "blocking", "failed_gates"])
  );
  const warnings = reviewStrings(readiness, ["warnings", "stale_base", "risky_files", "ignored_paths"]).concat(
    reviewStrings(review, ["warnings", "stale_base", "risky_files", "ignored_paths"])
  );
  const latestTest = asRecord(review.latest_test);
  const latestEval = asRecord(review.latest_eval);
  const testRuns = arrayField(review, "recent_gates").concat(Object.keys(latestTest).length ? [latestTest] : []);
  const evalRuns = arrayField(review, "recent_evals")
    .concat(arrayField(review, "evals"))
    .concat(arrayField(review, "evaluations"))
    .concat(Object.keys(latestEval).length ? [latestEval] : []);
  const transcriptLinks = transcriptAnchors();
  const coordination = coordinationSummaryFromSources(task, taskView, review, readiness);
  const conflictIds = conflictSetIdsFromSources(readiness, review);
  const overlaps = state.taskOverlaps || [];
  const providerTitle = providerSessionTitle(task?.title);
  const turnCount = transcriptTurnCount() || turns.length;
  const reviewReadiness = buildReviewReadiness({
    taskStatus: task?.status,
    changedPaths: changedFiles.length,
    turnCount,
    eventCount: events.length,
    blockers: blockers.length,
    warnings: warnings.length,
    conflictCount: conflictIds.length,
    overlapCount: overlaps.length,
    testRunCount: testRuns.length,
    evalRunCount: evalRuns.length,
    coordination
  });
  const refreshAction: ReviewAction = { action: "refresh", label: "Refresh", description: "Fetch the latest review state.", tone: "default" };
  const actionIcons = reviewActionIconHtml(reviewReadiness.actionGroups, [refreshAction]);
  const sectionsHtml = [
    `<section class="review-section">
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
        <div><dt>Turns</dt><dd>${turnCount}</dd></div>
        <div><dt>Events</dt><dd>${events.length}</dd></div>
        <div><dt>Changes</dt><dd>${changedFiles.length}</dd></div>
      </dl>
    </section>`,
    `<section class="review-section">
      <h3>Blockers and warnings</h3>
      ${reviewIssueList(blockers, warnings, readinessText(task?.status || "new"))}
    </section>`,
    `<section class="review-section">
      <h3>Coordination</h3>
      ${coordinationPanel(coordination)}
    </section>`,
    overlaps.length
      ? `<section class="review-section">
          <h3>Parallel work</h3>
          ${overlapReviewList(overlaps)}
          ${inlineActions({
            ariaLabel: "Parallel work actions",
            actions: [
              { action: "compareTasks", label: "Compare tasks", tone: "provider" },
              { action: "refresh", label: "Refresh", tone: "lane" },
              { action: "queueMerge", label: "Queue merge", tone: "lane" }
            ]
          })}
        </section>`
      : "",
    conflictIds.length
      ? `<section class="review-section">
          <h3>Conflicts</h3>
          <p class="muted">CrabDB reports ${conflictIds.length} open conflict set${conflictIds.length === 1 ? "" : "s"} for this task.</p>
          ${inlineActions({
            ariaLabel: "Conflict actions",
            className: "conflict-actions",
            actions: conflictIds.map((id) => ({
              action: "showConflict",
              data: { "conflict-id": id },
              label: `Open ${shortLabel(id)}`,
              tone: "review"
            }))
          })}
        </section>`
      : "",
    `<section class="review-section">
      <h3>Tests and evals</h3>
      ${testSummary(taskView, testRuns.concat(evalRuns))}
      ${inlineActions({
        ariaLabel: "Test and eval actions",
        actions: [
          { action: "runTests", label: "Run test", tone: "lane" },
          { action: "runEvals", label: "Run eval", tone: "lane" }
        ]
      })}
    </section>`,
    `<section class="review-section">
      <h3>Diffs</h3>
      ${changedFiles.length ? `<ul>${changedFiles.map((file) => `<li>${locationChip(file)}</li>`).join("")}</ul>` : `<p class="muted">No changed paths recorded yet.</p>`}
    </section>`,
    `<section class="review-section">
      <h3>Transcript</h3>
      ${transcriptLinks.length ? `<ul>${transcriptLinks.map((link) => `<li><a href="#${escapeHtml(link.id)}">${escapeHtml(link.label)}</a></li>`).join("")}</ul>` : `<p class="muted">No persisted turns yet.</p>`}
    </section>`
  ].join("");
  reviewDrawerProps = {
    id: "review",
    readiness: reviewReadiness,
    sectionsHtml,
    actionIcons,
    refreshAction
  };
  return `
    <aside id="review" class="review-drawer" aria-label="Review" tabindex="-1">
      <div class="review-drawer-react-root" data-review-drawer-root data-review-drawer-id="review"></div>
    </aside>
  `;
}

function reviewActionIconHtml(groups: ReviewActionGroup[], extraActions: ReviewAction[] = []): Record<string, string> {
  const icons: Record<string, string> = {};
  groups.flatMap((group) => group.actions).concat(extraActions).forEach((action) => {
    icons[action.action] = iconSvg(reviewActionIcon(action.action));
  });
  return icons;
}

const REVIEW_ACTION_ICONS: Record<string, IconName> = {
  compareTasks: "diff",
  dryRunApply: "selection",
  focusTranscript: "turn",
  openDiff: "diff",
  openWorkdir: "open",
  preserveFailedAttempt: "history",
  queueMerge: "lane",
  refresh: "refresh",
  rewind: "rewind",
  removeTask: "stop",
  runEvals: "review",
  runTests: "check"
};

function reviewActionIcon(action: string): IconName {
  return REVIEW_ACTION_ICONS[action] || "tool";
}

function reviewIssueList(blockers: string[], warnings: string[], fallback: string): string {
  const items = [
    ...blockers.map((item) => ({ tone: "blocked", label: "Blocked", item })),
    ...warnings.map((item) => ({ tone: "warning", label: "Warning", item }))
  ];
  if (!items.length) {
    return `<p class="muted">${escapeHtml(fallback)}</p>`;
  }
  return `<ul class="review-issue-list">${items
    .slice(0, 12)
    .map((issue) => `<li class="review-issue-${escapeClass(issue.tone)}"><span>${escapeHtml(issue.label)}</span>${escapeHtml(issue.item)}</li>`)
    .join("")}</ul>${items.length > 12 ? `<p class="muted">Showing 12 of ${items.length} readiness findings.</p>` : ""}`;
}

function overlapReviewList(overlaps: TaskOverlapView[]): string {
  return `<ul class="overlap-list">${overlaps
    .slice(0, 6)
    .map(
      (overlap) =>
        `<li><strong>${escapeHtml(overlap.title)}</strong><span>${escapeHtml(overlap.lane)} - ${escapeHtml(overlap.status)}${overlap.provider ? ` - ${escapeHtml(overlap.provider)}` : ""}</span><div class="chips">${overlap.sharedPaths.slice(0, 6).map((path) => locationChip(path)).join("")}</div></li>`
    )
    .join("")}</ul>${overlaps.length > 6 ? `<p class="muted">Showing 6 of ${overlaps.length} overlapping tasks.</p>` : ""}`;
}

function sendPrompt(): void {
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  const text = input?.value || "";
  const attachments = state.attachments || [];
  const draftState = composerDraftState(text, attachments.length);
  const sendBlockedReason = composerSendBlockedReason(Boolean(text.trim()), attachments.length, draftState.chars);
  if (sendBlockedReason) {
    announceToast(sendBlockedReason, "status");
    if (!text.trim() || draftState.tone === "limit") {
      focusComposer();
    }
    syncComposerAffordances();
    return;
  }
  vscode.postMessage({ type: "sendPrompt", text });
  composerDraft = "";
  persistWebviewState();
  if (input) {
    input.value = "";
    resizeComposerInput(input);
  }
  syncComposerAffordances();
}

function syncComposerAffordances(): void {
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  const text = input?.value || composerDraft;
  const attachments = state.attachments || [];
  const draftState = composerDraftState(text, attachments.length);
  const reason = composerSendBlockedReason(Boolean(text.trim()), attachments.length, draftState.chars);
  const sendButton = document.querySelector<HTMLButtonElement>('[data-action="send"]');
  if (sendButton) {
    const label = reason || "Send prompt";
    sendButton.disabled = Boolean(reason);
    sendButton.title = label;
    sendButton.setAttribute("aria-label", label);
  }
  const meta = document.querySelector<HTMLElement>("[data-composer-meta]");
  if (meta) {
    meta.textContent = composerMetrics(text, attachments);
  }
  const frame = input?.closest<HTMLElement>(".composer-input-frame");
  if (frame) {
    frame.classList.remove("composer-input-frame-empty", "composer-input-frame-ready", "composer-input-frame-warning", "composer-input-frame-limit");
    frame.classList.add(`composer-input-frame-${draftState.tone}`);
  }
  const draftStateNode = document.getElementById("composer-draft-state");
  const draftLabel = draftStateNode?.querySelector<HTMLElement>(".composer-draft-copy strong");
  const draftDetail = draftStateNode?.querySelector<HTMLElement>(".composer-draft-copy span");
  const meter = draftStateNode?.querySelector<HTMLElement>(".composer-meter");
  if (draftLabel) {
    draftLabel.textContent = draftState.label;
  }
  if (draftDetail) {
    draftDetail.textContent = draftState.detail;
  }
  if (meter) {
    meter.style.setProperty("--composer-meter", `${draftState.meterPercent}%`);
    meter.setAttribute("aria-valuemax", String(draftState.maxChars));
    meter.setAttribute("aria-valuenow", String(draftState.meterValue));
  }
  if (input) {
    if (draftState.tone === "limit") {
      input.setAttribute("aria-invalid", "true");
    } else {
      input.removeAttribute("aria-invalid");
    }
  }
  const hint = document.querySelector<HTMLElement>("[data-composer-empty-reason]");
  if (hint) {
    hint.textContent = reason || "";
    hint.hidden = !reason;
  }
  const clearButton = document.querySelector<HTMLButtonElement>("[data-composer-clear]");
  if (clearButton) {
    clearButton.disabled = !text || Boolean(state.sending || state.permissionPending);
  }
  syncComposerSendModeControls();
}

function resizeComposerInput(input: HTMLTextAreaElement): void {
  input.style.height = "auto";
  input.style.height = `${Math.min(input.scrollHeight, 220)}px`;
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
    ${issues.length ? `<ul>${issues.map((issue) => `<li><span class="coordination-issue-tone coordination-${escapeClass(issue.tone)}">${escapeHtml(issue.tone)}</span> ${escapeHtml(issue.message)}</li>`).join("")}</ul>` : `<p class="muted">No CrabDB coordination blockers reported.</p>`}
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
  persistWebviewState();
  resizeComposerInput(input);
  syncComposerAffordances();
  closeComposerControls();
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
  if (hint) {
    announceToast(hint, "status");
  }
}

function insertPromptPreset(presetId: string): void {
  const preset = COMPOSER_PROMPT_PRESETS.find((item) => item.id === presetId);
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  if (!preset || !input) {
    return;
  }
  insertComposerText(input, preset.text);
  announceToast(`${preset.label} prompt added.`, "status");
}

function insertComposerText(input: HTMLTextAreaElement, text: string): void {
  const start = input.selectionStart ?? input.value.length;
  const end = input.selectionEnd ?? input.value.length;
  const before = input.value.slice(0, start);
  const after = input.value.slice(end);
  const hasSelection = start !== end;
  const separatorBefore = before && !before.endsWith("\n") ? "\n\n" : "";
  const separatorAfter = after && !after.startsWith("\n") ? "\n\n" : "";
  input.value = hasSelection ? `${before}${text}${after}` : `${before}${separatorBefore}${text}${separatorAfter}${after}`;
  composerDraft = input.value;
  persistWebviewState();
  resizeComposerInput(input);
  syncComposerAffordances();
  input.focus();
  const cursor = before.length + (hasSelection ? text.length : separatorBefore.length + text.length);
  input.setSelectionRange(cursor, cursor);
}

function clearComposerDraft(): void {
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  if (!input || (!input.value && !composerDraft)) {
    return;
  }
  input.value = "";
  composerDraft = "";
  persistWebviewState();
  resizeComposerInput(input);
  syncComposerAffordances();
  input.focus();
  announceToast("Draft cleared.", "status");
}

function setComposerSendMode(mode: unknown): void {
  if (!isComposerSendMode(mode) || composerSendMode === mode) {
    return;
  }
  composerSendMode = mode;
  persistWebviewState();
  syncComposerSendModeControls();
  announceToast(mode === "fast" ? "Fast send enabled." : "Draft mode enabled.", "status");
}

function syncComposerSendModeControls(): void {
  document.querySelectorAll<HTMLButtonElement>(".composer-mode-button").forEach((button) => {
    const active = button.dataset.sendMode === composerSendMode;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", active ? "true" : "false");
  });
  const input = document.querySelector<HTMLTextAreaElement>(".composer-input");
  input?.setAttribute("aria-keyshortcuts", composerKeyShortcuts());
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

function announceToast(message: string, tone: "error" | "status"): void {
  announcement = message;
  const liveRegion = document.querySelector<HTMLElement>("[data-live-announcement]");
  if (liveRegion) {
    liveRegion.textContent = message;
  }
  const existing = document.querySelector(".toast");
  existing?.remove();
  const node = document.createElement("div");
  node.className = `toast ${tone}`;
  node.setAttribute("role", tone === "error" ? "alert" : "status");
  node.textContent = message;
  document.body.append(node);
  setTimeout(() => node.remove(), 6000);
}

function prepareJsonDrawer(): void {
  const active = document.activeElement;
  closeJsonDrawer({ restoreFocus: false });
  drawerRestoreFocus =
    active instanceof HTMLElement && active !== document.body && !active.closest(".json-drawer") ? active : undefined;
}

function configureJsonDrawer(drawer: HTMLElement, label: string): void {
  drawer.setAttribute("role", "dialog");
  drawer.setAttribute("aria-modal", "true");
  drawer.setAttribute("aria-label", label);
  drawer.setAttribute("tabindex", "-1");
}

function mountJsonDrawer(drawer: HTMLElement): void {
  setAppModalInert(true);
  document.body.append(drawer);
  drawer.querySelector<HTMLElement>("[data-action='closeDrawer']")?.focus();
  window.requestAnimationFrame(() => {
    void hydratePayloadDisclosures().then(() => hydrateInlineActions());
  });
}

function mountResultDrawer(props: ResultDrawerProps): void {
  setAppModalInert(true);
  resultDrawerModulePromise ??= import("./ResultDrawer.js")
    .then((module) => {
      resultDrawerModule = module;
      return module;
    })
    .catch((error) => {
      resultDrawerModulePromise = undefined;
      setAppModalInert(false);
      announceToast(`Unable to open drawer: ${error instanceof Error ? error.message : "unknown error"}`, "error");
      throw error;
    });
  void resultDrawerModulePromise.then((module) => {
    module.mountResultDrawer({
      props,
      onClose: () => closeJsonDrawer()
    });
    window.requestAnimationFrame(() => {
      void hydratePayloadDisclosures().then(() => hydrateInlineActions());
    });
  }).catch(() => undefined);
}

function setAppModalInert(inert: boolean): void {
  app?.toggleAttribute("inert", inert);
  if (inert) {
    app?.setAttribute("aria-hidden", "true");
  } else {
    app?.removeAttribute("aria-hidden");
  }
}

function activeJsonDrawer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(".json-drawer");
}

function handleJsonDrawerKeydown(event: KeyboardEvent): boolean {
  if (!activeJsonDrawer()) {
    return false;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    closeJsonDrawer();
    return true;
  }
  if (event.key === "Tab") {
    trapJsonDrawerFocus(event);
    return true;
  }
  return true;
}

function trapJsonDrawerFocus(event: KeyboardEvent): boolean {
  const drawer = activeJsonDrawer();
  if (!drawer) {
    return false;
  }
  const focusable = drawerFocusableElements(drawer);
  if (!focusable.length) {
    event.preventDefault();
    drawer.focus();
    return true;
  }
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (!first || !last) {
    event.preventDefault();
    drawer.focus();
    return true;
  }
  const active = document.activeElement;
  if (!(active instanceof HTMLElement) || !drawer.contains(active)) {
    event.preventDefault();
    first.focus();
    return true;
  }
  if (event.shiftKey && active === first) {
    event.preventDefault();
    last.focus();
    return true;
  }
  if (!event.shiftKey && active === last) {
    event.preventDefault();
    first.focus();
    return true;
  }
  return false;
}

function drawerFocusableElements(drawer: HTMLElement): HTMLElement[] {
  return Array.from(drawer.querySelectorAll<HTMLElement>(DRAWER_FOCUSABLE_SELECTOR)).filter(isVisibleFocusable);
}

function isVisibleFocusable(element: HTMLElement): boolean {
  if (element.hasAttribute("hidden") || element.getAttribute("aria-hidden") === "true") {
    return false;
  }
  return window.getComputedStyle(element).visibility !== "hidden" && element.getClientRects().length > 0;
}

function openDiffReviewDrawer(result: unknown): void {
  void getDiffReviewDrawerModule()
    .then((module) => {
      pendingDiffPreviews = [];
      const rendered = module.renderDiffReviewDrawer(result, {
        escapeHtml,
        escapeClass,
        shortLabel,
        inlineActions: ({ actions, ariaLabel, className }) =>
          inlineActions({
            ariaLabel,
            className,
            actions: actions.map(({ icon, ...action }) => ({
              ...action,
              iconHtml: iconSvg(icon as IconName)
            }))
          }),
        diffPreview,
        rawDetails
      });
      if (!rendered) {
        pendingDiffPreviews = [];
        openJsonDrawer("diff", result);
        return;
      }

      prepareJsonDrawer();
      const drawer = document.createElement("section");
      drawer.className = "json-drawer diff-review-drawer";
      configureJsonDrawer(drawer, "Review changes");
      drawer.innerHTML = rendered.html;
      mountJsonDrawer(drawer);
      selectDiffReviewFile(rendered.firstPath);
      void hydratePayloadDisclosures()
        .then(() => hydrateInlineActions())
        .then(() => drawer.querySelector<HTMLElement>("[data-action='closeDrawer']")?.focus());
      void hydrateDiffPreviews(++diffRenderEpoch).then(() => hydrateInlineActions());
    })
    .catch(() => {
      pendingDiffPreviews = [];
      openJsonDrawer("diff", result);
    });
}

function openJsonDrawer(type: string, result: unknown): void {
  prepareJsonDrawer();
  const drawer = document.createElement("section");
  const json = truncateText(redactedJson(result), MAX_RAW_JSON_CHARS);
  const title = drawerTitle(type);
  drawer.innerHTML = `
    ${codeBlock(json.text, { language: "json", title, copyLabel: "Copy JSON" })}
    ${json.truncated ? `<p class="muted">Result truncated at ${MAX_RAW_JSON_CHARS} chars.</p>` : ""}
  `;
  mountResultDrawer({
    title,
    description: "Redacted provider result payload.",
    badgeLabel: type,
    closeLabel: "Close drawer",
    bodyHtml: drawer.innerHTML
  });
}

function openCompareDrawer(result: unknown): void {
  prepareJsonDrawer();
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
  const comparePathWidget = comparePathAccordionWidget(shared, leftOnly, rightOnly);
  const widgets = comparePathWidget ? [comparePathWidget] : [];
  const bodyHtml = `
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
      ${comparePathWidget ? resultDrawerWidgetHost(comparePathWidget.id) : ""}
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
  mountResultDrawer({
    title: "Task compare",
    description: "Changed-path overlap and suggested next commands.",
    badgeLabel: `${shared.length} shared`,
    className: "compare-drawer",
    closeLabel: "Close drawer",
    bodyHtml,
    widgets
  });
}

function openConflictDrawer(result: unknown): void {
  prepareJsonDrawer();
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
  const widgets: ResultDrawerWidget[] = [];
  const bodyHtml = `
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
          ? `<div class="conflict-path-list">${paths.slice(0, 20).map((path, index) => conflictPathCard(path, index, widgets)).join("")}</div>
             ${paths.length > 20 ? `<p class="muted">Showing 20 of ${paths.length} paths.</p>` : ""}`
          : `<p class="muted">No path-level explanation was returned for this conflict set.</p>`
      }
    </section>
    ${recommendations.length ? conflictItemSection("Recommendations", recommendations, "resolution option") : ""}
    ${nextSteps.length ? conflictItemSection("Next steps", nextSteps, "next step") : ""}
    ${rawDetails(result)}
  `;
  mountResultDrawer({
    title: "Conflict details",
    description: "Conflict set summary, affected paths, and recommended recovery steps.",
    badgeLabel: status,
    className: "conflict-drawer",
    closeLabel: "Close conflict drawer",
    bodyHtml,
    widgets
  });
}

function resultDrawerWidgetHost(id: string): string {
  return `<div data-result-drawer-widget="${escapeHtml(id)}"></div>`;
}

function closeJsonDrawer(options: { restoreFocus?: boolean } = {}): void {
  resultDrawerModule?.closeResultDrawer();
  document.querySelector("[data-result-drawer-host]")?.remove();
  const drawer = document.querySelector(".json-drawer");
  drawer?.remove();
  diffEnhancerModule?.cleanupDetachedEnhancements?.();
  payloadDisclosureModulePromise?.then((module) => module.cleanupDetachedPayloadDisclosures()).catch(() => undefined);
  setAppModalInert(false);
  if (options.restoreFocus === false) {
    return;
  }
  const target = drawerRestoreFocus;
  drawerRestoreFocus = undefined;
  if (target?.isConnected) {
    target.focus({ preventScroll: true });
  }
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

function conflictPathCard(value: unknown, index: number, widgets: ResultDrawerWidget[]): string {
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
      ${lines.length ? conflictItemDetails("Lines", lines, "line", widgets, `path-${index + 1}-lines`) : ""}
      ${knownResolutions.length ? conflictItemDetails("Known resolutions", knownResolutions, "resolution", widgets, `path-${index + 1}-known-resolutions`) : ""}
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

function conflictItemDetails(
  title: string,
  items: unknown[],
  fallback: string,
  widgets: ResultDrawerWidget[],
  id: string
): string {
  const widgetId = `conflict-${id}`;
  widgets.push({
    type: "accordion",
    id: widgetId,
    className: "conflict-details",
    items: [
      {
        id: `${widgetId}-items`,
        title: `${title} (${items.length})`,
        triggerClassName: "conflict-details-summary",
        contentClassName: "conflict-details-panel",
        contentHtml: `
      <ul class="conflict-list">
        ${items.slice(0, 12).map((item) => `<li>${conflictItemText(item, fallback)}</li>`).join("")}
      </ul>
      ${items.length > 12 ? `<p class="muted">Showing 12 of ${items.length} items.</p>` : ""}
        `
      }
    ]
  });
  return resultDrawerWidgetHost(widgetId);
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
      <div class="compare-task-header">
        <span class="compare-task-label">${escapeHtml(label)}</span>
        <span class="compare-task-status status status-${escapeClass(status)}">${escapeHtml(status)}</span>
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

function comparePathAccordionWidget(
  shared: unknown[],
  leftOnly: unknown[],
  rightOnly: unknown[]
): ResultDrawerWidget | undefined {
  const items = [
    comparePathAccordionItem("compare-paths-shared", "Shared changed files", shared, "shared"),
    comparePathAccordionItem("compare-paths-left-only", "Left only", leftOnly, "single"),
    comparePathAccordionItem("compare-paths-right-only", "Right only", rightOnly, "single")
  ].filter((item): item is NonNullable<typeof item> => Boolean(item));
  if (!items.length) {
    return undefined;
  }
  return {
    type: "accordion",
    id: "compare-paths",
    className: "compare-paths",
    multiple: true,
    defaultOpenIds: shared.length ? ["compare-paths-shared"] : [],
    items
  };
}

function comparePathAccordionItem(
  id: string,
  title: string,
  paths: unknown[],
  mode: "shared" | "single"
): ResultDrawerWidget["items"][number] | undefined {
  if (!paths.length) {
    return undefined;
  }
  return {
    id,
    title: `${title} (${paths.length})`,
    className: "compare-paths-item",
    triggerClassName: "compare-paths-summary",
    contentClassName: "compare-paths-panel",
    contentHtml: `
      <ul>
        ${paths.slice(0, 30).map((path) => comparePathRow(path, mode)).join("")}
      </ul>
      ${paths.length > 30 ? `<p class="muted">Showing 30 of ${paths.length} paths.</p>` : ""}
    `
  };
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
  if (markdownModule) {
    return markdownModule.renderMarkdown(text, {
      maxChars: MAX_TEXT_CHARS,
      renderCodeBlock: (code, language) =>
        codeBlock(code, { language: language || "plaintext", title: "Code" })
    });
  }
  void loadMarkdownRenderer();
  const truncated = truncateText(text, MAX_TEXT_CHARS);
  return (
    `<p>${escapeHtml(truncated.text).replace(/\n/g, "<br>")}</p>` +
    (truncated.truncated ? `<p class="muted">Message preview truncated at ${MAX_TEXT_CHARS} chars.</p>` : "")
  );
}

function loadMarkdownRenderer(): Promise<typeof import("./markdownModel.js")> {
  if (!markdownModulePromise) {
    markdownModulePromise = import("./markdownModel.js").then((module) => {
      markdownModule = module;
      scheduleRender();
      return module;
    });
  }
  return markdownModulePromise;
}

function diffStats(oldText: string, newText: string): { oldLineCount: number; newLineCount: number; kind: string } {
  return {
    oldLineCount: lineCount(oldText),
    newLineCount: lineCount(newText),
    kind: !oldText && newText ? "created" : oldText && !newText ? "deleted" : "modified"
  };
}

function diffSummaryText(stats: { oldLineCount: number; newLineCount: number; kind: string }): string {
  return `${stats.kind} diff, ${stats.oldLineCount} before / ${stats.newLineCount} after`;
}

function diffPreview({
  path,
  oldText,
  newText,
  compact,
  patch,
  additions,
  deletions,
  nodeId,
  title
}: {
  path: string;
  oldText: string;
  newText: string;
  compact?: boolean | undefined;
  patch?: string | undefined;
  additions?: number | undefined;
  deletions?: number | undefined;
  nodeId?: string | undefined;
  title?: string | undefined;
}): string {
  const id = `diff-preview-${++diffPreviewCounter}`;
  const language = languageForResource(path, "text/plain");
  const stats = diffStats(oldText, newText);
  pendingDiffPreviews.push({ id, path, oldText, newText, compact, patch, additions, deletions, nodeId, title });
  return `
    <section class="diff-preview diff-preview-loading${compact ? " diff-preview-compact" : ""}" data-diff-preview-id="${escapeHtml(id)}" aria-busy="true" aria-label="${escapeHtml(title || "Diff preview")} for ${escapeHtml(path)}">
      ${compact ? "" : diffPreviewToolbar({ path, language, nodeId, additions: undefined, deletions: undefined, loading: true })}
      ${compact ? "" : `<div class="diff-preview-meta">
        <span>${stats.oldLineCount} before</span>
        <span>${stats.newLineCount} after</span>
        <span>preparing structured diff</span>
      </div>`}
      <div class="diff-loading" role="status">
        <span class="diff-loading-bar" aria-hidden="true"></span>
        <span>Preparing structured diff preview...</span>
      </div>
      <template class="diff-source"></template>
    </section>
  `;
}

function diffPreviewToolbar({
  path,
  language,
  nodeId,
  additions,
  deletions,
  loading
}: {
  path: string;
  language: string;
  nodeId?: string | undefined;
  additions?: number | undefined;
  deletions?: number | undefined;
  loading: boolean;
}): string {
  const actions = inlineActions({
    ariaLabel: `${shortLabel(path)} diff preview actions`,
    className: "diff-preview-actions",
    actions: [
      ...(nodeId
        ? [
            {
              action: "openNodeDiff",
              ariaLabel: "Open native diff",
              data: { "node-id": nodeId },
              iconHtml: iconSvg("diff"),
              iconOnly: true,
              label: "Open native diff",
              tone: "review" as const
            }
          ]
        : []),
      {
        action: "copyDiff",
        ariaLabel: "Copy unified diff",
        disabled: loading,
        iconHtml: iconSvg("copy"),
        iconOnly: true,
        label: "Copy unified diff",
        tone: "provider"
      },
      {
        action: "openDiffPreview",
        ariaLabel: "Open unified diff preview",
        data: { title: path, language: "diff" },
        disabled: loading,
        iconHtml: iconSvg("open"),
        iconOnly: true,
        label: "Open unified diff preview",
        tone: "provider"
      }
    ]
  });
  return `
    <div class="diff-preview-toolbar">
      <div class="diff-preview-title">
        <span class="code-title">${escapeHtml(shortLabel(path))}</span>
        <span class="code-language">${escapeHtml(language)}</span>
        ${
          loading
            ? `<span class="diff-stat diff-stat-loading">loading</span>`
            : `<span class="diff-stat additions">+${additions ?? 0}</span><span class="diff-stat deletions">-${deletions ?? 0}</span>`
        }
      </div>
      ${actions}
    </div>
  `;
}

function renderDiffRow(row: DiffRow): string {
  if (row.kind === "gap") {
    return `<div class="diff-row diff-row-gap" role="row"><span class="diff-gap-message" role="cell">${row.omitted || 0} unchanged line${row.omitted === 1 ? "" : "s"} hidden</span></div>`;
  }
  return `
    <div class="diff-row diff-row-${row.kind}" role="row">
      <span class="diff-line-number" role="cell">${row.oldLine ?? ""}</span>
      <code class="diff-code diff-code-old" role="cell">${renderDiffCell(row, "old")}</code>
      <span class="diff-line-number" role="cell">${row.newLine ?? ""}</span>
      <code class="diff-code diff-code-new" role="cell">${renderDiffCell(row, "new")}</code>
    </div>
  `;
}

function renderDiffCell(row: DiffRow, side: "old" | "new"): string {
  const text = side === "old" ? row.oldText : row.newText;
  if (text === undefined) {
    return "";
  }
  const segments = side === "old" ? row.oldSegments : row.newSegments;
  if (segments?.length) {
    return renderDiffSegments(segments);
  }
  return escapeHtml(text) || " ";
}

function renderDiffSegments(segments: DiffSegment[]): string {
  return segments
    .map((segment) => {
      const content = escapeHtml(segment.text) || " ";
      if (segment.tone === "added") {
        return `<ins>${content}</ins>`;
      }
      if (segment.tone === "removed") {
        return `<del>${content}</del>`;
      }
      return content;
    })
    .join("");
}

function rawDetails(value: unknown): string {
  const details = rawDetailsContent(value);
  return payloadDisclosure({
    className: "raw",
    label: "Details",
    bodyHtml: details
  });
}

function rawDetailsContent(value: unknown): string {
  const json = truncateText(redactedJson(value), MAX_RAW_JSON_CHARS);
  return `${codeBlock(json.text, { language: "json", title: "Redacted details", copyLabel: "Copy JSON" })}${json.truncated ? `<p class="muted">Details truncated at ${MAX_RAW_JSON_CHARS} chars.</p>` : ""}`;
}

function payloadDisclosure({
  bodyHtml,
  className,
  defaultOpen,
  label
}: {
  bodyHtml: string;
  className: string;
  defaultOpen?: boolean | undefined;
  label: string;
}): string {
  const id = `payload-${++payloadDisclosureCounter}`;
  payloadDisclosureProps.set(id, {
    id,
    bodyHtml,
    className,
    defaultOpen,
    label
  });
  return `<div data-payload-disclosure-root data-payload-disclosure-id="${escapeHtml(id)}"></div>`;
}

function inlineActions({
  actions,
  ariaLabel,
  className
}: Omit<InlineActionsProps, "id">): string {
  const id = `inline-actions-${++inlineActionsCounter}`;
  inlineActionsProps.set(id, {
    id,
    actions,
    ariaLabel,
    className
  });
  return `<div data-inline-actions-root data-inline-actions-id="${escapeHtml(id)}"></div>`;
}

function rawDetailsView(id: string, value: unknown): RawDetailsView {
  const json = truncateText(redactedJson(value), MAX_RAW_JSON_CHARS);
  return {
    id,
    label: "Details",
    contentHtml: codeBlock(json.text, { language: "json", title: "Redacted details", copyLabel: "Copy JSON" }),
    truncatedText: json.truncated ? `Details truncated at ${MAX_RAW_JSON_CHARS} chars.` : undefined
  };
}

function codeBlock(
  text: string,
  options: {
    language?: string | undefined;
    title?: string | undefined;
    meta?: string | undefined;
    copyLabel?: string | undefined;
    openPath?: string | undefined;
  } = {}
): string {
  const language = cleanLanguage(options.language || "plaintext");
  const languageLabel = language === "plaintext" ? "text" : language;
  const title = shortLabel(options.title || "Preview");
  const copyLabel = options.copyLabel || "Copy";
  const openPath = String(options.openPath || "").trim();
  const codeActions = inlineActions({
    ariaLabel: `${title} preview actions`,
    className: "code-actions",
    actions: [
      {
        action: "copyCode",
        ariaLabel: copyLabel,
        iconHtml: iconSvg("copy"),
        iconOnly: true,
        label: copyLabel,
        tone: "provider"
      },
      openPath
        ? {
            action: "openLocation",
            ariaLabel: "Open path",
            data: { path: openPath },
            iconHtml: iconSvg("open"),
            iconOnly: true,
            label: "Open path",
            tone: "provider"
          }
        : {
            action: "openTextPreview",
            ariaLabel: "Open preview in editor",
            data: { language, title },
            iconHtml: iconSvg("open"),
            iconOnly: true,
            label: "Open preview in editor",
            tone: "provider"
          }
    ]
  });
  return `
    <div class="code-frame">
      <div class="code-tools">
        <span class="code-title"><span>${escapeHtml(title)}</span>${options.meta ? `<small>${escapeHtml(options.meta)}</small>` : ""}</span>
        <span class="code-language">${escapeHtml(languageLabel)}</span>
        ${codeActions}
      </div>
      <pre class="code" data-highlight-language="${escapeHtml(language)}" aria-label="${escapeHtml(`${title} source preview`)}" tabindex="0">${escapeHtml(text)}</pre>
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
  if (lowerMime.includes("diff") || lowerMime.includes("patch")) {
    return "diff";
  }
  const extension = uri.split(/[?#]/, 1)[0]?.split(/[\\/]/).pop()?.split(".").pop()?.toLowerCase();
  switch (extension) {
    case "bash":
    case "sh":
    case "zsh":
      return "shellscript";
    case "md":
    case "mdx":
      return "markdown";
    case "js":
    case "mjs":
    case "cjs":
      return "javascript";
    case "jsx":
      return "jsx";
    case "ts":
    case "mts":
    case "cts":
      return "typescript";
    case "tsx":
      return "tsx";
    case "json":
    case "jsonc":
    case "lock":
      return "json";
    case "html":
    case "htm":
      return "html";
    case "css":
    case "go":
    case "xml":
      return extension;
    case "rs":
      return "rust";
    case "py":
      return "python";
    case "yml":
    case "yaml":
      return "yaml";
    case "txt":
    case "text":
      return "text";
    default:
      return "plaintext";
  }
}

async function highlightCodeBlocks(): Promise<void> {
  if (!document.querySelector("pre.code[data-highlight-language]:not([data-highlight-state])")) {
    return;
  }
  try {
    const highlighter = await getHighlightModule();
    await highlighter.highlightCodeBlocks();
  } catch {
    document.querySelectorAll<HTMLPreElement>("pre.code[data-highlight-language]:not([data-highlight-state])").forEach((block) => {
      block.dataset.highlightState = "failed";
    });
  }
}

function getHighlightModule(): Promise<typeof import("./highlight.js")> {
  if (!highlightModulePromise) {
    highlightModulePromise = import("./highlight.js").catch((error) => {
      highlightModulePromise = undefined;
      throw error;
    });
  }
  return highlightModulePromise;
}

function getDiffModelModule(): Promise<typeof import("./diffModel.js")> {
  if (!diffModelModulePromise) {
    diffModelModulePromise = import("./diffModel.js").catch((error) => {
      diffModelModulePromise = undefined;
      throw error;
    });
  }
  return diffModelModulePromise;
}

function getDiffEnhancerModule(): Promise<typeof import("./diffEnhancer.js")> {
  if (!diffEnhancerModulePromise) {
    diffEnhancerModulePromise = import("./diffEnhancer.js")
      .then((module) => {
        diffEnhancerModule = module;
        return module;
      })
      .catch((error) => {
        diffEnhancerModulePromise = undefined;
        throw error;
      });
  }
  return diffEnhancerModulePromise;
}

function getDiffReviewDrawerModule(): Promise<typeof import("./diffReviewDrawer.js")> {
  if (!diffReviewDrawerModulePromise) {
    diffReviewDrawerModulePromise = import("./diffReviewDrawer.js").catch((error) => {
      diffReviewDrawerModulePromise = undefined;
      throw error;
    });
  }
  return diffReviewDrawerModulePromise;
}

function cleanupDetachedDiffEnhancements(): void {
  diffEnhancerModule?.cleanupDetachedEnhancements?.();
}

function renderPatchDiffPreview(
  preview: PendingDiffPreview,
  language: string,
  stats: { additions: number; deletions: number }
): string {
  const patch = preview.patch || "";
  const truncated = truncateText(patch, MAX_RAW_JSON_CHARS);
  const fallbackHtml = preview.compact
    ? `
      <div class="diff-fallback diff-fallback-compact" aria-label="Raw patch fallback">
        <pre class="code diff-compact-code" data-highlight-language="diff" tabindex="0">${escapeHtml(truncated.text)}</pre>
        ${truncated.truncated ? `<p class="muted">Patch truncated at ${MAX_RAW_JSON_CHARS} chars.</p>` : ""}
      </div>
    `
    : `
      <div class="diff-fallback" aria-label="Raw patch fallback">
        ${codeBlock(truncated.text, { language: "diff", title: "Patch", copyLabel: "Copy patch" })}
        ${truncated.truncated ? `<p class="muted">Patch truncated at ${MAX_RAW_JSON_CHARS} chars.</p>` : ""}
      </div>
    `;
  return `
    ${preview.compact ? "" : diffPreviewToolbar({
      path: preview.path,
      language,
      nodeId: preview.nodeId,
      additions: stats.additions,
      deletions: stats.deletions,
      loading: false
    })}
    ${preview.compact ? "" : `<div class="diff-preview-meta">
      <span>patch</span>
      <span>+${stats.additions}</span>
      <span>-${stats.deletions}</span>
    </div>`}
    <div class="diffs-mount" data-diffs-mode="patch" data-diffs-compact="${preview.compact ? "true" : "false"}" data-diffs-path="${escapeHtml(preview.path)}" data-diffs-language="${escapeHtml(language)}" role="region" aria-label="${escapeHtml(preview.title || "Diff")}"></div>
    ${fallbackHtml}
    <template class="diff-patch-source">${escapeHtml(patch)}</template>
    <template class="diff-source">${escapeHtml(patch)}</template>
  `;
}

async function hydrateDiffPreviews(epoch: number): Promise<void> {
  const previews = pendingDiffPreviews.slice();
  if (!previews.length) {
    return;
  }

  try {
    const diffModel = await getDiffModelModule();
    if (epoch !== diffRenderEpoch) {
      return;
    }

    let enhancedPreviewCount = 0;
    for (const preview of previews) {
      const previewElement = document.querySelector<HTMLElement>(`.diff-preview[data-diff-preview-id="${preview.id}"]`);
      if (!previewElement) {
        continue;
      }
      if (preview.patch && !preview.oldText && !preview.newText) {
        const language = languageForResource(preview.path, "text/x-diff");
        const stats = { additions: preview.additions ?? 0, deletions: preview.deletions ?? 0 };
        previewElement.classList.remove("diff-preview-loading");
        previewElement.toggleAttribute("aria-busy", false);
        previewElement.innerHTML = renderPatchDiffPreview(preview, language, stats);
        enhancedPreviewCount += 1;
        continue;
      }
      const model = diffModel.buildDiffModel(preview.path, preview.oldText, preview.newText);
      const language = languageForResource(preview.path, "text/plain");
      previewElement.classList.remove("diff-preview-loading");
      previewElement.toggleAttribute("aria-busy", false);
      previewElement.innerHTML = renderHydratedDiffPreview(preview, model, language);
      if (!model.tooLarge && preview.oldText.length + preview.newText.length <= MAX_TEXT_CHARS * 4) {
        enhancedPreviewCount += 1;
      }
    }

    if (enhancedPreviewCount > 0) {
      const enhancer = await getDiffEnhancerModule();
      if (epoch === diffRenderEpoch) {
        await enhancer.enhanceDiffPreviews();
      }
    }
  } catch {
    if (epoch !== diffRenderEpoch) {
      return;
    }
    for (const preview of previews) {
      const previewElement = document.querySelector<HTMLElement>(`.diff-preview[data-diff-preview-id="${preview.id}"]`);
      if (previewElement) {
        previewElement.classList.remove("diff-preview-loading");
        previewElement.classList.add("diff-preview-error");
        previewElement.toggleAttribute("aria-busy", false);
        previewElement.innerHTML = renderDiffPreviewError(preview);
      }
    }
  }
}

function renderHydratedDiffPreview(preview: PendingDiffPreview, model: DiffModel, language: string): string {
  return `
    ${preview.compact ? "" : diffPreviewToolbar({
      path: preview.path,
      language,
      nodeId: preview.nodeId,
      additions: model.additions,
      deletions: model.deletions,
      loading: false
    })}
    ${preview.compact ? "" : `<div class="diff-preview-meta">
      <span>${escapeHtml(model.kind)}</span>
      <span>${model.oldLineCount} before</span>
      <span>${model.newLineCount} after</span>
      ${model.omittedRows > 0 ? `<span>${model.omittedRows} unchanged hidden</span>` : ""}
      ${model.tooLarge ? `<span>large preview</span>` : ""}
    </div>`}
    ${
      model.tooLarge
        ? ""
        : `<div class="diffs-mount" data-diffs-compact="${preview.compact ? "true" : "false"}" data-diffs-path="${escapeHtml(preview.path)}" data-diffs-language="${escapeHtml(language)}" role="region" aria-label="${escapeHtml(preview.title || "Diff")}"></div>`
    }
    <div class="diff-fallback" aria-label="Structured diff fallback">
      ${renderDiffGrid(model)}
    </div>
    <template class="diff-old-source">${escapeHtml(preview.oldText)}</template>
    <template class="diff-new-source">${escapeHtml(preview.newText)}</template>
    <template class="diff-source">${escapeHtml(model.rawDiff)}</template>
  `;
}

function renderDiffPreviewError(preview: PendingDiffPreview): string {
  const language = languageForResource(preview.path, "text/plain");
  return `
    ${preview.compact ? "" : diffPreviewToolbar({ path: preview.path, language, nodeId: preview.nodeId, loading: false })}
    <div class="diff-loading" role="status">
      <span>Diff preview failed. Raw before/after content is still available.</span>
    </div>
    <template class="diff-source"></template>
  `;
}

function renderDiffGrid(model: DiffModel): string {
  return `
    <div class="diff-grid" role="table" aria-label="Diff rows">
      <div class="diff-row diff-row-header" role="row">
        <span role="columnheader">Before</span>
        <span role="columnheader">Content</span>
        <span role="columnheader">After</span>
        <span role="columnheader">Content</span>
      </div>
      ${model.rows.map(renderDiffRow).join("")}
    </div>
  `;
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
  return timelineGroups(chatNodes(state.nodes)).filter((group) => group.turnId).length;
}

function transcriptAnchors(): Array<{ id: string; label: string }> {
  return timelineGroups(chatNodes(state.nodes))
    .filter((group) => group.turnId)
    .slice(0, 12)
    .map((group) => ({
      id: timelineGroupDomId(group),
      label: group.label
    }));
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
      return `<li>${escapeHtml(name)} <span class="test-run-status status-${escapeClass(status)}">${escapeHtml(status)}</span></li>`;
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

function formatTimestamp(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function eventCostLabel(cost: Record<string, unknown>): string {
  const total = numberChoice(cost, ["total", "totalCost", "cost"]);
  const input = numberChoice(cost, ["input", "inputCost"]);
  const output = numberChoice(cost, ["output", "outputCost"]);
  if (typeof total === "number") {
    return `$${total.toFixed(total < 1 ? 4 : 2)}`;
  }
  if (typeof input === "number" || typeof output === "number") {
    return [`in ${input?.toFixed(4) || "0"}`, `out ${output?.toFixed(4) || "0"}`].join(" / ");
  }
  return "";
}

function contentLabel(record: Record<string, unknown>, fallback: string): string {
  const annotations = asRecord(record.annotations);
  return String(record.title || record.name || annotations.title || annotations.label || fallback);
}

function resourceBlockLabel(block: unknown): string {
  const record = asRecord(block);
  if (record.type === "resource") {
    const resource = asRecord(record.resource);
    return shortLabel(String(resource.uri || resource.mimeType || "Embedded resource"));
  }
  if (record.type === "resource_link") {
    return shortLabel(String(record.title || record.name || record.uri || "Resource link"));
  }
  return shortLabel(String(record.type || "Resource"));
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
  | "lane"
  | "message"
  | "open"
  | "refresh"
  | "rewind"
  | "review"
  | "search"
  | "selection"
  | "send"
  | "session"
  | "settings"
  | "stop"
  | "terminal"
  | "tool"
  | "tree"
  | "turn";

type LucideIconNode = Array<[tag: string, attrs: Record<string, string | number | undefined>]>;

const LUCIDE_ICONS: Record<IconName, LucideIconNode> = {
  changed: FileDiff,
  check: SquareCheckBig,
  close: X,
  copy: Copy,
  diagnostics: CircleAlert,
  diff: DiffIcon,
  file: FileText,
  history: History,
  lane: FolderGit2,
  message: MessageSquare,
  open: ExternalLink,
  refresh: RefreshCw,
  rewind: RotateCcw,
  review: PanelRightOpen,
  search: Search,
  selection: SquareDashedMousePointer,
  send: Send,
  session: MessagesSquare,
  settings: Settings,
  stop: SquareStop,
  terminal: Terminal,
  tool: Wrench,
  tree: ListTree,
  turn: MessagesSquare
};

function iconSvg(icon: IconName): string {
  const node = LUCIDE_ICONS[icon] || LUCIDE_ICONS.tool;
  return `<svg class="icon lucide lucide-${escapeClass(icon)}" viewBox="0 0 24 24" aria-hidden="true" focusable="false" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${node
    .map(([tag, attrs]) => `<${tag}${lucideAttrs(attrs)}/>`)
    .join("")}</svg>`;
}

function lucideAttrs(attrs: Record<string, string | number | undefined>): string {
  const rendered = Object.entries(attrs)
    .filter(([, value]) => value !== undefined)
    .map(([key, value]) => `${key}="${escapeHtml(String(value))}"`);
  return rendered.length ? ` ${rendered.join(" ")}` : "";
}

function persistWebviewState(): void {
  const snapshot = { composerDraft, composerSendMode, reviewVisible, timelineFilter, timelineQuery };
  const json = JSON.stringify(snapshot);
  if (json === lastPersistedWebviewStateJson) {
    return;
  }
  lastPersistedWebviewStateJson = json;
  vscode.setState(snapshot);
}

function isComposerSendMode(value: unknown): value is ComposerSendMode {
  return typeof value === "string" && COMPOSER_SEND_MODES.has(value as ComposerSendMode);
}

function unsupportedContent(label: string, detail: unknown): string {
  return payloadDisclosure({
    className: "unsupported",
    label,
    bodyHtml: rawDetailsContent(detail)
  });
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
  const filtered = filterTimelineNodes(chatNodes(state.nodes), timelineFilter, timelineQuery);
  return sortTimelineNodes(filtered);
}

function chatNodes(nodes: RenderNode[]): RenderNode[] {
  return timelineDisplayNodes(nodes);
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

function sessionControlSelectors(disabled = ""): string {
  const controls = [...configSelectors(disabled), modeSelector(disabled), commandSelector(disabled)].filter(Boolean);
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

function currentProviderProfile(): NonNullable<WebviewState["providers"]>[number] | undefined {
  const providers = state.providers || [];
  return providers.find((provider) => provider.id === state.providerId) || providers.find((provider) => provider.label === state.provider);
}

function configSelectors(disabled = ""): string[] {
  return currentConfigOptions()
    .filter((option) => option.type === "select" && Array.isArray(option.options))
    .slice(0, 4)
    .map((option) => {
      const options = (option.options as unknown[]).map(asRecord);
      const currentValue = String(option.currentValue || "");
      return `
        <label class="select-control" title="${escapeHtml(String(option.description || option.name || "Configuration"))}">
          <span>${escapeHtml(String(option.name || option.id))}</span>
          <select data-action="setConfigOption" data-config-id="${escapeHtml(String(option.id))}" aria-label="${escapeHtml(String(option.name || option.id))}" ${disabled}>
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

function modeSelector(disabled = ""): string {
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
      <select data-action="setMode" aria-label="Mode" ${disabled}>
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

function commandSelector(disabled = ""): string {
  const commands = currentCommands();
  if (!commands.length) {
    return "";
  }
  return `
    <label class="select-control command-control">
      <span>Command</span>
      <select data-action="insertCommand" aria-label="Command" ${disabled}>
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

render();
vscode.postMessage({ type: "ready" });

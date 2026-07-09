export type ToolbarTone = "ok" | "warning" | "blocked" | "active" | "muted";

export interface ToolbarAction {
  action: "cancel" | "dryRunApply" | "focusComposer" | "focusReview" | "focusTranscript" | "refresh" | "startFollowUp";
  label: string;
  detail: string;
  tone: "primary" | "default" | "danger";
  disabled?: boolean | undefined;
}

export interface ToolbarChip {
  id: string;
  label: string;
  value: string;
  displayValue: string;
  tone: ToolbarTone;
  accessibilityLabel: string;
}

export interface ToolbarCapability {
  id: string;
  label: string;
  group: "workflow" | "input";
  enabled: boolean;
  detail: string;
}

export interface ToolbarRunState {
  label: string;
  detail: string;
  tone: ToolbarTone;
}

export interface ToolbarModelInput {
  taskStatus?: string | undefined;
  lane?: string | undefined;
  changedPaths: number;
  providerLabel?: string | undefined;
  providerCrabdbBacked?: boolean | undefined;
  sending?: boolean | undefined;
  permissionPending?: boolean | undefined;
  providerFailure?: boolean | undefined;
  supportsFromRef?: boolean | undefined;
  reviewVisible?: boolean | undefined;
  sessionLabel?: string | undefined;
  sessionTone?: string | undefined;
  acpSessionId?: string | undefined;
  nextAction?: string | undefined;
  modeLabel?: string | undefined;
  configCount: number;
  commandCount: number;
  coordinationLabels?: string[] | undefined;
  coordinationSeverity?: string | undefined;
  capabilities?: {
    image?: boolean | undefined;
    audio?: boolean | undefined;
    embeddedContext?: boolean | undefined;
  } | undefined;
}

export interface ToolbarModel {
  runState: ToolbarRunState;
  primaryAction: ToolbarAction;
  statusChips: ToolbarChip[];
  capabilities: ToolbarCapability[];
  capabilitySummary: string;
}

export function buildToolbarModel(input: ToolbarModelInput): ToolbarModel {
  const status = normalizeStatus(input.taskStatus);
  const changedPaths = count(input.changedPaths);
  const coordinationSeverity = normalizeStatus(input.coordinationSeverity);
  const runState = toolbarRunState(input, status, changedPaths, coordinationSeverity);
  const capabilities = toolbarCapabilities(input);
  const enabledCapabilities = capabilities.filter((capability) => capability.enabled).length;
  return {
    runState,
    primaryAction: toolbarPrimaryAction(input, status, changedPaths, coordinationSeverity),
    statusChips: toolbarStatusChips(input, changedPaths),
    capabilities,
    capabilitySummary: `${enabledCapabilities}/${capabilities.length} ready`
  };
}

function toolbarRunState(
  input: ToolbarModelInput,
  status: string,
  changedPaths: number,
  coordinationSeverity: string
): ToolbarRunState {
  if (input.permissionPending) {
    return {
      label: "Permission required",
      detail: "Resolve the pending tool request.",
      tone: "blocked"
    };
  }
  if (input.sending) {
    return {
      label: "Agent running",
      detail: "Transcript and gates update live.",
      tone: "active"
    };
  }
  if (input.providerFailure) {
    return {
      label: "Interrupted",
      detail: "Continue from the latest checkpoint.",
      tone: "blocked"
    };
  }
  if (["blocked", "conflicted", "failed"].includes(status) || coordinationSeverity === "blocked") {
    return {
      label: "Review required",
      detail: "Resolve blockers before applying.",
      tone: "blocked"
    };
  }
  if (coordinationSeverity === "warning") {
    return {
      label: "Needs review",
      detail: "Trail has warnings for this lane.",
      tone: "warning"
    };
  }
  if (status === "applied") {
    return {
      label: "Applied",
      detail: "Task record remains available.",
      tone: "ok"
    };
  }
  if (changedPaths > 0 || status === "ready") {
    return {
      label: "Ready for dry-run",
      detail: "Review changes before applying.",
      tone: "ok"
    };
  }
  return {
    label: "Ready",
    detail: "Ask the agent or attach context.",
    tone: "muted"
  };
}

function toolbarPrimaryAction(
  input: ToolbarModelInput,
  status: string,
  changedPaths: number,
  coordinationSeverity: string
): ToolbarAction {
  if (input.sending) {
    return {
      action: "cancel",
      label: "Cancel",
      detail: "Stop the current provider turn.",
      tone: "danger"
    };
  }
  if (input.permissionPending) {
    return {
      action: "focusTranscript",
      label: "Review approval",
      detail: "Jump to the pending permission card.",
      tone: "primary"
    };
  }
  if (input.providerFailure) {
    return {
      action: "startFollowUp",
      label: "Start follow-up",
      detail: "Continue from latest checkpoint.",
      tone: "primary"
    };
  }
  if (["blocked", "conflicted", "failed"].includes(status) || coordinationSeverity === "blocked" || coordinationSeverity === "warning") {
    return {
      action: "focusReview",
      label: input.reviewVisible ? "Inspect review" : "Open review",
      detail: "Check readiness, conflicts, approvals, and gates.",
      tone: "primary"
    };
  }
  if (changedPaths > 0) {
    return {
      action: "dryRunApply",
      label: "Dry-run apply",
      detail: "Preview changes before applying.",
      tone: "primary"
    };
  }
  return {
    action: "focusComposer",
    label: "Ask agent",
    detail: "Focus the message composer.",
    tone: "primary"
  };
}

function toolbarStatusChips(input: ToolbarModelInput, changedPaths: number): ToolbarChip[] {
  const providerLabel = input.providerLabel || "provider";
  const providerTone: ToolbarTone =
    input.providerCrabdbBacked === false ? "warning" : input.providerCrabdbBacked === true ? "ok" : "muted";
  const chips: ToolbarChip[] = [
    toolbarChip("provider", "Provider", `${providerLabel}${input.providerCrabdbBacked === false ? " (raw)" : ""}`, providerTone),
    toolbarChip(
      "session",
      "Session",
      input.sessionLabel || (input.acpSessionId ? "Active session" : "New session"),
      normalizeTone(input.sessionTone) || (input.acpSessionId ? "active" : "muted")
    ),
    toolbarChip("lane", "Lane", input.lane || "pending", "active"),
    toolbarChip("changes", "Changes", `${formatCount(changedPaths)} path${changedPaths === 1 ? "" : "s"}`, changedPaths > 0 ? "ok" : "muted")
  ];
  const nextAction = cleanText(input.nextAction);
  if (nextAction) {
    chips.push(toolbarChip("next", "Next", nextAction, "active"));
  }
  if (input.modeLabel) {
    chips.push(toolbarChip("mode", "Mode", input.modeLabel, "active"));
  }
  if (input.configCount > 0) {
    chips.push(toolbarChip("config", "Config", `${formatCount(input.configCount)} option${input.configCount === 1 ? "" : "s"}`, "active"));
  }
  for (const label of (input.coordinationLabels || []).slice(0, 3)) {
    chips.push(toolbarChip(`coordination-${chips.length}`, "Gate", label, normalizeTone(input.coordinationSeverity) || "muted"));
  }
  return chips;
}

function toolbarCapabilities(input: ToolbarModelInput): ToolbarCapability[] {
  const prompt = input.capabilities;
  const durable = input.providerCrabdbBacked === true;
  const raw = input.providerCrabdbBacked === false;
  const checkpoints = durable && input.supportsFromRef === true;
  return [
    {
      id: "durable-state",
      label: "Durable state",
      group: "workflow",
      enabled: durable,
      detail: durable
        ? "Trail persists transcript, checkpoints, review, and queue state."
        : raw
          ? "Raw provider route lacks durable Trail state."
          : "Provider route has not reported durability."
    },
    {
      id: "checkpoint-start",
      label: "Checkpoint start",
      group: "workflow",
      enabled: checkpoints,
      detail: checkpoints
        ? "Follow-ups can start from a Trail checkpoint."
        : durable
          ? "Checkpoint-start support is not advertised."
          : "Use a Trail-backed provider for checkpoint recovery."
    },
    {
      id: "review-gates",
      label: "Review gates",
      group: "workflow",
      enabled: durable,
      detail: durable
        ? "Review, queue, approvals, tests, and evals use Trail evidence."
        : "Durable gate state needs a Trail-backed route."
    },
    {
      id: "commands",
      label: "Commands",
      group: "workflow",
      enabled: input.commandCount > 0,
      detail: input.commandCount > 0 ? `${formatCount(input.commandCount)} slash command${input.commandCount === 1 ? "" : "s"} available.` : "No provider commands yet."
    },
    { id: "text", label: "Text", group: "input", enabled: true, detail: "Prompt text is available." },
    { id: "links", label: "Links", group: "input", enabled: true, detail: "Resource links are available." },
    {
      id: "inline",
      label: "Inline context",
      group: "input",
      enabled: prompt?.embeddedContext === true,
      detail: prompt?.embeddedContext === true ? "Files and selections can be embedded." : "Falls back to text or links."
    },
    {
      id: "image",
      label: "Images",
      group: "input",
      enabled: prompt?.image === true,
      detail: prompt?.image === true ? "Image attachments are enabled." : "Images are hidden for this provider."
    },
    {
      id: "audio",
      label: "Audio",
      group: "input",
      enabled: prompt?.audio === true,
      detail: prompt?.audio === true ? "Audio attachments are enabled." : "Audio is hidden for this provider."
    }
  ];
}

function normalizeTone(value: string | undefined): ToolbarTone | undefined {
  const normalized = normalizeStatus(value);
  switch (normalized) {
    case "ok":
    case "ready":
    case "applied":
    case "success":
      return "ok";
    case "warning":
    case "dirty":
      return "warning";
    case "blocked":
    case "conflicted":
    case "failed":
      return "blocked";
    case "active":
    case "running":
      return "active";
    case "muted":
    case "new":
      return "muted";
    default:
      return undefined;
  }
}

function toolbarChip(id: string, label: string, value: string, tone: ToolbarTone): ToolbarChip {
  const safeLabel = cleanText(label) || id;
  const safeValue = cleanText(value) || "none";
  return {
    id,
    label: safeLabel,
    value: safeValue,
    displayValue: shortenMiddle(safeValue, chipDisplayLimit(id)),
    tone,
    accessibilityLabel: `${safeLabel}: ${safeValue}`
  };
}

function chipDisplayLimit(id: string): number {
  switch (id) {
    case "next":
      return 56;
    case "provider":
      return 42;
    case "lane":
      return 40;
    default:
      return 34;
  }
}

function shortenMiddle(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value;
  }
  const head = Math.max(8, Math.floor((maxLength - 3) * 0.62));
  const tail = Math.max(6, maxLength - 3 - head);
  return `${value.slice(0, head)}...${value.slice(-tail)}`;
}

function normalizeStatus(value: string | undefined): string {
  return String(value || "new")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function cleanText(value: string | undefined): string {
  return String(value || "").replace(/\s+/g, " ").trim();
}

function count(value: number): number {
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

function formatCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(count(value));
}

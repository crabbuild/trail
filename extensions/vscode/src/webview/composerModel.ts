export const MAX_COMPOSER_DRAFT_CHARS = 120_000;

export interface ComposerSendState {
  hasDraft: boolean;
  attachmentCount: number;
  draftChars?: number | undefined;
  maxChars?: number | undefined;
  sending?: boolean | undefined;
  permissionPending?: boolean | undefined;
}

export interface ComposerDraftState {
  chars: number;
  lines: number;
  remaining: number;
  maxChars: number;
  meterPercent: number;
  meterValue: number;
  tone: "empty" | "ready" | "warning" | "limit";
  label: string;
  detail: string;
}

export type ComposerRailTone = "ready" | "active" | "warning" | "blocked" | "muted";

export interface ComposerRailInput {
  statusTone: "ready" | "context" | "running" | "waiting" | "warning";
  statusLabel: string;
  attachmentModes: string[];
  sendMode: "fast" | "draft";
  providerCrabdbBacked?: boolean | undefined;
}

export interface ComposerRailItem {
  id: "state" | "context" | "send" | "route";
  label: string;
  value: string;
  tone: ComposerRailTone;
}

export function composerSendBlockedReason(state: ComposerSendState): string | undefined {
  if (state.permissionPending) {
    return "Resolve the permission request before sending.";
  }
  if (state.sending) {
    return "The current prompt is still running.";
  }
  const maxChars = safeMaxChars(state.maxChars);
  if (
    state.hasDraft &&
    typeof state.draftChars === "number" &&
    Number.isFinite(state.draftChars) &&
    state.draftChars >= maxChars
  ) {
    return "Shorten the prompt or move bulky context into attachments before sending.";
  }
  if (!state.hasDraft && state.attachmentCount === 0) {
    return "Write a message or attach context before sending.";
  }
  return undefined;
}

export function composerMetrics(text: string, attachmentCount: number, maxChars = MAX_COMPOSER_DRAFT_CHARS): string {
  const { chars, lines, remaining } = composerDraftState(text, attachmentCount, maxChars);
  return [
    `${attachmentCount} attachment${attachmentCount === 1 ? "" : "s"}`,
    `${chars.toLocaleString()} char${chars === 1 ? "" : "s"}`,
    `${lines.toLocaleString()} line${lines === 1 ? "" : "s"}`,
    `${remaining.toLocaleString()} left`
  ].join(" - ");
}

export function composerDraftState(text: string, attachmentCount: number, maxChars = MAX_COMPOSER_DRAFT_CHARS): ComposerDraftState {
  const chars = Array.from(text).length;
  const lines = text ? text.split(/\r\n|\r|\n/).length : 0;
  const safeMax = safeMaxChars(maxChars);
  const remaining = Math.max(0, safeMax - chars);
  const meterValue = Math.min(chars, safeMax);
  const meterPercent = Math.round((meterValue / safeMax) * 100);
  const warningThreshold = Math.max(500, Math.floor(safeMax * 0.1));
  if (chars >= safeMax) {
    return {
      chars,
      lines,
      remaining,
      maxChars: safeMax,
      meterPercent,
      meterValue,
      tone: "limit",
      label: "Limit reached",
      detail: "Shorten the prompt or move bulky context into attachments."
    };
  }
  if (chars > 0 && remaining <= warningThreshold) {
    return {
      chars,
      lines,
      remaining,
      maxChars: safeMax,
      meterPercent,
      meterValue,
      tone: "warning",
      label: `${remaining.toLocaleString()} left`,
      detail: "This prompt is close to the composer limit."
    };
  }
  if (chars > 0) {
    return {
      chars,
      lines,
      remaining,
      maxChars: safeMax,
      meterPercent,
      meterValue,
      tone: "ready",
      label: `${chars.toLocaleString()} char${chars === 1 ? "" : "s"}`,
      detail: `${lines.toLocaleString()} line${lines === 1 ? "" : "s"} ready to send.`
    };
  }
  return {
    chars,
    lines,
    remaining,
    maxChars: safeMax,
    meterPercent,
    meterValue,
    tone: "empty",
    label: attachmentCount > 0 ? "Context-only prompt" : "Empty prompt",
    detail: attachmentCount > 0 ? "Attached context can be sent without prompt text." : "Write a message or attach context."
  };
}

export function composerRailItems(input: ComposerRailInput): ComposerRailItem[] {
  const attachmentSummary = attachmentModeSummary(input.attachmentModes);
  const routeValue =
    input.providerCrabdbBacked === true ? "CrabDB route" : input.providerCrabdbBacked === false ? "Raw provider" : "Provider route";
  return [
    {
      id: "state",
      label: "State",
      value: input.statusLabel || "Ready",
      tone: railToneFromStatus(input.statusTone)
    },
    {
      id: "context",
      label: "Context",
      value: attachmentSummary,
      tone: input.attachmentModes.length ? "ready" : "muted"
    },
    {
      id: "send",
      label: "Send",
      value: input.sendMode === "draft" ? "Enter newline" : "Enter sends",
      tone: "active"
    },
    {
      id: "route",
      label: "Route",
      value: routeValue,
      tone: input.providerCrabdbBacked === true ? "ready" : input.providerCrabdbBacked === false ? "warning" : "muted"
    }
  ];
}

function railToneFromStatus(statusTone: ComposerRailInput["statusTone"]): ComposerRailTone {
  switch (statusTone) {
    case "context":
    case "ready":
      return "ready";
    case "running":
      return "active";
    case "warning":
      return "warning";
    case "waiting":
      return "blocked";
    default:
      return "muted";
  }
}

function safeMaxChars(maxChars: number | undefined): number {
  return Number.isFinite(maxChars) && Number(maxChars) > 0 ? Math.floor(Number(maxChars)) : MAX_COMPOSER_DRAFT_CHARS;
}

export function attachmentModeSummary(modes: string[]): string {
  if (!modes.length) {
    return "No context";
  }
  const counts = new Map<string, number>();
  for (const mode of modes) {
    counts.set(mode, (counts.get(mode) || 0) + 1);
  }
  return Array.from(counts.entries())
    .map(([mode, count]) => `${count} ${mode}`)
    .join(", ");
}

export type EventTone = "info" | "success" | "warning" | "risk";
export type EventActionTone = "primary" | "default" | "danger";
export type EventKind =
  | "checkpoint"
  | "commands"
  | "completion"
  | "config"
  | "mode"
  | "resource"
  | "session"
  | "unknown"
  | "usage";

export interface EventFact {
  label: string;
  value: string;
  active?: boolean | undefined;
}

export interface EventCallout {
  title: string;
  detail: string;
  tone: EventTone;
}

export interface EventAction {
  action: "copyCheckpoint" | "startFollowUp" | "rewind";
  label: string;
  tone: EventActionTone;
  target?: string | undefined;
}

export interface EventPresentationInput {
  kind: EventKind;
  label?: string | undefined;
  status?: string | undefined;
  stopReason?: string | undefined;
  checkpointId?: string | undefined;
  checkpointPending?: boolean | undefined;
  updatedAt?: string | undefined;
  used?: number | undefined;
  size?: number | undefined;
  costLabel?: string | undefined;
  modeName?: string | undefined;
  modeCount?: number | undefined;
  configCount?: number | undefined;
  commandCount?: number | undefined;
  sessionId?: string | undefined;
  sessionTitle?: string | undefined;
  resourceLabel?: string | undefined;
}

export interface EventPresentation {
  title: string;
  detail: string;
  tone: EventTone;
  icon: string;
  statusLabel?: string | undefined;
  openByDefault: boolean;
  facts: EventFact[];
  callout?: EventCallout | undefined;
  actions?: EventAction[] | undefined;
}

export function buildEventPresentation(input: EventPresentationInput): EventPresentation {
  switch (input.kind) {
    case "checkpoint":
      return checkpointPresentation(input);
    case "completion":
      return completionPresentation(input);
    case "usage":
      return usagePresentation(input);
    case "mode":
      return {
        title: "Agent mode updated",
        detail: input.modeName || input.label || "Provider mode changed",
        tone: "info",
        icon: "settings",
        statusLabel: "Mode",
        openByDefault: false,
        facts: compactFacts([
          input.modeName ? { label: "Active", value: input.modeName, active: true } : undefined,
          input.modeCount ? { label: "Modes", value: formatCount(input.modeCount) } : undefined
        ])
      };
    case "config":
      return {
        title: "Session controls updated",
        detail: countDetail(input.configCount || 0, "configurable option"),
        tone: "info",
        icon: "settings",
        statusLabel: "Controls",
        openByDefault: false,
        facts: input.configCount ? [{ label: "Options", value: formatCount(input.configCount) }] : []
      };
    case "commands":
      return {
        title: "Slash commands available",
        detail: `${countDetail(input.commandCount || 0, "command")} exposed by provider`,
        tone: "info",
        icon: "terminal",
        statusLabel: "Commands",
        openByDefault: false,
        facts: input.commandCount ? [{ label: "Commands", value: formatCount(input.commandCount) }] : []
      };
    case "session":
      return {
        title: "Provider session updated",
        detail: input.sessionTitle || "Session metadata changed",
        tone: "info",
        icon: "history",
        statusLabel: "Session",
        openByDefault: false,
        facts: compactFacts([
          input.sessionId ? { label: "Session", value: input.sessionId, active: true } : undefined,
          input.updatedAt ? { label: "Updated", value: input.updatedAt } : undefined
        ])
      };
    case "resource":
      return {
        title: "Resource attached",
        detail: input.resourceLabel || input.label || "External context attached",
        tone: "info",
        icon: "file",
        statusLabel: "Resource",
        openByDefault: false,
        facts: input.resourceLabel ? [{ label: "Source", value: input.resourceLabel }] : []
      };
    case "unknown":
      return {
        title: input.label || "Unknown provider event",
        detail: "Raw event data is available for inspection.",
        tone: "warning",
        icon: "diagnostics",
        statusLabel: "Raw",
        openByDefault: true,
        facts: []
      };
    default:
      return {
        title: input.label || "Event",
        detail: "Provider event recorded.",
        tone: "info",
        icon: "spark",
        openByDefault: false,
        facts: []
      };
  }
}

function checkpointPresentation(input: EventPresentationInput): EventPresentation {
  const checkpointId = cleanText(input.checkpointId);
  const checkpointRef = checkpointId ? shortIdentifier(checkpointId) : "";
  const actions: EventAction[] = [
    {
      action: "startFollowUp",
      label: "Start follow-up",
      tone: "primary"
    },
    {
      action: "rewind",
      label: checkpointRef ? "Rewind to checkpoint" : "Rewind latest turn",
      tone: "default",
      target: checkpointId || undefined
    }
  ];
  if (checkpointId) {
    actions.unshift({
      action: "copyCheckpoint",
      label: "Copy checkpoint",
      tone: "default",
      target: checkpointId
    });
  }
  return {
    title: "Checkpoint saved",
    detail: checkpointRef ? `${checkpointRef} can start follow-ups or restore this lane.` : input.label || "CrabDB saved a durable recovery point.",
    tone: "success",
    icon: "history",
    statusLabel: "Durable",
    openByDefault: false,
    facts: compactFacts([
      checkpointId ? { label: "Checkpoint", value: checkpointId, active: true } : undefined,
      input.updatedAt ? { label: "Saved", value: input.updatedAt } : undefined,
      { label: "Recovery", value: "follow-up / rewind" }
    ]),
    callout: {
      title: "Durable recovery point",
      detail: checkpointRef
        ? `${checkpointRef} is available for follow-up starts, rewind, and failed-attempt preservation.`
        : "Available for follow-up starts, rewind, and failed-attempt preservation.",
      tone: "success"
    },
    actions
  };
}

function completionPresentation(input: EventPresentationInput): EventPresentation {
  const status = normalize(input.status);
  if (status === "failed") {
    return {
      title: "Turn failed",
      detail: input.label || "The provider stopped before producing an applyable turn.",
      tone: "risk",
      icon: "diagnostics",
      statusLabel: input.stopReason || "failed",
      openByDefault: true,
      facts: input.stopReason ? [{ label: "Stop", value: input.stopReason }] : []
    };
  }
  if (status === "cancelled") {
    return {
      title: "Turn cancelled",
      detail: input.label || "The current provider turn was cancelled.",
      tone: "risk",
      icon: "stop",
      statusLabel: "cancelled",
      openByDefault: true,
      facts: []
    };
  }
  if (status === "pending" || input.checkpointPending) {
    return {
      title: "Turn finishing",
      detail: input.label || "Turn complete; checkpoint pending.",
      tone: "warning",
      icon: "refresh",
      statusLabel: input.stopReason || "pending",
      openByDefault: false,
      facts: [{ label: "Checkpoint", value: "pending", active: true }]
    };
  }
  return {
    title: "Turn completed",
    detail: input.label || "The provider turn completed.",
    tone: "success",
    icon: "check",
    statusLabel: input.stopReason || "completed",
    openByDefault: false,
    facts: []
  };
}

function usagePresentation(input: EventPresentationInput): EventPresentation {
  const used = count(input.used || 0);
  const size = count(input.size || 0);
  const pct = size > 0 ? Math.min(100, Math.round((used / size) * 100)) : 0;
  const tone: EventTone = pct >= 90 ? "risk" : pct >= 70 ? "warning" : "info";
  return {
    title: "Context usage",
    detail: `${formatCount(used)} of ${formatCount(size)} tokens`,
    tone,
    icon: "diagnostics",
    statusLabel: `${pct}%`,
    openByDefault: pct >= 90,
    facts: compactFacts([
      { label: "Used", value: formatCount(used) },
      { label: "Window", value: formatCount(size) },
      input.costLabel ? { label: "Cost", value: input.costLabel } : undefined
    ])
  };
}

function countDetail(countValue: number, label: string): string {
  const safeCount = count(countValue);
  return `${formatCount(safeCount)} ${label}${safeCount === 1 ? "" : "s"}`;
}

function normalize(value: string | undefined): string {
  return String(value || "").toLowerCase();
}

function cleanText(value: string | undefined): string {
  return String(value || "").replace(/\s+/g, " ").trim();
}

function shortIdentifier(value: string): string {
  if (value.length <= 28) {
    return value;
  }
  return `${value.slice(0, 14)}...${value.slice(-10)}`;
}

function count(value: number): number {
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

function formatCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

function compactFacts(values: Array<EventFact | undefined>): EventFact[] {
  return values.filter((value): value is EventFact => Boolean(value));
}

import type { AgentTask, AgentTaskStatus, MergeQueueEntry } from "../trail/TaskRepository";
import type { CoordinationSeverity } from "../shared/coordinationSummary";

export type TaskTreeMode = "tasks" | "reviews" | "queue";

export interface TreeIconPresentation {
  id: string;
  color?: string | undefined;
}

export interface TreeCommandPresentation {
  command: string;
  title: string;
}

export interface TreeItemPresentation {
  label: string;
  description?: string | undefined;
  tooltip: string;
  icon: TreeIconPresentation;
  accessibilityLabel: string;
  command?: TreeCommandPresentation | undefined;
}

export function buildTaskTreePresentation(task: AgentTask, mode: Exclude<TaskTreeMode, "queue"> = "tasks"): TreeItemPresentation {
  const label = cleanText(task.title) || cleanText(task.lane) || cleanText(task.id) || "Untitled agent task";
  const lane = cleanText(task.lane);
  const provider = cleanText(task.provider);
  const model = cleanText(task.model);
  const coordination = task.coordination;
  const statusLabel = taskStatusLabel(task.status, coordination?.severity);
  const changedPaths = Array.isArray(task.changedPaths) ? task.changedPaths.map(cleanText).filter(Boolean) : [];
  const coordinationLabels = Array.isArray(coordination?.labels) ? coordination.labels.map(cleanText).filter(Boolean) : [];
  const coordinationIssues = Array.isArray(coordination?.issues) ? coordination.issues : [];
  const nextAction = cleanText(task.nextAction);
  const descriptionParts =
    mode === "reviews"
      ? [
          reviewLeadLabel(statusLabel, nextAction, changedPaths.length),
          changedPaths.length ? countLabel(changedPaths.length, "change") : "",
          ...coordinationLabels.slice(0, 2),
          provider ? shortenMiddle(provider, 28) : ""
        ]
      : [
          statusLabel,
          provider ? shortenMiddle(provider, 28) : "",
          lane && lane !== label ? shortenMiddle(shortRef(lane), 34) : "",
          changedPaths.length ? countLabel(changedPaths.length, "change") : "",
          ...coordinationLabels.slice(0, 2)
        ];
  const description = compactJoin(
    descriptionParts,
    " - "
  );
  const tooltip = compactLines([
    label,
    `Status: ${statusLabel}`,
    nextAction ? `Next action: ${nextAction}` : reviewFallbackAction(mode, statusLabel, changedPaths.length),
    provider ? `Provider: ${provider}` : "",
    model ? `Model: ${model}` : "",
    lane ? `Lane: ${lane}` : "",
    cleanText(task.sessionId) ? `Session: ${cleanText(task.sessionId)}` : "",
    cleanText(task.acpSessionId) ? `ACP session: ${cleanText(task.acpSessionId)}` : "",
    `Changed paths: ${changedPaths.length}`,
    ...changedPaths.slice(0, 5).map((path) => ` - ${path}`),
    changedPaths.length > 5 ? ` - ${changedPaths.length - 5} more` : "",
    coordinationLabels.length ? `Coordination: ${coordinationLabels.join(", ")}` : "",
    ...coordinationIssues.slice(0, 4).map((issue) => `${toneLabel(issue.tone)}: ${cleanText(issue.message)}`),
    cleanText(task.workdir) ? `Workdir: ${cleanText(task.workdir)}` : "",
    cleanText(task.latestCheckpoint) ? `Checkpoint: ${cleanText(task.latestCheckpoint)}` : "",
    cleanText(task.updatedAt) ? `Updated: ${cleanText(task.updatedAt)}` : ""
  ]);

  return {
    label,
    description,
    tooltip,
    icon: taskStatusIcon(task.status, coordination?.severity),
    accessibilityLabel: compactJoin(
      [
        label,
        statusLabel,
        changedPaths.length ? countLabel(changedPaths.length, "changed path") : "",
        nextAction ? `next action ${nextAction}` : ""
      ],
      ", "
    )
  };
}

export function buildQueueItemTreePresentation(entry: MergeQueueEntry): TreeItemPresentation {
  const sourceRef = cleanText(entry.sourceRef) || cleanText(entry.id) || "Queued merge";
  const targetRef = cleanText(entry.targetRef) || "main";
  const statusLabel = queueStatusLabel(entry.status);
  const priority = Number.isFinite(entry.priority) ? entry.priority : 0;
  const label = shortRef(sourceRef);
  const queueReason = queueReasonLabel(entry.raw);
  const description = compactJoin([statusLabel, `to ${shortRef(targetRef)}`, `P${priority}`, queueReason ? shortenMiddle(queueReason, 32) : ""], " - ");
  const tooltip = compactLines([
    `Merge queue: ${cleanText(entry.id) || sourceRef}`,
    `Source: ${sourceRef}`,
    `Target: ${targetRef}`,
    `Status: ${statusLabel}`,
    `Priority: ${priority}`,
    queueReason ? `Reason: ${queueReason}` : "",
    entry.createdAt !== undefined ? `Created: ${formatQueueTime(entry.createdAt)}` : "",
    entry.updatedAt !== undefined ? `Updated: ${formatQueueTime(entry.updatedAt)}` : ""
  ]);

  return {
    label,
    description,
    tooltip,
    icon: queueStatusIcon(entry.status),
    accessibilityLabel: `${label}, ${statusLabel}, merge into ${shortRef(targetRef)}`
  };
}

export function buildGroupTreePresentation(input: {
  id: string;
  label: string;
  count: number;
  kind: "task" | "queue";
}): TreeItemPresentation {
  const count = Math.max(0, Math.floor(input.count));
  const itemKind = input.kind === "queue" ? "entry" : "task";
  const label = `${cleanText(input.label) || "Group"} (${count})`;
  const description = countLabel(count, itemKind);
  return {
    label,
    description,
    tooltip: `${cleanText(input.label) || "Group"}: ${countLabel(count, itemKind)}`,
    icon: input.kind === "queue" ? queueGroupIcon(input.id) : taskGroupIcon(input.id),
    accessibilityLabel: `${cleanText(input.label) || "Group"}, ${countLabel(count, itemKind)}`
  };
}

export function buildEmptyTreePresentation(mode: TaskTreeMode, error?: string | undefined): TreeItemPresentation {
  if (cleanText(error)) {
    const viewLabel = mode === "queue" ? "merge queue" : mode === "reviews" ? "reviews" : "agent tasks";
    return {
      label: "Trail data unavailable",
      description: "Refresh or open settings",
      tooltip: compactLines([`Could not load ${viewLabel}.`, cleanText(error)]),
      icon: { id: "warning", color: "charts.yellow" },
      accessibilityLabel: `Trail data unavailable, ${viewLabel}`,
      command: {
        command: "trail.refreshTasks",
        title: "Refresh Trail views"
      }
    };
  }

  const labels: Record<TaskTreeMode, [string, string, TreeCommandPresentation]> = {
    tasks: ["No agent tasks yet", "Start a new task", { command: "trail.newAgentTask", title: "New Agent Task" }],
    reviews: ["No tasks need review", "Start a task to create review evidence", { command: "trail.newAgentTask", title: "New Agent Task" }],
    queue: ["No queued merges", "Open review to queue a lane", { command: "trail.openLatestReview", title: "Open Latest Review" }]
  };
  const [label, description, command] = labels[mode];
  return {
    label,
    description,
    tooltip: `${label}. ${description}.`,
    icon: { id: "info" },
    accessibilityLabel: `${label}, ${description}`,
    command
  };
}

export function normalizeTreeStatus(status: AgentTaskStatus | string | undefined): string {
  return cleanText(status).toLowerCase() || "active";
}

export function taskTreeGroupStatus(task: Pick<AgentTask, "status" | "coordination">): string {
  if (task.coordination?.severity === "blocked") {
    return "blocked";
  }
  if (task.coordination?.severity === "warning") {
    return "attention";
  }
  return normalizeTreeStatus(task.status);
}

function taskStatusLabel(status: AgentTaskStatus, severity: CoordinationSeverity | undefined): string {
  if (severity === "blocked") {
    return "Blocked";
  }
  if (severity === "warning") {
    return "Needs attention";
  }
  switch (normalizeTreeStatus(status)) {
    case "ready":
      return "Ready to review";
    case "dirty":
      return "Needs checkpoint";
    case "blocked":
      return "Blocked";
    case "conflicted":
      return "Conflicted";
    case "applied":
      return "Applied";
    case "active":
      return "Running";
    case "empty":
      return "Empty";
    default:
      return titleCaseStatus(status);
  }
}

function reviewLeadLabel(statusLabel: string, nextAction: string, changedPaths: number): string {
  if (nextAction) {
    return `Next: ${shortenMiddle(nextAction, 30)}`;
  }
  if (changedPaths > 0) {
    return statusLabel === "Ready to review" ? "Review changes" : statusLabel;
  }
  return "Review transcript";
}

function reviewFallbackAction(mode: Exclude<TaskTreeMode, "queue">, statusLabel: string, changedPaths: number): string {
  if (mode !== "reviews") {
    return "";
  }
  if (changedPaths > 0) {
    return `Suggested action: ${statusLabel === "Ready to review" ? "Review changes" : statusLabel}`;
  }
  return "Suggested action: Review transcript";
}

function queueStatusLabel(status: string): string {
  switch (normalizeTreeStatus(status)) {
    case "running":
      return "Running";
    case "queued":
      return "Queued";
    case "conflicted":
      return "Conflicted";
    case "failed":
      return "Failed";
    case "merged":
      return "Merged";
    case "cancelled":
      return "Cancelled";
    default:
      return titleCaseStatus(status);
  }
}

function taskStatusIcon(status: AgentTaskStatus, severity: CoordinationSeverity | undefined): TreeIconPresentation {
  if (severity === "blocked") {
    return { id: "warning", color: "charts.yellow" };
  }
  if (severity === "warning") {
    return { id: "issues", color: "charts.yellow" };
  }
  switch (normalizeTreeStatus(status)) {
    case "ready":
      return { id: "pass", color: "charts.green" };
    case "dirty":
      return { id: "circle-filled", color: "charts.yellow" };
    case "blocked":
      return { id: "warning", color: "charts.yellow" };
    case "conflicted":
      return { id: "error", color: "charts.red" };
    case "applied":
      return { id: "check", color: "charts.green" };
    case "active":
      return { id: "sync~spin", color: "charts.blue" };
    default:
      return { id: "circle-outline" };
  }
}

function taskGroupIcon(group: string): TreeIconPresentation {
  switch (normalizeTreeStatus(group)) {
    case "attention":
      return { id: "issues", color: "charts.yellow" };
    case "ready":
      return { id: "checklist", color: "charts.green" };
    case "blocked":
      return { id: "lock", color: "charts.yellow" };
    case "conflicted":
      return { id: "warning", color: "charts.red" };
    case "dirty":
      return { id: "diff", color: "charts.yellow" };
    case "active":
      return { id: "sync~spin", color: "charts.blue" };
    case "applied":
      return { id: "verified", color: "charts.green" };
    default:
      return { id: "list-tree" };
  }
}

function queueGroupIcon(group: string): TreeIconPresentation {
  switch (normalizeTreeStatus(group)) {
    case "running":
      return { id: "sync~spin", color: "charts.blue" };
    case "queued":
      return { id: "git-merge", color: "charts.purple" };
    case "conflicted":
      return { id: "warning", color: "charts.red" };
    case "failed":
      return { id: "error", color: "charts.red" };
    case "merged":
      return { id: "pass", color: "charts.green" };
    case "cancelled":
      return { id: "circle-slash" };
    default:
      return { id: "list-tree" };
  }
}

function queueStatusIcon(status: string): TreeIconPresentation {
  switch (normalizeTreeStatus(status)) {
    case "running":
      return { id: "sync~spin", color: "charts.blue" };
    case "queued":
      return { id: "git-merge", color: "charts.purple" };
    case "conflicted":
      return { id: "warning", color: "charts.red" };
    case "failed":
      return { id: "error", color: "charts.red" };
    case "merged":
      return { id: "pass", color: "charts.green" };
    case "cancelled":
      return { id: "circle-slash" };
    default:
      return { id: "circle-outline" };
  }
}

function queueReasonLabel(raw: unknown): string {
  const record = asRecord(raw);
  return (
    stringChoice(record, ["reason", "message", "detail", "explanation", "blocked_reason", "blockedReason", "failure", "error"]) ||
    stringChoice(asRecord(record.readiness), ["reason", "message", "detail"]) ||
    stringChoice(asRecord(record.review), ["reason", "message", "detail"])
  );
}

function formatQueueTime(value: number | string): string {
  if (typeof value === "number" && Number.isFinite(value)) {
    return new Date(value * 1000).toISOString();
  }
  return cleanText(value);
}

function countLabel(count: number, singular: string): string {
  const safeCount = Math.max(0, Math.floor(count));
  return `${safeCount} ${singular}${safeCount === 1 ? "" : "s"}`;
}

function titleCaseStatus(value: unknown): string {
  const status = cleanText(value) || "Unknown";
  return status
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map((part) => `${part.slice(0, 1).toUpperCase()}${part.slice(1).toLowerCase()}`)
    .join(" ");
}

function shortRef(value: string): string {
  return value.replace(/^refs\/(?:heads|lanes|remotes)\//, "");
}

function shortenMiddle(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value;
  }
  const head = Math.max(4, Math.floor((maxLength - 3) * 0.65));
  const tail = Math.max(4, maxLength - 3 - head);
  return `${value.slice(0, head)}...${value.slice(-tail)}`;
}

function toneLabel(value: string): string {
  return normalizeTreeStatus(value) === "blocked" ? "Blocked" : "Warning";
}

function compactJoin(values: string[], separator: string): string {
  return values.map(cleanText).filter(Boolean).join(separator);
}

function compactLines(values: string[]): string {
  return values.map(cleanLine).filter((value) => value.trim()).join("\n");
}

function cleanText(value: unknown): string {
  return typeof value === "string" ? value.replace(/\s+/g, " ").trim() : "";
}

function cleanLine(value: unknown): string {
  return typeof value === "string" ? value.replace(/[ \t]+/g, " ").trimEnd() : "";
}

function stringChoice(record: Record<string, unknown>, keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) {
      return cleanText(value);
    }
  }
  return "";
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

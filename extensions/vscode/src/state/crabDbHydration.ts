import type { TaskView } from "../crabdb/TaskRepository";
import type { RenderNode } from "../shared/renderModel";

export function hydrateTaskView(view: TaskView): RenderNode[] {
  const nodes: RenderNode[] = [];
  const task = view.task;

  view.turns.forEach((turnValue, turnIndex) => {
    const turnWrapper = asRecord(turnValue);
    const turn = asRecord(turnWrapper.turn);
    const turnId = stringField(turn, "turn_id") || stringField(turn, "turnId") || `turn-${turnIndex + 1}`;
    const status = renderStatus(stringField(turn, "status"));
    const messages = arrayField(turnWrapper, "messages");

    messages.forEach((messageValue, messageIndex) => {
      const message = asRecord(messageValue);
      const role = stringField(message, "role") === "user" ? "user" : "assistant";
      const body = stringField(message, "body") || "";
      nodes.push({
        id: `crabdb-message:${turnId}:${messageIndex}`,
        kind: "message",
        taskId: task.id,
        lane: task.lane,
        turnId,
        provider: task.provider,
        source: "crabdb",
        status,
        createdAt: timestampString(message.created_at),
        updatedAt: timestampString(message.created_at),
        raw: messageValue,
        role,
        content: [{ type: "text", text: body }],
        text: body,
        streaming: false
      });
    });

    const toolSummaries = arrayField(turnWrapper, "tool_summaries");
    toolSummaries.forEach((summary, summaryIndex) => {
      nodes.push({
        id: `crabdb-tool:${turnId}:${summaryIndex}`,
        kind: "tool",
        taskId: task.id,
        lane: task.lane,
        turnId,
        provider: task.provider,
        source: "crabdb",
        status: "completed",
        raw: summary,
        toolCallId: `summary-${turnId}-${summaryIndex}`,
        title: String(summary),
        toolKind: "other",
        toolStatus: "completed",
        locations: [],
        content: []
      });
    });

    const checkpoint = stringField(turnWrapper, "checkpoint") || stringField(turn, "after_change");
    if (checkpoint) {
      nodes.push({
        id: `crabdb-checkpoint:${turnId}`,
        kind: "checkpoint",
        taskId: task.id,
        lane: task.lane,
        turnId,
        provider: task.provider,
        source: "crabdb",
        status: "completed",
        raw: turnValue,
        checkpointId: checkpoint,
        label: `Checkpoint ${checkpoint}`
      });
    }
  });

  return nodes;
}

export function mergeHydratedNodes(hydrated: RenderNode[], current: RenderNode[]): RenderNode[] {
  const hydratedIds = new Set(hydrated.map((node) => node.id));
  const live = current.filter((node) => {
    if (node.source === "crabdb") {
      return false;
    }
    return !hydratedIds.has(node.id) && ["pending", "in_progress"].includes(node.status);
  });
  return [...hydrated, ...live];
}

function renderStatus(status: string | undefined): RenderNode["status"] {
  switch (status) {
    case "failed":
      return "failed";
    case "cancelled":
      return "cancelled";
    case "pending":
      return "pending";
    case "in_progress":
    case "running":
      return "in_progress";
    default:
      return "completed";
  }
}

function timestampString(value: unknown): string | undefined {
  if (typeof value !== "number") {
    return undefined;
  }
  const millis = value > 10_000_000_000 ? value : value * 1000;
  return new Date(millis).toISOString();
}

function arrayField(record: Record<string, unknown>, key: string): unknown[] {
  const value = record[key];
  return Array.isArray(value) ? value : [];
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

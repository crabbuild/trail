import type { TaskView } from "../crabdb/TaskRepository";
import type { SessionUpdate } from "../shared/acpTypes";
import { applyRenderPatches, reduceSessionUpdate } from "../shared/acpRenderReducers";
import type { MessageNode, RenderNode } from "../shared/renderModel";

export function hydrateTaskView(view: TaskView): RenderNode[] {
  const nodes: RenderNode[] = [];
  const task = view.task;

  view.turns.forEach((turnValue, turnIndex) => {
    const turnNodes: RenderNode[] = [];
    const turnWrapper = asRecord(turnValue);
    const turn = asRecord(turnWrapper.turn);
    const turnId = stringField(turn, "turn_id") || stringField(turn, "turnId") || `turn-${turnIndex + 1}`;
    const status = renderStatus(stringField(turn, "status"));
    const messages = arrayField(turnWrapper, "messages");
    const events = arrayField(turnWrapper, "events");
    const turnCompletedAt =
      timestampString(turn.ended_at) || timestampString(turn.updated_at) || timestampString(turnWrapper.ended_at);

    turnNodes.push(...hydrateTurnTimeline(messages, events, view, turnId, status));

    const toolSummaries = turnNodes.some((node) => node.kind === "tool") ? [] : arrayField(turnWrapper, "tool_summaries");
    toolSummaries.forEach((summary, summaryIndex) => {
      turnNodes.push({
        id: `crabdb-tool:${turnId}:${summaryIndex}`,
        kind: "tool",
        taskId: task.id,
        lane: task.lane,
        turnId,
        provider: task.provider,
        source: "crabdb",
        status: "completed",
        createdAt: turnCompletedAt,
        updatedAt: turnCompletedAt,
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
      turnNodes.push({
        id: `crabdb-checkpoint:${turnId}`,
        kind: "checkpoint",
        taskId: task.id,
        lane: task.lane,
        turnId,
        provider: task.provider,
        source: "crabdb",
        status: "completed",
        createdAt: turnCompletedAt,
        updatedAt: turnCompletedAt,
        raw: turnValue,
        checkpointId: checkpoint,
        label: `Checkpoint ${checkpoint}`
      });
    }
    nodes.push(...turnNodes);
  });

  return nodes;
}

interface HydratedMessageEntry {
  index: number;
  messageId?: string | undefined;
  node: MessageNode;
}

function hydrateTurnTimeline(
  messages: unknown[],
  events: unknown[],
  view: TaskView,
  turnId: string,
  status: RenderNode["status"]
): RenderNode[] {
  const messageEntries = messages.map((messageValue, messageIndex) =>
    hydrateMessageEntry(messageValue, messageIndex, view, turnId, status)
  );
  const messagesById = new Map<string, HydratedMessageEntry>();
  for (const entry of messageEntries) {
    if (entry.messageId && !messagesById.has(entry.messageId)) {
      messagesById.set(entry.messageId, entry);
    }
  }

  let nodes: RenderNode[] = [];
  const usedMessageIndexes = new Set<number>();
  let placedMessages = 0;

  for (const eventValue of events) {
    const event = asRecord(eventValue);
    const eventType = stringField(event, "event_type") || stringField(event, "eventType");
    const messageId = recordIdField(event, "message_id") || recordIdField(event, "messageId");
    if (eventType === "message_added" && messageId) {
      const entry = messagesById.get(messageId);
      if (entry && !usedMessageIndexes.has(entry.index)) {
        const timestamp = timestampString(event.created_at);
        nodes.push({
          ...entry.node,
          createdAt: entry.node.createdAt || timestamp,
          updatedAt: timestamp || entry.node.updatedAt
        });
        usedMessageIndexes.add(entry.index);
        placedMessages += 1;
      }
      continue;
    }
    nodes = hydrateSessionUpdateEvent(nodes, eventValue, view, turnId, status);
  }

  if (placedMessages === 0) {
    return sortHydratedTurnNodes([
      ...messageEntries.map((entry) => entry.node),
      ...hydrateToolEvents(events, view, turnId, status)
    ]);
  }

  const unplacedMessages = messageEntries
    .filter((entry) => !usedMessageIndexes.has(entry.index))
    .map((entry) => entry.node);
  if (unplacedMessages.length) {
    nodes.push(...sortHydratedTurnNodes(unplacedMessages));
  }
  return nodes;
}

function hydrateMessageEntry(
  messageValue: unknown,
  messageIndex: number,
  view: TaskView,
  turnId: string,
  status: RenderNode["status"]
): HydratedMessageEntry {
  const task = view.task;
  const message = asRecord(messageValue);
  const messageId = recordIdField(message, "message_id") || recordIdField(message, "messageId") || recordIdField(message, "id");
  const role = stringField(message, "role") === "user" ? "user" : "assistant";
  const body = stringField(message, "body") || "";
  const createdAt = timestampString(message.created_at);
  return {
    index: messageIndex,
    messageId,
    node: {
      id: `crabdb-message:${turnId}:${messageId || messageIndex}`,
      kind: "message",
      taskId: task.id,
      lane: task.lane,
      turnId,
      provider: task.provider,
      source: "crabdb",
      status,
      createdAt,
      updatedAt: createdAt,
      raw: messageValue,
      role,
      content: [{ type: "text", text: body }],
      text: body,
      streaming: false
    }
  };
}

function sortHydratedTurnNodes(nodes: RenderNode[]): RenderNode[] {
  return nodes
    .map((node, index) => ({ node, index }))
    .sort((left, right) => {
      const time = nodeSortTime(left.node) - nodeSortTime(right.node);
      if (time !== 0) {
        return time;
      }
      const phase = nodeSortPhase(left.node) - nodeSortPhase(right.node);
      if (phase !== 0) {
        return phase;
      }
      return left.index - right.index;
    })
    .map((item) => item.node);
}

function nodeSortTime(node: RenderNode): number {
  const millis = Date.parse(node.createdAt || node.updatedAt || "");
  return Number.isFinite(millis) ? millis : Number.MAX_SAFE_INTEGER;
}

function nodeSortPhase(node: RenderNode): number {
  if (node.kind === "message" && node.role === "user") {
    return 0;
  }
  if (node.kind === "tool") {
    return 10;
  }
  if (node.kind === "terminal" || node.kind === "diff" || node.kind === "approval") {
    return 11;
  }
  if (node.kind === "message") {
    return 20;
  }
  if (node.kind === "checkpoint" || node.kind === "completion") {
    return 90;
  }
  return 50;
}

function hydrateToolEvents(
  events: unknown[],
  view: TaskView,
  turnId: string,
  fallbackStatus: RenderNode["status"]
): RenderNode[] {
  let nodes: RenderNode[] = [];
  for (const eventValue of events) {
    nodes = hydrateSessionUpdateEvent(nodes, eventValue, view, turnId, fallbackStatus);
  }
  return nodes;
}

function hydrateSessionUpdateEvent(
  nodes: RenderNode[],
  eventValue: unknown,
  view: TaskView,
  turnId: string,
  fallbackStatus: RenderNode["status"]
): RenderNode[] {
  const task = view.task;
  const event = asRecord(eventValue);
  const update = sessionUpdateFromEvent(event);
  if (!update) {
    return nodes;
  }
  const timestamp = timestampString(event.created_at);
  const patches = reduceSessionUpdate(update, {
    taskId: task.id,
    lane: task.lane,
    acpSessionId: task.acpSessionId,
    currentTurnId: turnId,
    provider: task.provider,
    now: () => timestamp || new Date(0).toISOString()
  }).map((patch) => {
    if (!patch.node) {
      return patch;
    }
    const status = hydratedNodeStatus(patch.node.status, fallbackStatus);
    return {
      ...patch,
      node: {
        ...patch.node,
        source: "crabdb" as const,
        status,
        ...(patch.node.kind === "tool" && isOpenStatus(patch.node.toolStatus) && !isOpenStatus(status)
          ? { toolStatus: status }
          : {}),
        createdAt: patch.node.createdAt || timestamp,
        updatedAt: timestamp || patch.node.updatedAt,
        raw: patch.node.raw ?? eventValue
      }
    };
  });
  return applyRenderPatches(nodes, patches);
}

function hydratedNodeStatus(status: RenderNode["status"], fallbackStatus: RenderNode["status"]): RenderNode["status"] {
  if (isOpenStatus(status) && !isOpenStatus(fallbackStatus)) {
    return fallbackStatus;
  }
  return status;
}

function isOpenStatus(status: string): boolean {
  return status === "pending" || status === "in_progress";
}

function sessionUpdateFromEvent(event: Record<string, unknown>): SessionUpdate | undefined {
  const eventType = stringField(event, "event_type") || stringField(event, "eventType");
  const payload = asRecord(event.payload);
  if (
    eventType === "tool_call" ||
    eventType === "tool_call_update" ||
    eventType === "plan_update" ||
    eventType === "plan" ||
    eventType === "available_commands_update" ||
    eventType === "acp_available_commands_update" ||
    (typeof eventType === "string" && eventType.startsWith("acp_"))
  ) {
    return sessionUpdatePayload(payload);
  }
  if (eventType === "span_started") {
    const attributes = asRecord(payload.attributes);
    return sessionUpdatePayload(attributes);
  }
  if (eventType === "span_ended") {
    const result = asRecord(payload.result);
    return sessionUpdatePayload(result);
  }
  return undefined;
}

function sessionUpdatePayload(payload: Record<string, unknown>): SessionUpdate | undefined {
  if (!isSessionUpdate(payload)) {
    return undefined;
  }
  if (payload.sessionUpdate === "available_commands_update" && !Array.isArray(payload.availableCommands)) {
    const commandNames = arrayField(payload, "command_names").filter((name): name is string => typeof name === "string");
    return {
      ...payload,
      availableCommands: commandNames.map((name) => ({ name, description: "" }))
    } as SessionUpdate;
  }
  return payload;
}

function isSessionUpdate(value: unknown): value is SessionUpdate {
  return typeof asRecord(value).sessionUpdate === "string";
}

export function mergeHydratedNodes(hydrated: RenderNode[], current: RenderNode[]): RenderNode[] {
  const hydratedIds = new Set(hydrated.map((node) => node.id));
  const hasHydratedTranscript = hydrated.some((node) => node.turnId);
  const live = current.filter((node) => {
    if (node.source === "crabdb") {
      return false;
    }
    if (hasHydratedTranscript && node.kind === "completion" && node.checkpointPending) {
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

function recordIdField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  if (typeof value === "string") {
    return value;
  }
  const nested = asRecord(value);
  return stringField(nested, "id") || stringField(nested, "0");
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

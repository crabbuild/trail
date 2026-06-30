import type { TaskView } from "../crabdb/TaskRepository";
import type { SessionUpdate } from "../shared/acpTypes";
import { applyRenderPatches, reduceSessionUpdate } from "../shared/acpRenderReducers";
import type { MessageNode, RenderNode } from "../shared/renderModel";

export function hydrateTaskView(view: TaskView): RenderNode[] {
  const nodes: RenderNode[] = [];
  const task = view.task;
  let nextTimelineOrder = 0;

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

    const toolSummaries = turnNodes.some((node) => node.kind === "tool")
      ? []
      : arrayField(turnWrapper, "tool_summaries").filter((summary) => !isInternalToolSummary(summary));
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
    const orderedTurnNodes = assignTimelineOrder(turnNodes, nextTimelineOrder);
    nextTimelineOrder += orderedTurnNodes.length;
    nodes.push(...orderedTurnNodes);
  });

  return ensureUniqueHydratedNodeIds(nodes);
}

function assignTimelineOrder(nodes: RenderNode[], start: number): RenderNode[] {
  return nodes.map((node, index) => (
    node.timelineOrder === undefined ? { ...node, timelineOrder: start + index + 1 } : node
  ));
}

function ensureUniqueHydratedNodeIds(nodes: RenderNode[]): RenderNode[] {
  const used = new Set<string>();
  let changed = false;
  const next = nodes.map((node) => {
    if (!used.has(node.id)) {
      used.add(node.id);
      return node;
    }
    changed = true;
    const id = nextHydratedCollisionNodeId(used, node);
    used.add(id);
    return {
      ...node,
      id
    };
  });
  return changed ? next : nodes;
}

function nextHydratedCollisionNodeId(used: Set<string>, node: RenderNode): string {
  const suffix = sanitizedCollisionSuffix(
    [node.turnId, node.acpSessionId, node.source].filter(Boolean).join(":") ||
      `${node.taskId}:${node.lane}`
  );
  const base = suffix ? `${node.id}:${suffix}` : node.id;
  if (!used.has(base)) {
    return base;
  }
  for (let sequence = 2; sequence < Number.MAX_SAFE_INTEGER; sequence += 1) {
    const id = `${base}:${sequence}`;
    if (!used.has(id)) {
      return id;
    }
  }
  return `${base}:${Date.now()}`;
}

function sanitizedCollisionSuffix(value: string): string {
  return value
    .trim()
    .replace(/[^A-Za-z0-9_.:-]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function isInternalToolSummary(value: unknown): boolean {
  const text = String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
  return (
    /^acp prompt turn(?: \([^)]+\))?$/.test(text) ||
    /^span_(?:started|ended)(?: \([^)]+\))?$/.test(text)
  );
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
  const duplicateMessageIds = duplicateIds(messages.map(messageIdFromValue));
  const messageEntries = messages.map((messageValue, messageIndex) =>
    hydrateMessageEntry(messageValue, messageIndex, view, turnId, status, duplicateMessageIds)
  );
  const messagesById = new Map<string, HydratedMessageEntry[]>();
  for (const entry of messageEntries) {
    if (entry.messageId) {
      const queue = messagesById.get(entry.messageId);
      if (queue) {
        queue.push(entry);
      } else {
        messagesById.set(entry.messageId, [entry]);
      }
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
      const entry = takeNextMessageEntry(messagesById, messageId, usedMessageIndexes);
      if (entry) {
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
  status: RenderNode["status"],
  duplicateMessageIds: Set<string>
): HydratedMessageEntry {
  const task = view.task;
  const message = asRecord(messageValue);
  const messageId = messageIdFromValue(messageValue);
  const role = stringField(message, "role") === "user" ? "user" : "assistant";
  const body = stringField(message, "body") || "";
  const createdAt = timestampString(message.created_at);
  const nodeId = messageId && !duplicateMessageIds.has(messageId)
    ? messageId
    : messageId
      ? `${messageId}:${messageIndex}`
      : `${messageIndex}`;
  return {
    index: messageIndex,
    messageId,
    node: {
      id: `crabdb-message:${turnId}:${nodeId}`,
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
      acpMessageId: messageId,
      content: [{ type: "text", text: body }],
      text: body,
      streaming: false
    }
  };
}

function takeNextMessageEntry(
  messagesById: Map<string, HydratedMessageEntry[]>,
  messageId: string,
  usedMessageIndexes: Set<number>
): HydratedMessageEntry | undefined {
  const queue = messagesById.get(messageId);
  while (queue?.length) {
    const entry = queue.shift()!;
    if (!usedMessageIndexes.has(entry.index)) {
      return entry;
    }
  }
  return undefined;
}

function messageIdFromValue(messageValue: unknown): string | undefined {
  const message = asRecord(messageValue);
  return recordIdField(message, "message_id") || recordIdField(message, "messageId") || recordIdField(message, "id");
}

function duplicateIds(ids: Array<string | undefined>): Set<string> {
  const seen = new Set<string>();
  const duplicates = new Set<string>();
  for (const id of ids) {
    if (!id) {
      continue;
    }
    if (seen.has(id)) {
      duplicates.add(id);
    } else {
      seen.add(id);
    }
  }
  return duplicates;
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
  const orderedHydrated = orderHydratedNodesFromCurrent(hydrated, current);
  const hasHydratedTranscript = hydrated.some((node) => node.turnId);
  const hydratedForEquivalence = orderedHydrated.filter(
    (node) => !hasMoreCompleteLiveEquivalent(node, current, hasHydratedTranscript)
  );
  const matchedHydratedIds = new Set<string>();
  const live = current.filter((node) => {
    if (node.source === "crabdb") {
      return false;
    }
    if (hasHydratedTranscript && node.kind === "completion" && node.checkpointPending) {
      return false;
    }
    if (hasHydratedEquivalentNode(node, hydratedForEquivalence, matchedHydratedIds)) {
      return false;
    }
    return ["pending", "in_progress"].includes(node.status) || isPreservableCompletedLiveNode(node, hasHydratedTranscript);
  });
  return reindexTimelineOrder(ensureUniqueMergedNodeIds(orderTimelineScopesFromCurrent([...hydratedForEquivalence, ...live], current)));
}

function hasMoreCompleteLiveEquivalent(
  hydratedNode: RenderNode,
  current: RenderNode[],
  hasHydratedTranscript: boolean
): boolean {
  return current.some((node) =>
    isPreservableCompletedLiveNode(node, hasHydratedTranscript) &&
    hasOverlappingTimelineKey(node, hydratedNode) &&
    !hydratedNodeCanReplaceLiveNode(node, hydratedNode)
  );
}

function ensureUniqueMergedNodeIds(nodes: RenderNode[]): RenderNode[] {
  const used = new Set<string>();
  let changed = false;
  const next = nodes.map((node) => {
    if (!used.has(node.id)) {
      used.add(node.id);
      return node;
    }
    changed = true;
    const id = nextMergedCollisionNodeId(used, node);
    used.add(id);
    return {
      ...node,
      id
    };
  });
  return changed ? next : nodes;
}

function nextMergedCollisionNodeId(used: Set<string>, node: RenderNode): string {
  const suffix = sanitizedCollisionSuffix(
    [node.turnId, node.acpSessionId, node.source].filter(Boolean).join(":") ||
      `${node.taskId}:${node.lane}`
  );
  const base = suffix ? `${node.id}:${suffix}` : node.id;
  if (!used.has(base)) {
    return base;
  }
  for (let sequence = 2; sequence < Number.MAX_SAFE_INTEGER; sequence += 1) {
    const id = `${base}:${sequence}`;
    if (!used.has(id)) {
      return id;
    }
  }
  return `${base}:${Date.now()}`;
}

function reindexTimelineOrder(nodes: RenderNode[]): RenderNode[] {
  return nodes.map((node, index) => {
    const timelineOrder = index + 1;
    return node.timelineOrder === timelineOrder ? node : { ...node, timelineOrder };
  });
}

interface CurrentTimelineOrder {
  index: number;
  timelineOrder?: number | undefined;
}

function orderHydratedNodesFromCurrent(hydrated: RenderNode[], current: RenderNode[]): RenderNode[] {
  return orderTimelineScopesFromCurrent(hydrated, current);
}

function orderTimelineScopesFromCurrent(nodes: RenderNode[], current: RenderNode[]): RenderNode[] {
  const orderQueues = new Map<string, CurrentTimelineOrder[]>();
  current.forEach((node, index) => {
    for (const key of timelineOrderKeys(node)) {
      const queue = orderQueues.get(key);
      const order = { index, timelineOrder: node.timelineOrder };
      if (queue) {
        queue.push(order);
      } else {
        orderQueues.set(key, [order]);
      }
    }
  });
  if (!orderQueues.size) {
    return nodes;
  }
  const ordered: RenderNode[] = [];
  for (const segment of hydratedTimelineSegments(nodes)) {
    ordered.push(...orderHydratedSegmentFromCurrent(segment, orderQueues));
  }
  return ordered;
}

function hasHydratedEquivalentNode(
  node: RenderNode,
  hydrated: RenderNode[],
  matchedHydratedIds: Set<string>
): boolean {
  for (const hydratedNode of hydrated) {
    if (matchedHydratedIds.has(hydratedNode.id)) {
      continue;
    }
    if (hasOverlappingTimelineKey(node, hydratedNode) && hydratedNodeCanReplaceLiveNode(node, hydratedNode)) {
      matchedHydratedIds.add(hydratedNode.id);
      return true;
    }
  }
  return false;
}

function hasOverlappingTimelineKey(left: RenderNode, right: RenderNode): boolean {
  const rightKeys = timelineOrderKeys(right);
  return timelineOrderKeys(left).some((key) => rightKeys.includes(key));
}

function hydratedNodeCanReplaceLiveNode(liveNode: RenderNode, hydratedNode: RenderNode): boolean {
  if (liveNode.kind !== hydratedNode.kind) {
    return false;
  }
  if (liveNode.kind === "message" && hydratedNode.kind === "message") {
    if (liveNode.role !== hydratedNode.role) {
      return false;
    }
    return hydratedMessageTextCanReplaceLiveText(liveNode.text, hydratedNode.text);
  }
  return renderCompletenessScore(hydratedNode) >= renderCompletenessScore(liveNode);
}

function hydratedMessageTextCanReplaceLiveText(liveText: string, hydratedText: string): boolean {
  const stableLiveText = stableTimelineText(liveText);
  const stableHydratedText = stableTimelineText(hydratedText);
  return stableHydratedText === stableLiveText || stableHydratedText.startsWith(stableLiveText);
}

function renderCompletenessScore(node: RenderNode): number {
  switch (node.kind) {
    case "message":
      return stableTimelineText(node.text).length;
    case "thought":
      return node.content.map((block) => block.type === "text" ? block.text : JSON.stringify(block)).join("").length;
    case "tool":
      return (
        node.title.length +
        node.locations.length * 25 +
        stableJsonLength(node.content) +
        stableJsonLength(node.rawInput) +
        stableJsonLength(node.rawOutput)
      );
    case "terminal":
      return [
        node.title,
        node.command,
        node.cwd,
        node.output,
        node.stdout,
        node.stderr
      ].reduce((total, value) => total + String(value || "").length, 0);
    case "diff":
      return node.path.length + String(node.oldText || "").length + node.newText.length;
    case "plan":
      return stableJsonLength(node.entries);
    case "approval":
      return node.title.length + node.options.length * 10;
    case "resource":
      return stableJsonLength(node.content);
    case "checkpoint":
      return String(node.checkpointId || "").length + node.label.length;
    default:
      return 0;
  }
}

function stableJsonLength(value: unknown): number {
  if (value === undefined || value === null) {
    return 0;
  }
  try {
    return JSON.stringify(value)?.length || 0;
  } catch {
    return String(value).length;
  }
}

function isPreservableCompletedLiveNode(node: RenderNode, hasHydratedTranscript: boolean): boolean {
  if (!hasHydratedTranscript || node.source !== "acp-live" || isOpenStatus(node.status)) {
    return false;
  }
  switch (node.kind) {
    case "message":
    case "thought":
    case "plan":
    case "tool":
    case "diff":
    case "terminal":
    case "approval":
    case "checkpoint":
    case "resource":
      return true;
    default:
      return false;
  }
}

function hydratedTimelineSegments(nodes: RenderNode[]): RenderNode[][] {
  const segments: RenderNode[][] = [];
  for (const node of nodes) {
    const previous = segments[segments.length - 1];
    if (previous?.length && timelineScopeKey(previous[0]!) === timelineScopeKey(node)) {
      previous.push(node);
    } else {
      segments.push([node]);
    }
  }
  return segments;
}

function orderHydratedSegmentFromCurrent(segment: RenderNode[], orderQueues: Map<string, CurrentTimelineOrder[]>): RenderNode[] {
  return segment
    .map((node, index) => {
      const currentOrder = takeTimelineOrderIndex(node, orderQueues);
      return {
        node: currentOrder?.timelineOrder === undefined ? node : { ...node, timelineOrder: currentOrder.timelineOrder },
        index,
        currentIndex: currentOrder?.index
      };
    })
    .sort((left, right) => {
      const leftMatched = left.currentIndex !== undefined;
      const rightMatched = right.currentIndex !== undefined;
      if (leftMatched && rightMatched) {
        return left.currentIndex! - right.currentIndex! || left.index - right.index;
      }
      if (leftMatched !== rightMatched) {
        return leftMatched ? -1 : 1;
      }
      return left.index - right.index;
    })
    .map((item) => item.node);
}

function takeTimelineOrderIndex(node: RenderNode, orderQueues: Map<string, CurrentTimelineOrder[]>): CurrentTimelineOrder | undefined {
  for (const key of timelineOrderKeys(node)) {
    const queue = orderQueues.get(key);
    const order = queue?.shift();
    if (order) {
      return order;
    }
  }
  return undefined;
}

function timelineOrderKeys(node: RenderNode): string[] {
  const scope = timelineScopeKey(node);
  const keys = [`${scope}:id:${node.id}`];
  if (node.kind === "message") {
    if (node.acpMessageId) {
      keys.push(`${scope}:message-id:${node.acpMessageId}`);
    }
    keys.push(`${scope}:message:${node.role}:${stableTimelineText(node.text)}`);
  } else if (node.kind === "tool") {
    keys.push(`${scope}:tool:${node.toolCallId}`);
    if (node.acpToolCallId) {
      keys.push(`${scope}:tool:${node.acpToolCallId}`);
    }
  } else if (node.kind === "terminal" || node.kind === "diff" || node.kind === "approval") {
    if (node.acpToolCallId) {
      keys.push(`${scope}:${node.kind}:tool:${node.acpToolCallId}`);
    }
  }
  return [...new Set(keys)];
}

function timelineScopeKey(node: RenderNode): string {
  return `${node.taskId}:${node.lane}:${node.turnId || ""}`;
}

function stableTimelineText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
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

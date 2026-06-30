import type { TaskView } from "../crabdb/TaskRepository";
import type { ContentBlock, SessionUpdate } from "../shared/acpTypes";
import { applyRenderPatches, contentToText, reduceSessionUpdate } from "../shared/acpRenderReducers";
import type { MessageNode, RenderNode } from "../shared/renderModel";

export function hydrateTaskView(view: TaskView): RenderNode[] {
  const nodes: RenderNode[] = [];
  const task = view.task;
  let nextTimelineOrder = 0;

  const turns = view.turns.length ? view.turns : fallbackRootTranscriptTurns(view);
  turns.forEach((turnValue, turnIndex) => {
    const turnNodes: RenderNode[] = [];
    const turnWrapper = asRecord(turnValue);
    const turn = asRecord(turnWrapper.turn);
    const turnId = stringField(turn, "turn_id") || stringField(turn, "turnId") || `turn-${turnIndex + 1}`;
    const status = renderStatus(stringField(turn, "status"));
    const messages = arrayField(turnWrapper, "messages");
    const events = arrayField(turnWrapper, "events");
    const turnCompletedAt =
      timestampField(turn, "ended_at", "endedAt", "updated_at", "updatedAt") ||
      timestampField(turnWrapper, "ended_at", "endedAt", "updated_at", "updatedAt");

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

    const checkpoint = stringField(turnWrapper, "checkpoint") || stringField(turn, "after_change") || stringField(turn, "afterChange");
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
  return nodes.map((node, index) => {
    const timelineOrder = start + index + 1;
    return node.timelineOrder === timelineOrder ? node : { ...node, timelineOrder };
  });
}

function fallbackRootTranscriptTurns(view: TaskView): unknown[] {
  if (!view.messages.length && !view.events.length) {
    return [];
  }
  const groups = new Map<string, RootTranscriptTurnGroup>();
  const messageTurnIds = rootMessageTurnIdQueues(view.events);
  let nextIndex = 0;
  for (const message of view.messages) {
    rootTranscriptTurnGroup(groups, message, nextIndex++, messageTurnIds, true).messages.push(message);
  }
  for (const event of view.events) {
    rootTranscriptTurnGroup(groups, event, nextIndex++, messageTurnIds, false).events.push(event);
  }
  const orderedGroups = [...groups.values()].sort((left, right) => left.firstTime - right.firstTime || left.index - right.index);
  const lastGroup = orderedGroups[orderedGroups.length - 1];
  return orderedGroups.map((group) => {
    const checkpoint = group.checkpoint || (group === lastGroup ? view.task.latestCheckpoint : undefined);
    const turn: Record<string, unknown> = {
      turn_id: group.turnId,
      status: view.task.status
    };
    if (group.updatedAt || view.task.updatedAt) {
      turn.updated_at = group.updatedAt || view.task.updatedAt;
    }
    if (checkpoint) {
      turn.after_change = checkpoint;
    }
    return {
      turn,
      messages: group.messages,
      events: group.events,
      ...(checkpoint ? { checkpoint } : {})
    };
  });
}

interface RootTranscriptTurnGroup {
  turnId: string;
  messages: unknown[];
  events: unknown[];
  firstTime: number;
  index: number;
  checkpoint?: string | undefined;
  updatedAt?: string | undefined;
}

function rootTranscriptTurnGroup(
  groups: Map<string, RootTranscriptTurnGroup>,
  value: unknown,
  index: number,
  messageTurnIds: Map<string, RootMessageTurnIdEntry[]>,
  consumeMessageTurnId: boolean
): RootTranscriptTurnGroup {
  const record = asRecord(value);
  const messageId = recordIdField(record, "message_id") || recordIdField(record, "messageId") || recordIdField(record, "id");
  const explicitTurnId = stringField(record, "turn_id") || stringField(record, "turnId");
  const time = rootTranscriptItemTime(record);
  const eventTurnId = messageId
    ? rootMessageTurnId(messageTurnIds, messageId, explicitTurnId, consumeMessageTurnId, time)
    : undefined;
  const turnId = explicitTurnId || eventTurnId || "turn-1";
  let group = groups.get(turnId);
  if (!group) {
    group = {
      turnId,
      messages: [],
      events: [],
      firstTime: Number.MAX_SAFE_INTEGER,
      index
    };
    groups.set(turnId, group);
  }
  if (time < group.firstTime) {
    group.firstTime = time;
  }
  group.checkpoint ||= rootTranscriptItemCheckpoint(record);
  group.updatedAt ||= rootTranscriptItemUpdatedAt(record);
  return group;
}

interface RootMessageTurnIdEntry {
  turnId: string;
  time: number;
  index: number;
}

function rootMessageTurnIdQueues(events: unknown[]): Map<string, RootMessageTurnIdEntry[]> {
  const turnIds = new Map<string, RootMessageTurnIdEntry[]>();
  let index = 0;
  for (const eventValue of orderHydrationEvents(events)) {
    const event = asRecord(eventValue);
    const eventType = stringField(event, "event_type") || stringField(event, "eventType");
    if (eventType !== "message_added") {
      continue;
    }
    const messageId = messageAddedEventMessageId(event);
    const turnId = stringField(event, "turn_id") || stringField(event, "turnId");
    if (messageId && turnId) {
      const queue = turnIds.get(messageId);
      const entry = { turnId, time: rootTranscriptItemTime(event), index: index++ };
      if (queue) {
        queue.push(entry);
      } else {
        turnIds.set(messageId, [entry]);
      }
    }
  }
  return turnIds;
}

function rootMessageTurnId(
  messageTurnIds: Map<string, RootMessageTurnIdEntry[]>,
  messageId: string,
  explicitTurnId: string | undefined,
  consume: boolean,
  messageTime: number
): string | undefined {
  const queue = messageTurnIds.get(messageId);
  if (!queue?.length) {
    return undefined;
  }
  if (!consume) {
    return queue[0]?.turnId;
  }
  if (explicitTurnId) {
    const index = queue.findIndex((entry) => entry.turnId === explicitTurnId);
    if (index >= 0) {
      queue.splice(index, 1);
    }
    return explicitTurnId;
  }
  const index = rootMessageTurnIdEntryIndex(queue, messageTime);
  const [entry] = queue.splice(index, 1);
  return entry?.turnId;
}

function rootMessageTurnIdEntryIndex(queue: RootMessageTurnIdEntry[], messageTime: number): number {
  if (!Number.isFinite(messageTime)) {
    return 0;
  }
  const exactIndex = queue.findIndex((entry) => entry.time === messageTime);
  if (exactIndex >= 0) {
    return exactIndex;
  }
  let bestIndex = 0;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const [index, entry] of queue.entries()) {
    if (!Number.isFinite(entry.time)) {
      continue;
    }
    const distance = Math.abs(entry.time - messageTime);
    if (distance < bestDistance || (distance === bestDistance && entry.index < queue[bestIndex]!.index)) {
      bestDistance = distance;
      bestIndex = index;
    }
  }
  return bestIndex;
}

function rootTranscriptItemTime(record: Record<string, unknown>): number {
  for (const value of [record.created_at, record.createdAt, record.updated_at, record.updatedAt, record.ended_at, record.endedAt]) {
    const timestamp = timestampString(value);
    if (!timestamp) {
      continue;
    }
    const millis = Date.parse(timestamp);
    if (Number.isFinite(millis)) {
      return millis;
    }
  }
  return Number.MAX_SAFE_INTEGER;
}

function rootTranscriptItemCheckpoint(record: Record<string, unknown>): string | undefined {
  return (
    stringField(record, "checkpoint") ||
    stringField(record, "after_change") ||
    stringField(record, "afterChange")
  );
}

function rootTranscriptItemUpdatedAt(record: Record<string, unknown>): string | undefined {
  return (
    timestampString(record.ended_at) ||
    timestampString(record.endedAt) ||
    timestampString(record.updated_at) ||
    timestampString(record.updatedAt) ||
    timestampString(record.created_at) ||
    timestampString(record.createdAt)
  );
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
  const orderedEvents = orderHydrationEvents(events);
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
  let fallbackMessageIndex = messageEntries.length;

  for (const eventValue of orderedEvents) {
    const event = asRecord(eventValue);
    const eventType = stringField(event, "event_type") || stringField(event, "eventType");
    if (eventType === "message_added") {
      const messageId = messageAddedEventMessageId(event);
      const timestamp = timestampField(event, "created_at", "createdAt", "updated_at", "updatedAt");
      const entry = messageId ? takeNextMessageEntry(messagesById, messageId, usedMessageIndexes, timestamp) : undefined;
      if (entry) {
        nodes.push({
          ...entry.node,
          createdAt: entry.node.createdAt || timestamp,
          updatedAt: timestamp || entry.node.updatedAt
        });
        usedMessageIndexes.add(entry.index);
        placedMessages += 1;
      } else {
        const fallback = hydrateMessageAddedEventEntry(
          event,
          fallbackMessageIndex,
          view,
          turnId,
          status,
          duplicateMessageIds
        );
        if (fallback) {
          fallbackMessageIndex += 1;
          nodes.push({
            ...fallback.node,
            createdAt: fallback.node.createdAt || timestamp,
            updatedAt: timestamp || fallback.node.updatedAt
          });
          placedMessages += 1;
        }
      }
      continue;
    }
    nodes = hydrateSessionUpdateEvent(nodes, eventValue, view, turnId, status);
  }

  if (placedMessages === 0) {
    return sortHydratedTurnNodes([
      ...messageEntries.map((entry) => entry.node),
      ...hydrateToolEvents(orderedEvents, view, turnId, status)
    ]);
  }

  const unplacedMessages = messageEntries
    .filter((entry) => !usedMessageIndexes.has(entry.index))
    .map((entry) => entry.node);
  if (unplacedMessages.length) {
    return insertHydratedUnplacedMessages(nodes, sortHydratedTurnNodes(unplacedMessages));
  }
  return nodes;
}

function insertHydratedUnplacedMessages(nodes: RenderNode[], messages: RenderNode[]): RenderNode[] {
  const next = [...nodes];
  for (const message of messages) {
    const insertIndex = next.findIndex((candidate) => shouldInsertHydratedNodeBefore(message, candidate));
    if (insertIndex < 0) {
      next.push(message);
    } else {
      next.splice(insertIndex, 0, message);
    }
  }
  return next;
}

function shouldInsertHydratedNodeBefore(node: RenderNode, candidate: RenderNode): boolean {
  const nodeTime = nodeSortTime(node);
  const candidateTime = nodeSortTime(candidate);
  if (nodeTime !== candidateTime) {
    return nodeTime < candidateTime;
  }
  const phase = nodeSortPhase(node) - nodeSortPhase(candidate);
  return phase < 0;
}

function orderHydrationEvents(events: unknown[]): unknown[] {
  return events
    .map((event, index) => ({ event, index }))
    .sort((left, right) => eventSortTime(left.event) - eventSortTime(right.event) || left.index - right.index)
    .map((item) => item.event);
}

function eventSortTime(eventValue: unknown): number {
  const timestamp = timestampField(asRecord(eventValue), "created_at", "createdAt", "updated_at", "updatedAt");
  if (!timestamp) {
    return Number.MAX_SAFE_INTEGER;
  }
  const millis = Date.parse(timestamp);
  return Number.isFinite(millis) ? millis : Number.MAX_SAFE_INTEGER;
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
  const content = messageContentBlocks(message);
  const createdAt = timestampField(message, "created_at", "createdAt", "updated_at", "updatedAt");
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
      content,
      text: content.map(contentToText).join(""),
      streaming: false
    }
  };
}

function messageAddedEventMessageId(event: Record<string, unknown>): string | undefined {
  const payload = asRecord(event.payload);
  const nestedMessage = asRecord(payload.message);
  return (
    recordIdField(event, "message_id") ||
    recordIdField(event, "messageId") ||
    recordIdField(event, "id") ||
    recordIdField(payload, "message_id") ||
    recordIdField(payload, "messageId") ||
    recordIdField(payload, "id") ||
    recordIdField(nestedMessage, "message_id") ||
    recordIdField(nestedMessage, "messageId") ||
    recordIdField(nestedMessage, "id")
  );
}

function hydrateMessageAddedEventEntry(
  event: Record<string, unknown>,
  messageIndex: number,
  view: TaskView,
  turnId: string,
  status: RenderNode["status"],
  duplicateMessageIds: Set<string>
): HydratedMessageEntry | undefined {
  const message = messageRecordFromMessageAddedEvent(event);
  return message ? hydrateMessageEntry(message, messageIndex, view, turnId, status, duplicateMessageIds) : undefined;
}

function messageRecordFromMessageAddedEvent(event: Record<string, unknown>): Record<string, unknown> | undefined {
  const payload = asRecord(event.payload);
  const nestedMessage = asRecord(payload.message);
  const message: Record<string, unknown> = Object.keys(nestedMessage).length ? { ...nestedMessage } : { ...payload };
  copyFieldIfPresent(message, event, "message_id");
  copyFieldIfPresent(message, event, "messageId");
  copyFieldIfPresent(message, event, "id");
  copyFieldIfPresent(message, payload, "message_id");
  copyFieldIfPresent(message, payload, "messageId");
  copyFieldIfPresent(message, payload, "id");
  copyFieldIfPresent(message, event, "role");
  copyFieldIfPresent(message, event, "created_at");
  copyFieldIfPresent(message, event, "createdAt");
  copyFieldIfPresent(message, event, "updated_at");
  copyFieldIfPresent(message, event, "updatedAt");
  copyFieldIfPresent(message, payload, "role");
  copyFieldIfPresent(message, payload, "created_at");
  copyFieldIfPresent(message, payload, "createdAt");
  copyFieldIfPresent(message, payload, "updated_at");
  copyFieldIfPresent(message, payload, "updatedAt");
  copyFieldIfPresent(message, event, "content");
  copyFieldIfPresent(message, event, "body");
  copyFieldIfPresent(message, event, "text");
  copyFieldIfPresent(message, event, "content_text");
  copyFieldIfPresent(message, event, "contentText");
  copyFieldIfPresent(message, event, "message");
  copyFieldIfPresent(message, payload, "content");
  copyFieldIfPresent(message, payload, "body");
  copyFieldIfPresent(message, payload, "text");
  copyFieldIfPresent(message, payload, "content_text");
  copyFieldIfPresent(message, payload, "contentText");
  return hasRenderableMessageContent(message) ? message : undefined;
}

function copyFieldIfPresent(target: Record<string, unknown>, source: Record<string, unknown>, key: string): void {
  if (target[key] === undefined && source[key] !== undefined) {
    target[key] = source[key];
  }
}

function hasRenderableMessageContent(message: Record<string, unknown>): boolean {
  return contentBlockArray(message.content).length > 0 || messageText(message) !== "";
}

function messageContentBlocks(message: Record<string, unknown>): ContentBlock[] {
  const content = contentBlockArray(message.content);
  if (content.length) {
    return content;
  }
  return [{ type: "text", text: messageText(message) }];
}

function messageText(message: Record<string, unknown>): string {
  return (
    stringField(message, "body") ||
    stringField(message, "content") ||
    stringField(message, "text") ||
    stringField(message, "content_text") ||
    stringField(message, "contentText") ||
    stringField(message, "message") ||
    ""
  );
}

function contentBlockArray(value: unknown): ContentBlock[] {
  if (!Array.isArray(value)) {
    const content = contentBlock(value);
    return content ? [content] : [];
  }
  const content: ContentBlock[] = [];
  for (const item of value) {
    const block = contentBlock(item);
    if (block) {
      content.push(block);
    }
  }
  return content;
}

function contentBlock(value: unknown): ContentBlock | undefined {
  if (typeof value === "string") {
    return { type: "text", text: value };
  }
  const record = asRecord(value);
  if (record.type === "text" && typeof record.text !== "string") {
    const text = stringField(record, "content") || stringField(record, "value");
    if (text !== undefined) {
      return { ...record, type: "text", text };
    }
  }
  if (typeof record.type === "string") {
    return record as ContentBlock;
  }
  const text = stringField(record, "text");
  return text === undefined ? undefined : { type: "text", text };
}

function takeNextMessageEntry(
  messagesById: Map<string, HydratedMessageEntry[]>,
  messageId: string,
  usedMessageIndexes: Set<number>,
  timestamp?: string | undefined
): HydratedMessageEntry | undefined {
  const queue = messagesById.get(messageId);
  if (!queue?.length) {
    return undefined;
  }
  const matchingIndex = messageEntryMatchIndex(queue, usedMessageIndexes, timestamp);
  if (matchingIndex >= 0) {
    const [entry] = queue.splice(matchingIndex, 1);
    return entry;
  }
  while (queue.length) {
    const entry = queue.shift()!;
    if (!usedMessageIndexes.has(entry.index)) {
      return entry;
    }
  }
  return undefined;
}

function messageEntryMatchIndex(
  queue: HydratedMessageEntry[],
  usedMessageIndexes: Set<number>,
  timestamp?: string | undefined
): number {
  if (!timestamp) {
    return -1;
  }
  const target = Date.parse(timestamp);
  if (!Number.isFinite(target)) {
    return -1;
  }
  let bestIndex = -1;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (let index = 0; index < queue.length; index += 1) {
    const entry = queue[index]!;
    if (usedMessageIndexes.has(entry.index)) {
      continue;
    }
    const entryTime = messageEntryTime(entry);
    if (entryTime === undefined) {
      continue;
    }
    const distance = Math.abs(entryTime - target);
    if (distance === 0) {
      return index;
    }
    const bestEntry = bestIndex >= 0 ? queue[bestIndex] : undefined;
    if (
      distance < bestDistance ||
      (distance === bestDistance && (!bestEntry || entry.index < bestEntry.index))
    ) {
      bestDistance = distance;
      bestIndex = index;
    }
  }
  return bestIndex;
}

function messageEntryTime(entry: HydratedMessageEntry): number | undefined {
  const millis = Date.parse(entry.node.createdAt || entry.node.updatedAt || "");
  return Number.isFinite(millis) ? millis : undefined;
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
  const timestamp = timestampField(event, "created_at", "createdAt", "updated_at", "updatedAt");
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
  const updateKind = sessionUpdateKindFromEventType(eventType);
  if (
    isHydratableSessionUpdateEventType(updateKind) ||
    (typeof eventType === "string" && eventType.startsWith("acp_"))
  ) {
    return sessionUpdateFromRecords(updateKind, payload, event);
  }
  if (eventType === "span_started") {
    const attributes = asRecord(payload.attributes);
    return sessionUpdateFromRecords(eventType, attributes, payload, event);
  }
  if (eventType === "span_ended") {
    const result = asRecord(payload.result);
    return sessionUpdateFromRecords(eventType, result, payload, event);
  }
  return undefined;
}

function sessionUpdateKindFromEventType(eventType: string | undefined): string | undefined {
  if (!eventType) {
    return undefined;
  }
  return eventType.startsWith("acp_") ? eventType.slice("acp_".length) : eventType;
}

function isHydratableSessionUpdateEventType(eventType: string | undefined): boolean {
  switch (eventType) {
    case "user_message_chunk":
    case "agent_message_chunk":
    case "agent_thought_chunk":
    case "tool_call":
    case "tool_call_update":
    case "plan":
    case "plan_update":
    case "available_commands_update":
    case "current_mode_update":
    case "config_option_update":
    case "session_info_update":
    case "usage_update":
      return true;
    default:
      return false;
  }
}

function sessionUpdateFromRecords(eventType: string | undefined, ...records: Record<string, unknown>[]): SessionUpdate | undefined {
  for (const record of records) {
    const update = sessionUpdatePayload(record) || inferredSessionUpdatePayload(eventType, record);
    if (update) {
      return update;
    }
  }
  return undefined;
}

function inferredSessionUpdatePayload(eventType: string | undefined, payload: Record<string, unknown>): SessionUpdate | undefined {
  const updateKind = sessionUpdateKindFromEventType(eventType);
  if (isMessageChunkSessionUpdate(updateKind)) {
    return messageChunkSessionUpdateRecord(updateKind, payload);
  }
  if (updateKind === "tool_call" || updateKind === "tool_call_update") {
    const toolCallId = toolCallIdFromRecord(payload);
    return toolCallId ? (toolSessionUpdateRecord(updateKind, payload, toolCallId) as SessionUpdate) : undefined;
  }
  if (updateKind === "span_started") {
    return inferredSpanStartedToolUpdate(payload);
  }
  if (updateKind === "span_ended") {
    return inferredSpanEndedToolUpdate(payload);
  }
  if (updateKind === "plan" || updateKind === "plan_update") {
    const entries = arrayField(payload, "entries");
    return entries.length ? ({ ...payload, sessionUpdate: "plan", entries } as SessionUpdate) : undefined;
  }
  return undefined;
}

function inferredSpanStartedToolUpdate(payload: Record<string, unknown>): SessionUpdate | undefined {
  const attributes = asRecord(payload.attributes);
  const isToolSpan = stringField(payload, "span_type") === "tool" || Boolean(toolCallIdFromRecord(payload) || toolCallIdFromRecord(attributes));
  if (!isToolSpan) {
    return undefined;
  }
  const toolCallId = stringField(payload, "span_id") || toolCallIdFromRecord(payload) || toolCallIdFromRecord(attributes);
  if (!toolCallId) {
    return undefined;
  }
  const update: Record<string, unknown> = {
    ...attributes,
    status: stringField(attributes, "status") || stringField(payload, "status") || "in_progress",
    title: toolTitleFromRecords(attributes, payload),
    kind: stringField(attributes, "kind") || stringField(attributes, "type") || stringField(payload, "kind") || "other"
  };
  return toolSessionUpdateRecord("tool_call", update, toolCallId) as SessionUpdate;
}

function inferredSpanEndedToolUpdate(payload: Record<string, unknown>): SessionUpdate | undefined {
  const result = asRecord(payload.result);
  const toolCallId = toolCallIdFromRecord(result) || toolCallIdFromRecord(payload) || stringField(payload, "span_id");
  if (!toolCallId) {
    return undefined;
  }
  const update: Record<string, unknown> = {
    ...result,
    status: stringField(payload, "status") || stringField(result, "status") || "completed",
    kind: stringField(result, "kind") || stringField(result, "type") || stringField(payload, "kind") || "other"
  };
  const title = toolTitleFromRecords(result, payload);
  if (title) {
    update.title = title;
  }
  if (update.rawOutput === undefined && update.raw_output === undefined && hasOutputFields(result)) {
    update.rawOutput = result;
  }
  return toolSessionUpdateRecord("tool_call_update", update, toolCallId) as SessionUpdate;
}

function sessionUpdatePayload(payload: Record<string, unknown>): SessionUpdate | undefined {
  if (!isSessionUpdate(payload)) {
    return undefined;
  }
  if (isMessageChunkSessionUpdate(payload.sessionUpdate)) {
    return messageChunkSessionUpdateRecord(payload.sessionUpdate, payload);
  }
  if (payload.sessionUpdate === "tool_call" || payload.sessionUpdate === "tool_call_update") {
    const toolCallId = toolCallIdFromRecord(payload);
    if (toolCallId) {
      return toolSessionUpdateRecord(payload.sessionUpdate, payload, toolCallId) as SessionUpdate;
    }
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

type MessageChunkSessionUpdate = "user_message_chunk" | "agent_message_chunk" | "agent_thought_chunk";

function isMessageChunkSessionUpdate(value: unknown): value is MessageChunkSessionUpdate {
  return value === "user_message_chunk" || value === "agent_message_chunk" || value === "agent_thought_chunk";
}

function messageChunkSessionUpdateRecord(
  sessionUpdate: MessageChunkSessionUpdate,
  payload: Record<string, unknown>
): SessionUpdate | undefined {
  const content = messageChunkContentBlock(payload);
  if (!content) {
    return undefined;
  }
  const update: Record<string, unknown> = {
    ...payload,
    sessionUpdate,
    content
  };
  const messageId = messageChunkIdFromRecord(payload);
  if (messageId) {
    update.messageId = messageId;
  }
  return update as SessionUpdate;
}

function messageChunkContentBlock(payload: Record<string, unknown>): ContentBlock | undefined {
  const content = contentBlock(payload.content);
  if (content) {
    return content;
  }
  const text =
    stringField(payload, "text") ||
    stringField(payload, "body") ||
    stringField(payload, "message") ||
    stringField(payload, "content_text") ||
    stringField(payload, "contentText") ||
    stringField(payload, "value");
  return text === undefined ? undefined : { type: "text", text };
}

function messageChunkIdFromRecord(record: Record<string, unknown>): string | undefined {
  return recordIdField(record, "messageId") || recordIdField(record, "message_id");
}

function toolSessionUpdateRecord(
  sessionUpdate: "tool_call" | "tool_call_update",
  payload: Record<string, unknown>,
  toolCallId: string
): Record<string, unknown> {
  const update: Record<string, unknown> = {
    ...payload,
    sessionUpdate,
    toolCallId
  };
  if (sessionUpdate === "tool_call" && !stringField(update, "title")) {
    update.title = stringField(payload, "title") || stringField(payload, "name") || stringField(payload, "tool") || "Tool call";
  }
  copyAliasField(update, payload, "rawInput", "raw_input");
  copyAliasField(update, payload, "rawOutput", "raw_output");
  return update;
}

function toolCallIdFromRecord(record: Record<string, unknown>): string | undefined {
  return (
    recordIdField(record, "toolCallId") ||
    recordIdField(record, "tool_call_id") ||
    recordIdField(record, "tool_id") ||
    recordIdField(record, "id")
  );
}

function copyAliasField(
  target: Record<string, unknown>,
  source: Record<string, unknown>,
  targetKey: string,
  aliasKey: string
): void {
  if (target[targetKey] === undefined && source[aliasKey] !== undefined) {
    target[targetKey] = source[aliasKey];
  }
}

function toolTitleFromRecords(...records: Record<string, unknown>[]): string | undefined {
  for (const record of records) {
    const title =
      stringField(record, "title") ||
      stringField(record, "name") ||
      stringField(record, "tool") ||
      stringField(record, "command");
    if (title) {
      return title;
    }
  }
  return undefined;
}

function hasOutputFields(record: Record<string, unknown>): boolean {
  return [
    "formatted_output",
    "formattedOutput",
    "output",
    "stdout",
    "stdoutPreview",
    "stdout_preview",
    "stderr",
    "stderrPreview",
    "stderr_preview",
    "text"
  ].some((key) => typeof record[key] === "string");
}

function isSessionUpdate(value: unknown): value is SessionUpdate {
  return typeof asRecord(value).sessionUpdate === "string";
}

export function mergeHydratedNodes(hydrated: RenderNode[], current: RenderNode[]): RenderNode[] {
  const orderedHydrated = orderHydratedNodesFromCurrent(hydrated, current);
  const hasHydratedTranscript = hydrated.some((node) => node.turnId);
  const hydratedForEquivalence = filterHydratedNodesForEquivalence(
    orderedHydrated,
    current,
    hasHydratedTranscript
  );
  const matchedHydratedIds = new Set<string>();
  const live = current.filter((node, index) => {
    if (node.source === "crabdb") {
      return false;
    }
    if (hasHydratedTranscript && node.kind === "completion" && node.checkpointPending) {
      return false;
    }
    if (hasHydratedEquivalentNode(node, hydratedForEquivalence, matchedHydratedIds, current, index)) {
      return false;
    }
    return ["pending", "in_progress"].includes(node.status) || isPreservableCompletedLiveNode(node, hasHydratedTranscript);
  });
  return reindexTimelineOrder(ensureUniqueMergedNodeIds(orderTimelineScopesFromCurrent([...hydratedForEquivalence, ...live], current)));
}

interface CurrentTimelineNodeOrder extends CurrentTimelineOrder {
  node: RenderNode;
}

function filterHydratedNodesForEquivalence(
  hydrated: RenderNode[],
  current: RenderNode[],
  hasHydratedTranscript: boolean
): RenderNode[] {
  const orderQueues = new Map<string, CurrentTimelineNodeOrder[]>();
  current.forEach((node, index) => {
    if (!isPreservableCompletedLiveNode(node, hasHydratedTranscript)) {
      return;
    }
    const order = { index, timelineOrder: node.timelineOrder, node };
    for (const key of timelineOrderKeys(node)) {
      const queue = orderQueues.get(key);
      if (queue) {
        queue.push(order);
      } else {
        orderQueues.set(key, [order]);
      }
    }
  });
  const usedCurrentIndexes = new Set<number>();
  return hydrated.filter((hydratedNode) => {
    const liveOrder = takeTimelineOrder(hydratedNode, orderQueues, usedCurrentIndexes);
    if (!liveOrder) {
      return true;
    }
    return hydratedNodeCanReplaceLiveNode(liveOrder.node, hydratedNode, {
      allowMessagePrefixReplacement: canUseMessagePrefixReplacement(liveOrder.node, current, liveOrder.index)
    });
  });
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
  matchedHydratedIds: Set<string>,
  current: RenderNode[],
  currentIndex: number
): boolean {
  for (const hydratedNode of hydrated) {
    if (matchedHydratedIds.has(hydratedNode.id)) {
      continue;
    }
    if (
      hasOverlappingTimelineKey(node, hydratedNode) &&
      hydratedNodeCanReplaceLiveNode(node, hydratedNode, {
        allowMessagePrefixReplacement: canUseMessagePrefixReplacement(node, current, currentIndex)
      })
    ) {
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

interface MessageReplacementOptions {
  allowMessagePrefixReplacement?: boolean | undefined;
}

function hydratedNodeCanReplaceLiveNode(
  liveNode: RenderNode,
  hydratedNode: RenderNode,
  options: MessageReplacementOptions = {}
): boolean {
  if (liveNode.kind !== hydratedNode.kind) {
    return false;
  }
  if (liveNode.kind === "message" && hydratedNode.kind === "message") {
    if (liveNode.role !== hydratedNode.role) {
      return false;
    }
    return hydratedMessageTextCanReplaceLiveText(liveNode.text, hydratedNode.text, options);
  }
  return renderCompletenessScore(hydratedNode) >= renderCompletenessScore(liveNode);
}

function hydratedMessageTextCanReplaceLiveText(
  liveText: string,
  hydratedText: string,
  options: MessageReplacementOptions
): boolean {
  const stableLiveText = stableTimelineText(liveText);
  const stableHydratedText = stableTimelineText(hydratedText);
  if (stableHydratedText === stableLiveText) {
    return true;
  }
  return options.allowMessagePrefixReplacement !== false && stableHydratedText.startsWith(stableLiveText);
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
  const usedCurrentIndexes = new Set<number>();
  return segment
    .map((node, index) => {
      const currentOrder = takeTimelineOrder(node, orderQueues, usedCurrentIndexes);
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

function takeTimelineOrder<T extends CurrentTimelineOrder>(
  node: RenderNode,
  orderQueues: Map<string, T[]>,
  usedCurrentIndexes: Set<number>
): T | undefined {
  for (const key of timelineOrderKeys(node)) {
    const queue = orderQueues.get(key);
    while (queue?.length) {
      const order = queue.shift()!;
      if (usedCurrentIndexes.has(order.index)) {
        continue;
      }
      usedCurrentIndexes.add(order.index);
      return order;
    }
  }
  return undefined;
}

function timelineOrderKeys(node: RenderNode): string[] {
  const scope = timelineScopeKey(node);
  const keys = [`${scope}:id:${node.id}`];
  if (node.kind === "message") {
    const text = stableTimelineText(node.text);
    if (node.acpMessageId) {
      keys.push(`${scope}:message-id-text:${node.acpMessageId}:${node.role}:${text}`);
      keys.push(`${scope}:message-id:${node.acpMessageId}`);
    }
    keys.push(`${scope}:message:${node.role}:${text}`);
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

function canUseMessagePrefixReplacement(liveNode: RenderNode, current: RenderNode[], currentIndex: number): boolean {
  if (liveNode.kind !== "message" || !liveNode.acpMessageId) {
    return true;
  }
  return !current.some((candidate, index) =>
    index > currentIndex &&
    candidate.kind === "message" &&
    candidate.source === liveNode.source &&
    candidate.role === liveNode.role &&
    candidate.acpMessageId === liveNode.acpMessageId &&
    timelineScopeKey(candidate) === timelineScopeKey(liveNode)
  );
}

function stableTimelineText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function timestampField(record: Record<string, unknown>, ...keys: string[]): string | undefined {
  for (const key of keys) {
    const timestamp = timestampString(record[key]);
    if (timestamp) {
      return timestamp;
    }
  }
  return undefined;
}

function renderStatus(status: string | undefined): RenderNode["status"] {
  switch (status) {
    case "failed":
      return "failed";
    case "cancelled":
      return "cancelled";
    case "pending":
      return "pending";
    case "active":
    case "in-progress":
    case "in_progress":
    case "running":
      return "in_progress";
    default:
      return "completed";
  }
}

function timestampString(value: unknown): string | undefined {
  if (typeof value === "number") {
    return unixTimestampString(value);
  }
  if (typeof value !== "string") {
    return undefined;
  }
  const trimmed = value.trim();
  if (!trimmed) {
    return undefined;
  }
  if (/^[+-]?\d+(?:\.\d+)?$/.test(trimmed)) {
    return unixTimestampString(Number(trimmed));
  }
  return timestampFromMillis(Date.parse(trimmed));
}

function unixTimestampString(value: number): string | undefined {
  if (!Number.isFinite(value)) {
    return undefined;
  }
  const millis = Math.abs(value) > 10_000_000_000 ? value : value * 1000;
  return timestampFromMillis(millis);
}

function timestampFromMillis(millis: number): string | undefined {
  if (!Number.isFinite(millis)) {
    return undefined;
  }
  const date = new Date(millis);
  return Number.isFinite(date.getTime()) ? date.toISOString() : undefined;
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

import type { ContentBlock } from "../shared/acpTypes";
import type { RenderNode, RenderPatch } from "../shared/renderModel";

export interface RenderPatchChanges {
  changedNodeIds: Set<string>;
  addedNodeIds: Set<string>;
  removedNodeIds: Set<string>;
}

export function parseRenderRevision(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0 ? value : undefined;
}

export function parseBaseRenderRevision(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}

export function shouldAcceptRenderRevision(revision: number | undefined, latestRevision: number): boolean {
  return revision === undefined ? latestRevision === 0 : revision > latestRevision;
}

export type RenderPatchBatchDecision = "apply" | "drop" | "resync";

export function renderPatchBatchDecision(
  baseRevision: number | undefined,
  revision: number | undefined,
  latestRevision: number
): RenderPatchBatchDecision {
  if (baseRevision === undefined || revision === undefined || revision <= latestRevision) {
    return "drop";
  }
  return baseRevision === latestRevision ? "apply" : "resync";
}

export function applyRenderPatchesLocally(nodes: RenderNode[], patches: RenderPatch[]): RenderNode[] {
  return applyRenderPatchesLocallyAndCollect(nodes, patches).nodes;
}

export function changedRenderNodes(beforeById: Map<string, RenderNode>, nextNodes: RenderNode[]): RenderPatchChanges {
  const nextIds = new Set(nextNodes.map((node) => node.id));
  const changedNodeIds = new Set<string>();
  const addedNodeIds = new Set<string>();
  for (const node of nextNodes) {
    const previous = beforeById.get(node.id);
    if (!previous) {
      addedNodeIds.add(node.id);
      changedNodeIds.add(node.id);
    } else if (previous !== node) {
      changedNodeIds.add(node.id);
    }
  }
  const removedNodeIds = new Set([...beforeById.keys()].filter((id) => !nextIds.has(id)));
  return { changedNodeIds, addedNodeIds, removedNodeIds };
}

export function changedRenderNodesFromPatches(nodes: RenderNode[], patches: RenderPatch[]): RenderPatchChanges {
  return applyRenderPatchesLocallyAndCollect(nodes, patches).changes;
}

function applyRenderPatchesLocallyAndCollect(
  nodes: RenderNode[],
  patches: RenderPatch[]
): { nodes: RenderNode[]; changes: RenderPatchChanges } {
  let next = [...nodes];
  const knownIds = new Set(next.map((node) => node.id));
  const changedNodeIds = new Set<string>();
  const addedNodeIds = new Set<string>();
  const removedNodeIds = new Set<string>();
  const normalizedIdsByRawId = new Map<string, string[]>();

  for (const patch of patches) {
    if ((patch.type === "append" || patch.type === "replace" || patch.type === "upsert") && patch.node) {
      const node = normalizePatchNodeForLocalTimeline(next, patch.node, patch.type);
      if (!node) {
        continue;
      }
      const id = node.id;
      const index = patch.type === "append" ? -1 : next.findIndex((candidate) => candidate.id === id);
      if (index >= 0) {
        next[index] = mergeLocalPatchNode(next[index]!, node, patch.type);
      } else {
        next.push(node);
      }
      if (!knownIds.has(id)) {
        addedNodeIds.add(id);
      }
      knownIds.add(id);
      removedNodeIds.delete(id);
      changedNodeIds.add(id);
      trackNormalizedPatchId(normalizedIdsByRawId, patch.node.id, id);
      const movedFinalMarkers = moveLocalTurnFinalMarkersAfterNode(next, node);
      next = movedFinalMarkers.nodes;
      for (const movedId of movedFinalMarkers.changedNodeIds) {
        changedNodeIds.add(movedId);
      }
      continue;
    }
    if (patch.type === "remove" && patch.id) {
      const removeId = consumeNormalizedPatchId(normalizedIdsByRawId, patch.id) || patch.id;
      const beforeLength = next.length;
      next = next.filter((node) => node.id !== removeId);
      if (next.length === beforeLength) {
        continue;
      }
      if (addedNodeIds.has(removeId)) {
        addedNodeIds.delete(removeId);
        changedNodeIds.delete(removeId);
      } else if (knownIds.has(removeId)) {
        removedNodeIds.add(removeId);
      }
      knownIds.delete(removeId);
    }
  }

  return { nodes: next, changes: { changedNodeIds, addedNodeIds, removedNodeIds } };
}

function moveLocalTurnFinalMarkersAfterNode(
  nodes: RenderNode[],
  node: RenderNode
): { nodes: RenderNode[]; changedNodeIds: string[] } {
  if (!node.turnId || isTurnFinalMarker(node)) {
    return { nodes, changedNodeIds: [] };
  }
  const nodeIndex = nodes.findIndex((candidate) => candidate.id === node.id);
  if (nodeIndex < 0) {
    return { nodes, changedNodeIds: [] };
  }
  let nextTimelineOrder = maxLocalTimelineOrder(nodes);
  const kept: RenderNode[] = [];
  const moved: RenderNode[] = [];
  const changedNodeIds: string[] = [];
  nodes.forEach((candidate, index) => {
    if (
      candidate.id !== node.id &&
      isTurnFinalMarker(candidate) &&
      sameTurnFinalMarkerScope(candidate, node) &&
      shouldMoveTurnFinalMarkerAfterNode(candidate, index, node, nodeIndex)
    ) {
      nextTimelineOrder += 1;
      const movedNode = {
        ...candidate,
        timelineOrder: nextTimelineOrder
      } as RenderNode;
      moved.push(movedNode);
      changedNodeIds.push(movedNode.id);
      return;
    }
    kept.push(candidate);
  });
  return changedNodeIds.length
    ? { nodes: [...kept, ...moved], changedNodeIds }
    : { nodes, changedNodeIds };
}

function shouldMoveTurnFinalMarkerAfterNode(
  marker: RenderNode,
  markerIndex: number,
  node: RenderNode,
  nodeIndex: number
): boolean {
  if (markerIndex < nodeIndex) {
    return true;
  }
  const markerOrder = Number.isFinite(marker.timelineOrder) ? marker.timelineOrder! : markerIndex + 1;
  const nodeOrder = Number.isFinite(node.timelineOrder) ? node.timelineOrder! : nodeIndex + 1;
  return markerOrder <= nodeOrder;
}

function maxLocalTimelineOrder(nodes: RenderNode[]): number {
  return nodes.reduce((max, node, index) => Math.max(max, node.timelineOrder ?? index + 1), 0);
}

function isTurnFinalMarker(node: RenderNode): boolean {
  return node.kind === "completion" || node.kind === "checkpoint";
}

function sameTurnFinalMarkerScope(marker: RenderNode, node: RenderNode): boolean {
  return (
    marker.taskId === node.taskId &&
    marker.lane === node.lane &&
    marker.turnId !== undefined &&
    marker.turnId === node.turnId
  );
}

function mergeLocalPatchNode(existing: RenderNode, incoming: RenderNode, patchType: RenderPatch["type"]): RenderNode {
  if (patchType !== "upsert" || !isStreamNode(existing) || !isStreamNode(incoming) || !sameTimelineScope(existing, incoming)) {
    return incoming;
  }
  return mergeLocalStreamNode(existing, incoming);
}

function trackNormalizedPatchId(target: Map<string, string[]>, rawId: string, normalizedId: string): void {
  if (rawId === normalizedId) {
    return;
  }
  const ids = target.get(rawId) || [];
  ids.push(normalizedId);
  target.set(rawId, ids);
}

function consumeNormalizedPatchId(target: Map<string, string[]>, rawId: string): string | undefined {
  const ids = target.get(rawId);
  const normalizedId = ids?.pop();
  if (!ids?.length) {
    target.delete(rawId);
  }
  return normalizedId;
}

function normalizePatchNodeForLocalTimeline(
  nodes: RenderNode[],
  node: RenderNode,
  patchType: RenderPatch["type"]
): RenderNode | undefined {
  if (patchType === "append") {
    return normalizeAppendedLocalNode(nodes, node);
  }
  if (patchType === "upsert" && isStreamNode(node)) {
    const existingIndex = nodes.findIndex((candidate) => candidate.id === node.id);
    const existing = existingIndex >= 0 ? nodes[existingIndex] : undefined;
    if (existing && sameTimelineScope(existing, node)) {
      if (!hasNonStreamTimelineBoundaryAfter(nodes, existingIndex, node)) {
        return node;
      }
      const continuation = streamContinuationAfterBoundary(existing, node);
      if (!continuation) {
        return undefined;
      }
      const appendableContinuation = latestNodeInSameTurn(nodes, continuation);
      if (appendableContinuation && appendableContinuation !== existing && canAppendStreamNode(appendableContinuation, continuation)) {
        return streamNodeWithTimelineIdentity(continuation, appendableContinuation);
      }
      return {
        ...continuation,
        id: nextStreamNodeId(nodes, continuation)
      };
    }
    const appendable = latestNodeInSameTurn(nodes, node);
    if (appendable && canAppendStreamNode(appendable, node)) {
      return streamNodeWithTimelineIdentity(node, appendable);
    }
    if (existingIndex < 0) {
      return node;
    }
    return {
      ...node,
      id: nextStreamNodeId(nodes, node)
    };
  }
  return normalizeCollidingLocalNode(nodes, node);
}

function streamNodeWithTimelineIdentity(node: StreamNode, existing: StreamNode): StreamNode {
  return {
    ...node,
    id: existing.id,
    createdAt: existing.createdAt,
    timelineOrder: existing.timelineOrder,
    acpMessageId: existing.acpMessageId
  };
}

function normalizeAppendedLocalNode(nodes: RenderNode[], node: RenderNode): RenderNode {
  if (!nodes.some((candidate) => candidate.id === node.id)) {
    return node;
  }
  if (isStreamNode(node)) {
    return {
      ...node,
      id: nextStreamNodeId(nodes, node)
    };
  }
  return {
    ...node,
    id: nextScopedCollisionNodeId(nodes, node)
  };
}

type StreamNode = Extract<RenderNode, { kind: "message" | "thought" }>;

function isStreamNode(node: RenderNode): node is StreamNode {
  return node.kind === "message" || node.kind === "thought";
}

function canAppendStreamNode(existing: RenderNode, incoming: StreamNode): existing is StreamNode {
  if (incoming.kind === "message") {
    return (
      existing.kind === "message" &&
      existing.role === incoming.role &&
      existing.acpMessageId === incoming.acpMessageId
    );
  }
  return existing.kind === "thought" && existing.acpMessageId === incoming.acpMessageId;
}

function mergeLocalStreamNode(existing: StreamNode, incoming: StreamNode): StreamNode {
  if (existing.kind !== incoming.kind) {
    return incoming;
  }
  const content = mergeStreamContentBlocks(existing.content, incoming.content);
  if (incoming.kind === "message") {
    const existingMessage = existing as Extract<StreamNode, { kind: "message" }>;
    return {
      ...incoming,
      createdAt: existingMessage.createdAt,
      timelineOrder: existingMessage.timelineOrder,
      acpMessageId: existingMessage.acpMessageId,
      content,
      text: contentBlocksToText(content),
      streaming: mergedLocalMessageStreaming(existingMessage, incoming)
    };
  }
  return {
    ...incoming,
    createdAt: existing.createdAt,
    timelineOrder: existing.timelineOrder,
    acpMessageId: existing.acpMessageId,
    content
  };
}

function mergedLocalMessageStreaming(
  existing: Extract<StreamNode, { kind: "message" }>,
  incoming: Extract<StreamNode, { kind: "message" }>
): boolean {
  return isActiveRenderStatus(incoming.status) && (existing.streaming || incoming.streaming);
}

function isTextOnlyStreamNode(node: StreamNode): boolean {
  return node.content.length > 0 && node.content.every((block) => block.type === "text");
}

function streamContinuationAfterBoundary(existing: RenderNode, incoming: StreamNode): StreamNode | undefined {
  if (!isStreamNode(existing) || existing.kind !== incoming.kind) {
    return incoming;
  }
  if (isTextOnlyStreamNode(existing) && isTextOnlyStreamNode(incoming)) {
    const existingText = contentBlocksToText(existing.content);
    const incomingText = contentBlocksToText(incoming.content);
    if (!incomingText.startsWith(existingText)) {
      return incoming;
    }
    const suffix = incomingText.slice(existingText.length);
    if (!suffix) {
      return undefined;
    }
    return streamNodeWithContent(incoming, [{ type: "text", text: suffix }]);
  }

  if (!contentBlocksStartWith(incoming.content, existing.content)) {
    return incoming;
  }
  const suffix = incoming.content.slice(existing.content.length);
  if (!suffix.length) {
    return undefined;
  }
  return streamNodeWithContent(incoming, suffix);
}

function mergeStreamContentBlocks(previous: ContentBlock[], incoming: ContentBlock[]): ContentBlock[] {
  if (!isTextOnlyContentBlocks(previous) || !isTextOnlyContentBlocks(incoming)) {
    if (contentBlocksStartWith(incoming, previous)) {
      return mergeAdjacentTextContentBlocks(incoming);
    }
    if (contentBlocksStartWith(previous, incoming)) {
      return mergeAdjacentTextContentBlocks(previous);
    }
    return mergeAdjacentTextContentBlocks([...previous, ...incoming]);
  }
  const previousText = contentBlocksToText(previous);
  const incomingText = contentBlocksToText(incoming);
  if (incomingText.startsWith(previousText)) {
    return mergeAdjacentTextContentBlocks(incoming);
  }
  if (previousText.startsWith(incomingText)) {
    return mergeAdjacentTextContentBlocks(previous);
  }
  return mergeAdjacentTextContentBlocks([...previous, ...incoming]);
}

function isTextOnlyContentBlocks(blocks: ContentBlock[]): boolean {
  return blocks.length > 0 && blocks.every((block) => block.type === "text");
}

function contentBlocksStartWith(blocks: ContentBlock[], prefix: ContentBlock[]): boolean {
  if (prefix.length > blocks.length) {
    return false;
  }
  return prefix.every((block, index) => sameContentBlock(block, blocks[index]!));
}

function sameContentBlock(left: ContentBlock, right: ContentBlock): boolean {
  return stableJson(left) === stableJson(right);
}

function streamNodeWithContent(node: StreamNode, content: ContentBlock[]): StreamNode {
  return node.kind === "message"
    ? { ...node, content, text: contentBlocksToText(content) }
    : { ...node, content };
}

function mergeAdjacentTextContentBlocks(blocks: ContentBlock[]): ContentBlock[] {
  const merged: ContentBlock[] = [];
  for (const block of blocks) {
    const previous = merged[merged.length - 1];
    if (previous?.type === "text" && block.type === "text") {
      merged[merged.length - 1] = {
        ...previous,
        text: `${previous.text}${block.text}`
      };
      continue;
    }
    merged.push(block);
  }
  return merged;
}

function contentBlocksToText(blocks: ContentBlock[]): string {
  return blocks.map((block) => block.type === "text" ? block.text : `[${block.type || "content"}]`).join("");
}

function stableJson(value: unknown): string {
  return JSON.stringify(stableJsonValue(value));
}

function stableJsonValue(value: unknown): unknown {
  if (value === null || typeof value !== "object") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => stableJsonValue(item));
  }
  const record = value as Record<string, unknown>;
  const sorted: Record<string, unknown> = {};
  for (const key of Object.keys(record).sort()) {
    sorted[key] = stableJsonValue(record[key]);
  }
  return sorted;
}

function latestNodeInSameTurn(nodes: RenderNode[], node: RenderNode): RenderNode | undefined {
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    const candidate = nodes[index];
    if (candidate && sameTimelineScope(candidate, node)) {
      return candidate;
    }
  }
  return undefined;
}

function hasNonStreamTimelineBoundaryAfter(nodes: RenderNode[], index: number, node: RenderNode): boolean {
  for (let nextIndex = index + 1; nextIndex < nodes.length; nextIndex += 1) {
    const candidate = nodes[nextIndex];
    if (candidate && sameTimelineScope(candidate, node) && isFinalNonStreamBoundary(candidate)) {
      return true;
    }
  }
  return false;
}

function isFinalNonStreamBoundary(node: RenderNode): boolean {
  if (isStreamNode(node)) {
    return false;
  }
  if (node.kind === "tool") {
    return isFinalRenderStatus(node.status) || isFinalRenderStatus(node.toolStatus);
  }
  if (node.kind === "terminal") {
    return isFinalRenderStatus(node.status) || isFinalRenderStatus(node.terminalStatus);
  }
  return isFinalRenderStatus(node.status);
}

function normalizeCollidingLocalNode(nodeList: RenderNode[], node: RenderNode): RenderNode {
  const existingIndex = nodeList.findIndex((candidate) => candidate.id === node.id);
  const existingById = existingIndex >= 0 ? nodeList[existingIndex] : undefined;
  if (!existingById || canUpdateExistingNodeById(nodeList, existingIndex, node)) {
    return node;
  }
  const semanticMatch = findSameSemanticNode(nodeList, node);
  if (semanticMatch) {
    return {
      ...node,
      id: semanticMatch.id,
      createdAt: semanticMatch.createdAt,
      timelineOrder: semanticMatch.timelineOrder
    };
  }
  return {
    ...node,
    id: nextScopedCollisionNodeId(nodeList, node)
  };
}

function canUpdateExistingNodeById(nodes: RenderNode[], existingIndex: number, incoming: RenderNode): boolean {
  const existing = nodes[existingIndex];
  if (!existing) {
    return false;
  }
  if (existing.kind !== incoming.kind) {
    return false;
  }
  if (sameTimelineScope(existing, incoming)) {
    return (
      !hasExternalTimelineBoundaryAfter(nodes, existingIndex, incoming) ||
      canUpdateAfterExternalBoundary(existing, incoming)
    );
  }
  const existingKey = semanticNodeKey(existing);
  return existingKey !== undefined && existingKey === semanticNodeKey(incoming);
}

function hasExternalTimelineBoundaryAfter(nodes: RenderNode[], index: number, node: RenderNode): boolean {
  for (let nextIndex = index + 1; nextIndex < nodes.length; nextIndex += 1) {
    const candidate = nodes[nextIndex];
    if (!candidate || !sameTimelineScope(candidate, node) || isSameToolFamilyNode(candidate, node)) {
      continue;
    }
    return true;
  }
  return false;
}

function isSameToolFamilyNode(candidate: RenderNode, node: RenderNode): boolean {
  const toolCallId = node.kind === "tool" ? node.toolCallId : node.acpToolCallId;
  if (!toolCallId) {
    return false;
  }
  return candidate.acpToolCallId === toolCallId && (candidate.kind === "terminal" || candidate.kind === "diff");
}

function canUpdateAfterExternalBoundary(existing: RenderNode, incoming: RenderNode): boolean {
  if (isActiveRenderStatus(existing.status)) {
    return true;
  }
  if (existing.kind === "tool" && incoming.kind === "tool") {
    return rawSessionUpdate(incoming) === "tool_call_update";
  }
  if (existing.kind === "terminal" && incoming.kind === "terminal") {
    return isActiveRenderStatus(existing.terminalStatus);
  }
  return incoming.kind !== "tool" && incoming.kind !== "terminal" && incoming.kind !== "diff";
}

function isActiveRenderStatus(status: string | undefined): boolean {
  return status === "pending" || status === "in_progress";
}

function isFinalRenderStatus(status: string | undefined): boolean {
  return status === "completed" || status === "failed" || status === "cancelled";
}

function rawSessionUpdate(node: RenderNode): string | undefined {
  const raw = node.raw;
  if (!raw || typeof raw !== "object") {
    return undefined;
  }
  const value = (raw as { sessionUpdate?: unknown }).sessionUpdate;
  return typeof value === "string" ? value : undefined;
}

function sameTimelineScope(left: RenderNode, right: RenderNode): boolean {
  return (
    left.taskId === right.taskId &&
    left.lane === right.lane &&
    left.turnId === right.turnId &&
    left.acpSessionId === right.acpSessionId &&
    left.source === right.source
  );
}

function findSameSemanticNode(nodes: RenderNode[], node: RenderNode): RenderNode | undefined {
  const key = semanticNodeKey(node);
  if (!key) {
    return undefined;
  }
  return nodes.find((candidate) => candidate.id !== node.id && semanticNodeKey(candidate) === key);
}

function semanticNodeKey(node: RenderNode): string | undefined {
  const scope = `${node.taskId}\u0000${node.lane}\u0000${node.turnId || ""}\u0000${node.acpSessionId || ""}\u0000${node.source}`;
  switch (node.kind) {
    case "tool":
      return `${scope}\u0000tool\u0000${node.acpToolCallId || node.toolCallId}`;
    case "terminal":
      return `${scope}\u0000terminal\u0000${node.acpToolCallId || ""}\u0000${node.terminalId}`;
    case "diff":
      return `${scope}\u0000diff\u0000${node.acpToolCallId || ""}\u0000${node.path}`;
    case "approval":
      return `${scope}\u0000approval\u0000${node.requestId}`;
    case "plan":
      return `${scope}\u0000plan`;
    case "resource":
      return `${scope}\u0000resource\u0000${node.id}`;
    default:
      return undefined;
  }
}

function nextScopedCollisionNodeId(nodes: RenderNode[], node: RenderNode): string {
  const used = new Set(nodes.map((candidate) => candidate.id));
  const suffix = scopedCollisionSuffix(node);
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

function scopedCollisionSuffix(node: RenderNode): string {
  return sanitizeIdSegment(
    [node.turnId, node.acpSessionId, node.source].filter(Boolean).join(":") ||
      `${node.taskId}:${node.lane}`
  );
}

function nextStreamNodeId(nodes: RenderNode[], node: StreamNode): string {
  const base = node.kind === "message"
    ? `message:${node.role}:${node.acpMessageId || "anonymous"}`
    : `thought:${node.acpMessageId || "anonymous"}`;
  const used = new Set(nodes.map((candidate) => candidate.id));
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

function sanitizeIdSegment(value: string): string {
  return value
    .trim()
    .replace(/[^A-Za-z0-9_.:-]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export function hasTimelineStructuralChange(
  beforeNodes: RenderNode[],
  nextNodes: RenderNode[],
  changes: RenderPatchChanges
): boolean {
  if (changes.addedNodeIds.size || changes.removedNodeIds.size) {
    return true;
  }
  const beforeById = new Map(beforeNodes.map((node) => [node.id, node]));
  const nextById = new Map(nextNodes.map((node) => [node.id, node]));
  for (const id of changes.changedNodeIds) {
    const before = beforeById.get(id);
    const next = nextById.get(id);
    if (!before || !next) {
      continue;
    }
    if (timelineStructureKey(before) !== timelineStructureKey(next)) {
      return true;
    }
  }
  return false;
}

function timelineStructureKey(node: RenderNode): string {
  return JSON.stringify({
    kind: node.kind,
    taskId: node.taskId,
    lane: node.lane,
    turnId: node.turnId,
    acpSessionId: node.acpSessionId,
    status: node.status,
    sortKey: timelineSortKey(node)
  });
}

function timelineSortKey(node: RenderNode): number | string {
  if (Number.isFinite(node.timelineOrder)) {
    return node.timelineOrder!;
  }
  for (const value of [node.createdAt, node.updatedAt]) {
    if (!value) {
      continue;
    }
    const time = Date.parse(value);
    if (Number.isFinite(time)) {
      return time;
    }
  }
  return "";
}

export function isStreamingTextPatchPayload(patch: RenderPatch): boolean {
  const node = patch.node;
  if (patch.type !== "upsert" || !node || node.source !== "acp-live") {
    return false;
  }
  if (node.status !== "pending" && node.status !== "in_progress") {
    return false;
  }
  if (node.kind !== "message" && node.kind !== "thought") {
    return false;
  }
  return node.content.length > 0 && node.content.every((block) => block.type === "text");
}

export function isLiveNodePatchPayload(patch: RenderPatch): boolean {
  const node = patch.node;
  if ((patch.type !== "upsert" && patch.type !== "replace") || !node || node.source !== "acp-live") {
    return false;
  }
  if (node.status !== "pending" && node.status !== "in_progress") {
    return false;
  }
  if (node.kind === "message" || node.kind === "thought") {
    return isStreamingTextPatchPayload(patch);
  }
  return node.kind === "plan" || node.kind === "tool" || node.kind === "diff" || node.kind === "terminal";
}

export function isHydratableNodePatchPayload(patch: RenderPatch): boolean {
  const node = patch.node;
  if ((patch.type !== "upsert" && patch.type !== "replace") || !node) {
    return false;
  }
  if (isLiveNodePatchPayload(patch)) {
    return true;
  }
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
    node.kind === "resource"
  );
}

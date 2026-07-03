import type { ContentBlock } from "../shared/acpTypes";
import type { RenderNode, RenderPatch } from "../shared/renderModel";
import { textContentValue } from "./contentTextModel";

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

  for (let patchIndex = 0; patchIndex < patches.length; patchIndex += 1) {
    const patch = patches[patchIndex]!;
    if ((patch.type === "append" || patch.type === "replace" || patch.type === "upsert") && patch.node) {
      const node = normalizePatchNodeForLocalTimeline(next, patch.node, patch.type, laterPatchNodes(patches, patchIndex));
      if (!node) {
        continue;
      }
      const id = node.id;
      const index = patch.type === "append" ? -1 : next.findIndex((candidate) => candidate.id === id);
      let appliedNode: RenderNode;
      if (index >= 0) {
        next[index] = mergeLocalPatchNode(next[index]!, node, patch.type);
        appliedNode = next[index]!;
      } else {
        next.push(node);
        appliedNode = node;
      }
      if (!knownIds.has(id)) {
        addedNodeIds.add(id);
      }
      knownIds.add(id);
      removedNodeIds.delete(id);
      changedNodeIds.add(id);
      trackNormalizedPatchId(normalizedIdsByRawId, patch.node.id, id);
      if (appliedNode.kind === "tool") {
        const synced = syncLocalExpandedToolChildNodes(next, appliedNode);
        next = synced.nodes;
        for (const syncedId of synced.changedNodeIds) {
          changedNodeIds.add(syncedId);
        }
      }
      const movedFinalMarkers = moveLocalTurnFinalMarkersAfterNode(next, appliedNode);
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

type LocalExpandedToolChildNode = Extract<RenderNode, { kind: "diff" | "terminal" }>;

function syncLocalExpandedToolChildNodes(
  nodes: RenderNode[],
  tool: Extract<RenderNode, { kind: "tool" }>
): { nodes: RenderNode[]; changedNodeIds: string[] } {
  let changed = false;
  const changedNodeIds: string[] = [];
  const next = nodes.map((node) => {
    if (!isLocalExpandedToolChildNode(node) || !localChildBelongsToToolScope(node, tool)) {
      return node;
    }
    const updated = syncLocalExpandedToolChildNode(node, tool);
    if (updated === node) {
      return node;
    }
    changed = true;
    changedNodeIds.push(node.id);
    return updated;
  });
  return changed ? { nodes: next, changedNodeIds } : { nodes, changedNodeIds: [] };
}

function isLocalExpandedToolChildNode(node: RenderNode): node is LocalExpandedToolChildNode {
  return node.kind === "diff" || node.kind === "terminal";
}

function syncLocalExpandedToolChildNode(
  node: LocalExpandedToolChildNode,
  tool: Extract<RenderNode, { kind: "tool" }>
): LocalExpandedToolChildNode {
  if (node.kind === "terminal") {
    const terminalStatus = syncTerminalStatusFromTool(node.terminalStatus, tool.toolStatus);
    if (node.status === tool.status && node.terminalStatus === terminalStatus) {
      return node;
    }
    return {
      ...node,
      status: tool.status,
      terminalStatus,
      updatedAt: tool.updatedAt
    };
  }
  if (node.status === tool.status) {
    return node;
  }
  return {
    ...node,
    status: tool.status,
    updatedAt: tool.updatedAt
  };
}

function localChildBelongsToToolScope(
  child: LocalExpandedToolChildNode,
  tool: Extract<RenderNode, { kind: "tool" }>
): boolean {
  if (!localChildMatchesToolCallId(child, tool) || !sameTimelineScope(child, tool)) {
    return false;
  }
  const suffix = scopedToolNodeSuffix(tool);
  return suffix ? child.id.endsWith(`:${suffix}`) : true;
}

function localChildMatchesToolCallId(
  child: LocalExpandedToolChildNode,
  tool: Extract<RenderNode, { kind: "tool" }>
): boolean {
  return child.acpToolCallId === tool.toolCallId || child.acpToolCallId === tool.acpToolCallId;
}

function scopedToolNodeSuffix(tool: Extract<RenderNode, { kind: "tool" }>): string | undefined {
  const base = `tool:${tool.toolCallId}:`;
  return tool.id.startsWith(base) ? tool.id.slice(base.length) : undefined;
}

function syncTerminalStatusFromTool(current: string | undefined, next: string | undefined): string | undefined {
  if (!next || current === next || !shouldAdoptToolTerminalStatus(current)) {
    return current;
  }
  return next;
}

function shouldAdoptToolTerminalStatus(current: string | undefined): boolean {
  if (!current) {
    return true;
  }
  return isToolLikeStatus(current);
}

function isToolLikeStatus(status: string | undefined): boolean {
  switch (normalizeStatus(status)) {
    case "pending":
    case "in-progress":
    case "running":
    case "completed":
    case "succeeded":
    case "success":
    case "passed":
    case "failed":
    case "error":
    case "cancelled":
    case "canceled":
      return true;
    default:
      return false;
  }
}

function normalizeStatus(status: string | undefined): string {
  return String(status || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
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
    marker.turnId === node.turnId &&
    compatibleFinalMarkerScopeValue(marker.acpSessionId, node.acpSessionId) &&
    compatibleFinalMarkerScopeValue(marker.provider, node.provider)
  );
}

function compatibleFinalMarkerScopeValue(left: string | undefined, right: string | undefined): boolean {
  return left === right || left === undefined || right === undefined;
}

function mergeLocalPatchNode(existing: RenderNode, incoming: RenderNode, patchType: RenderPatch["type"]): RenderNode {
  if (
    patchType === "upsert" &&
    existing.kind === "terminal" &&
    incoming.kind === "terminal" &&
    sameTimelineScope(existing, incoming) &&
    existing.terminalId === incoming.terminalId
  ) {
    return mergeLocalTerminalNode(existing, incoming);
  }
  if (
    patchType !== "upsert" ||
    !isStreamNode(existing) ||
    !isStreamNode(incoming) ||
    !compatibleStreamTimelineScope(existing, incoming)
  ) {
    return incoming;
  }
  return mergeLocalStreamNode(existing, incoming);
}

function mergeLocalTerminalNode(
  existing: Extract<RenderNode, { kind: "terminal" }>,
  incoming: Extract<RenderNode, { kind: "terminal" }>
): Extract<RenderNode, { kind: "terminal" }> {
  const preserveFinalStatus = isFinalRenderStatus(existing.status) && isActiveRenderStatus(incoming.status);
  const merged: Extract<RenderNode, { kind: "terminal" }> = {
    ...incoming,
    createdAt: existing.createdAt,
    timelineOrder: existing.timelineOrder,
    status: preserveFinalStatus ? existing.status : incoming.status,
    terminalStatus: localTerminalStatusForMerge(existing, incoming, preserveFinalStatus),
    exitCode: incoming.exitCode ?? existing.exitCode,
    elapsedMs: incoming.elapsedMs ?? existing.elapsedMs,
    output: mergeTerminalText(existing.output, incoming.output, incoming, ["outputDelta", "output_delta"]),
    stdout: mergeTerminalText(existing.stdout, incoming.stdout, incoming, ["stdoutDelta", "stdout_delta"]),
    stderr: mergeTerminalText(existing.stderr, incoming.stderr, incoming, ["stderrDelta", "stderr_delta"])
  };
  const title = incoming.title ?? existing.title;
  if (title !== undefined) {
    merged.title = title;
  }
  const command = incoming.command ?? existing.command;
  if (command !== undefined) {
    merged.command = command;
  }
  const cwd = incoming.cwd ?? existing.cwd;
  if (cwd !== undefined) {
    merged.cwd = cwd;
  }
  return merged;
}

function localTerminalStatusForMerge(
  existing: Extract<RenderNode, { kind: "terminal" }>,
  incoming: Extract<RenderNode, { kind: "terminal" }>,
  preserveFinalStatus: boolean
): string | undefined {
  if (!incoming.terminalStatus) {
    return existing.terminalStatus;
  }
  if (preserveFinalStatus && isToolLikeStatus(incoming.terminalStatus)) {
    return existing.terminalStatus;
  }
  return incoming.terminalStatus;
}

function mergeTerminalText(
  existing: string | undefined,
  incoming: string | undefined,
  incomingSource: unknown,
  deltaKeys: string[]
): string | undefined {
  const delta = terminalDeltaText(incomingSource, deltaKeys);
  if (delta !== undefined) {
    return `${existing || ""}${delta}`;
  }
  return incoming ?? existing;
}

function terminalDeltaText(source: unknown, deltaKeys: string[]): string | undefined {
  const sourceRecord = asRecord(source);
  const rawRecord = asRecord(sourceRecord.raw);
  for (const record of [rawRecord, sourceRecord]) {
    for (const key of deltaKeys) {
      const value = stringField(record, key);
      if (value !== undefined) {
        return value;
      }
    }
  }
  return undefined;
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
  patchType: RenderPatch["type"],
  laterBatchNodes: RenderNode[]
): RenderNode | undefined {
  const laterBatchNodeIds = new Set(laterBatchNodes.map((candidate) => candidate.id));
  if (patchType === "append") {
    return normalizeAppendedLocalNode(nodes, node);
  }
  if (patchType === "replace") {
    return normalizeReplacementLocalNode(nodes, node, laterBatchNodes);
  }
  if (patchType === "upsert" && isStreamNode(node)) {
    const existingIndex = nodes.findIndex((candidate) => candidate.id === node.id);
    const existing = existingIndex >= 0 ? nodes[existingIndex] : undefined;
    if (existing && compatibleStreamTimelineScope(existing, node)) {
      if (!hasNonStreamTimelineBoundaryAfter(nodes, existingIndex, node, laterBatchNodeIds)) {
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

function normalizeReplacementLocalNode(nodes: RenderNode[], node: RenderNode, laterBatchNodes: RenderNode[]): RenderNode {
  const existingById = nodes.find((candidate) => candidate.id === node.id);
  if (!existingById) {
    const semanticMatch = findSameReplacementNode(nodes, node);
    return semanticMatch
      ? {
          ...node,
          id: semanticMatch.id,
          createdAt: semanticMatch.createdAt,
          timelineOrder: semanticMatch.timelineOrder
        }
      : node;
  }
  if (
    canReplaceExistingNodeById(existingById, node) ||
    hasLaterReplacementMoveTarget(existingById, laterBatchNodes)
  ) {
    return node;
  }
  if (isStreamNode(node)) {
    return {
      ...node,
      id: nextStreamNodeId(nodes, node)
    };
  }
  const semanticMatch = findSameReplacementNode(nodes, node);
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
    id: nextScopedCollisionNodeId(nodes, node)
  };
}

function streamNodeWithTimelineIdentity(node: StreamNode, existing: StreamNode): StreamNode {
  return {
    ...node,
    id: existing.id,
    createdAt: existing.createdAt,
    timelineOrder: existing.timelineOrder,
    acpMessageId: existing.acpMessageId || node.acpMessageId
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

function laterPatchNodes(patches: RenderPatch[], patchIndex: number): RenderNode[] {
  const nodes: RenderNode[] = [];
  for (let index = patchIndex + 1; index < patches.length; index += 1) {
    const node = patches[index]?.node;
    if (node) {
      nodes.push(node);
    }
  }
  return nodes;
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
      compatibleStreamMessageIds(existing.acpMessageId, incoming.acpMessageId)
    );
  }
  return existing.kind === "thought" && compatibleStreamMessageIds(existing.acpMessageId, incoming.acpMessageId);
}

function compatibleStreamMessageIds(existingId: string | undefined, incomingId: string | undefined): boolean {
  return existingId === incomingId || existingId === undefined || incomingId === undefined;
}

function mergeLocalStreamNode(existing: StreamNode, incoming: StreamNode): StreamNode {
  if (existing.kind !== incoming.kind) {
    return incoming;
  }
  const content = mergeStreamContentBlocks(existing.content, incoming.content);
  const status = mergedLocalStreamStatus(existing, incoming);
  if (incoming.kind === "message") {
    const existingMessage = existing as Extract<StreamNode, { kind: "message" }>;
    return {
      ...incoming,
      status,
      createdAt: existingMessage.createdAt,
      timelineOrder: existingMessage.timelineOrder,
      acpSessionId: incoming.acpSessionId || existingMessage.acpSessionId,
      provider: incoming.provider || existingMessage.provider,
      acpMessageId: existingMessage.acpMessageId || incoming.acpMessageId,
      content,
      text: contentBlocksToText(content),
      streaming: mergedLocalMessageStreaming(existingMessage, incoming, status)
    };
  }
  return {
    ...incoming,
    status,
    createdAt: existing.createdAt,
    timelineOrder: existing.timelineOrder,
    acpSessionId: incoming.acpSessionId || existing.acpSessionId,
    provider: incoming.provider || existing.provider,
    acpMessageId: existing.acpMessageId || incoming.acpMessageId,
    content
  };
}

function mergedLocalStreamStatus(existing: StreamNode, incoming: StreamNode): RenderNode["status"] {
  return isFinalRenderStatus(existing.status) && isActiveRenderStatus(incoming.status) ? existing.status : incoming.status;
}

function mergedLocalMessageStreaming(
  existing: Extract<StreamNode, { kind: "message" }>,
  incoming: Extract<StreamNode, { kind: "message" }>,
  status: RenderNode["status"]
): boolean {
  return isActiveRenderStatus(status) && (existing.streaming || incoming.streaming);
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
      const previousText = textContentValue(previous);
      const blockText = textContentValue(block);
      if (previousText === undefined || blockText === undefined) {
        merged.push(block);
        continue;
      }
      merged[merged.length - 1] = {
        ...previous,
        text: `${previousText}${blockText}`
      };
      continue;
    }
    merged.push(block);
  }
  return merged;
}

function contentBlocksToText(blocks: ContentBlock[]): string {
  return blocks.map(contentBlockToText).join("");
}

function contentBlockToText(content: ContentBlock): string {
  const record = content as Record<string, unknown>;
  const text = textContentValue(content);
  if (text !== undefined) {
    return text;
  }
  if (content.type === "resource_link" && typeof record.name === "string") {
    return typeof record.title === "string" ? `${record.title} (${record.name})` : record.name;
  }
  const resource = record.resource as Record<string, unknown> | undefined;
  if (content.type === "resource" && resource && typeof resource.text === "string") {
    return resource.text;
  }
  if (content.type === "image") {
    return "[image]";
  }
  if (content.type === "audio") {
    return "[audio]";
  }
  return `[${content.type || "content"}]`;
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
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
    if (candidate && compatibleStreamTimelineScope(candidate, node)) {
      return candidate;
    }
  }
  return undefined;
}

function hasNonStreamTimelineBoundaryAfter(
  nodes: RenderNode[],
  index: number,
  node: RenderNode,
  ignoredBoundaryIds: Set<string>
): boolean {
  for (let nextIndex = index + 1; nextIndex < nodes.length; nextIndex += 1) {
    const candidate = nodes[nextIndex];
    if (
      candidate &&
      !ignoredBoundaryIds.has(candidate.id) &&
      compatibleStreamTimelineScope(candidate, node) &&
      isNonStreamTimelineBoundary(candidate)
    ) {
      return true;
    }
  }
  return false;
}

function isNonStreamTimelineBoundary(node: RenderNode): boolean {
  if (isStreamNode(node)) {
    return false;
  }
  switch (node.kind) {
    case "commands":
    case "config":
    case "mode":
    case "session":
    case "usage":
      return false;
    default:
      return true;
  }
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
    if (!sameSemanticNodeWhenKnown(existing, incoming)) {
      return false;
    }
    return (
      !hasExternalTimelineBoundaryAfter(nodes, existingIndex, incoming) ||
      canUpdateAfterExternalBoundary(existing, incoming)
    );
  }
  const existingKey = semanticNodeKey(existing);
  return existingKey !== undefined && existingKey === semanticNodeKey(incoming);
}

function canReplaceExistingNodeById(existing: RenderNode, incoming: RenderNode): boolean {
  if (existing.kind !== incoming.kind || existing.taskId !== incoming.taskId || existing.lane !== incoming.lane) {
    return false;
  }
  if (isGlobalControlNode(existing) && isGlobalControlNode(incoming)) {
    return true;
  }
  return (
    sameOptionalScopeValue(existing.turnId, incoming.turnId) &&
    sameOptionalScopeValue(existing.acpSessionId, incoming.acpSessionId) &&
    sameSemanticNodeWhenKnown(existing, incoming)
  );
}

function hasLaterReplacementMoveTarget(existing: RenderNode, laterBatchNodes: RenderNode[]): boolean {
  return laterBatchNodes.some((candidate) => candidate.id !== existing.id && sameReplacementMoveTarget(existing, candidate));
}

function sameReplacementMoveTarget(existing: RenderNode, candidate: RenderNode): boolean {
  if (existing.kind !== candidate.kind || !sameTimelineScope(existing, candidate)) {
    return false;
  }
  return replacementMoveKey(existing) !== undefined && replacementMoveKey(existing) === replacementMoveKey(candidate);
}

function replacementMoveKey(node: RenderNode): string | undefined {
  switch (node.kind) {
    case "message":
      return `message:${node.role}:${node.acpMessageId || ""}`;
    case "thought":
      return `thought:${node.acpMessageId || ""}`;
    case "tool":
      return `tool:${node.acpToolCallId || node.toolCallId}`;
    case "terminal":
      return `terminal:${node.acpToolCallId || ""}:${node.terminalId}`;
    case "diff":
      return `diff:${node.acpToolCallId || ""}:${node.path}`;
    case "approval":
      return `approval:${approvalToolIdentity(node) || ""}:${node.requestId}`;
    case "plan":
      return "plan";
    case "checkpoint":
      return `checkpoint:${node.checkpointId || ""}`;
    case "completion":
      return "completion";
    case "unknown":
      return `unknown:${unknownEventIdentity(node)}`;
    default:
      return undefined;
  }
}

function isGlobalControlNode(node: RenderNode): boolean {
  switch (node.kind) {
    case "commands":
    case "config":
    case "mode":
    case "session":
    case "usage":
      return true;
    default:
      return false;
  }
}

function sameOptionalScopeValue(left: string | undefined, right: string | undefined): boolean {
  return left === right || left === undefined || right === undefined;
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
  const toolCallIds = node.kind === "tool"
    ? [node.toolCallId, node.acpToolCallId].filter((id): id is string => typeof id === "string")
    : [node.acpToolCallId].filter((id): id is string => typeof id === "string");
  if (!toolCallIds.length) {
    return false;
  }
  return toolCallIds.includes(candidate.acpToolCallId || "") && (candidate.kind === "terminal" || candidate.kind === "diff");
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

function compatibleStreamTimelineScope(left: RenderNode, right: RenderNode): boolean {
  return (
    left.taskId === right.taskId &&
    left.lane === right.lane &&
    left.turnId === right.turnId &&
    compatibleOptionalScopeValue(left.acpSessionId, right.acpSessionId) &&
    left.source === right.source
  );
}

function compatibleOptionalScopeValue(left: string | undefined, right: string | undefined): boolean {
  return left === right || left === undefined || right === undefined;
}

function findSameSemanticNode(nodes: RenderNode[], node: RenderNode): RenderNode | undefined {
  const key = semanticNodeKey(node);
  if (!key) {
    return undefined;
  }
  return nodes.find((candidate) => candidate.id !== node.id && semanticNodeKey(candidate) === key);
}

function findSameReplacementNode(nodes: RenderNode[], node: RenderNode): RenderNode | undefined {
  return nodes.find((candidate) => candidate.id !== node.id && sameSnapshotReplacementNode(candidate, node));
}

function sameSnapshotReplacementNode(left: RenderNode, right: RenderNode): boolean {
  if (left.kind !== right.kind || !sameSnapshotReplacementScope(left, right)) {
    return false;
  }
  switch (left.kind) {
    case "message":
      return right.kind === "message" && sameSnapshotMessageNode(left, right);
    case "thought":
      return right.kind === "thought" && sameSnapshotText(contentBlocksToText(left.content), contentBlocksToText(right.content));
    case "tool":
      return right.kind === "tool" && sameRequiredString(toolIdentity(left), toolIdentity(right));
    case "terminal":
      return right.kind === "terminal" && sameRequiredString(left.terminalId, right.terminalId) && sameOptionalString(left.acpToolCallId, right.acpToolCallId);
    case "diff":
      return right.kind === "diff" && left.path === right.path && sameOptionalString(left.acpToolCallId, right.acpToolCallId);
    case "approval":
      return right.kind === "approval" && left.requestId === right.requestId && sameOptionalString(approvalToolIdentity(left), approvalToolIdentity(right));
    case "plan":
    case "completion":
      return true;
    case "checkpoint":
      return right.kind === "checkpoint" && sameOptionalString(left.checkpointId, right.checkpointId);
    case "resource":
      return right.kind === "resource" && stableJson(left.content) === stableJson(right.content);
    case "unknown":
      return right.kind === "unknown" && unknownEventIdentity(left) === unknownEventIdentity(right);
    default:
      return false;
  }
}

function sameSnapshotReplacementScope(left: RenderNode, right: RenderNode): boolean {
  return (
    left.taskId === right.taskId &&
    left.lane === right.lane &&
    compatibleSnapshotTurnScope(left, right) &&
    compatibleOptionalScopeValue(left.acpSessionId, right.acpSessionId) &&
    compatibleOptionalScopeValue(left.provider, right.provider)
  );
}

function compatibleSnapshotTurnScope(left: RenderNode, right: RenderNode): boolean {
  if (compatibleOptionalScopeValue(left.turnId, right.turnId)) {
    return true;
  }
  return isCompletedLiveHydrationReplacement(left, right);
}

function isCompletedLiveHydrationReplacement(left: RenderNode, right: RenderNode): boolean {
  const live = left.source === "acp-live" ? left : right.source === "acp-live" ? right : undefined;
  const hydrated = left.source === "crabdb" ? left : right.source === "crabdb" ? right : undefined;
  return Boolean(live && hydrated && !isActiveReplacementNode(live));
}

function isActiveReplacementNode(node: RenderNode): boolean {
  if (isActiveRenderStatus(node.status)) {
    return true;
  }
  if (node.kind === "tool") {
    return isActiveRenderStatus(node.toolStatus);
  }
  if (node.kind === "terminal") {
    return isActiveRenderStatus(node.terminalStatus);
  }
  return false;
}

function sameSnapshotMessageNode(
  left: Extract<RenderNode, { kind: "message" }>,
  right: Extract<RenderNode, { kind: "message" }>
): boolean {
  if (left.role !== right.role) {
    return false;
  }
  if (left.acpMessageId && right.acpMessageId) {
    return left.acpMessageId === right.acpMessageId;
  }
  return sameSnapshotText(left.text, right.text);
}

function sameSnapshotText(left: string, right: string): boolean {
  const normalizedLeft = normalizeSnapshotText(left);
  const normalizedRight = normalizeSnapshotText(right);
  if (!normalizedLeft || !normalizedRight) {
    return false;
  }
  return (
    normalizedLeft === normalizedRight ||
    normalizedLeft.startsWith(normalizedRight) ||
    normalizedRight.startsWith(normalizedLeft)
  );
}

function normalizeSnapshotText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function toolIdentity(node: Extract<RenderNode, { kind: "tool" }>): string | undefined {
  return node.acpToolCallId || node.toolCallId;
}

function sameRequiredString(left: string | undefined, right: string | undefined): boolean {
  return Boolean(left && right && left === right);
}

function sameOptionalString(left: string | undefined, right: string | undefined): boolean {
  return left === right || left === undefined || right === undefined;
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
      return `${scope}\u0000approval\u0000${approvalToolIdentity(node) || ""}\u0000${node.requestId}`;
    case "plan":
      return `${scope}\u0000plan`;
    case "resource":
      return `${scope}\u0000resource\u0000${node.id}`;
    case "unknown":
      return `${scope}\u0000unknown\u0000${unknownEventIdentity(node)}`;
    default:
      return undefined;
  }
}

function sameSemanticNodeWhenKnown(left: RenderNode, right: RenderNode): boolean {
  const leftKey = replacementMoveKey(left);
  const rightKey = replacementMoveKey(right);
  return leftKey === undefined || rightKey === undefined || leftKey === rightKey;
}

function approvalToolIdentity(node: Extract<RenderNode, { kind: "approval" }>): string | undefined {
  return node.acpToolCallId || node.tool.acpToolCallId || node.tool.toolCallId;
}

function unknownEventIdentity(node: Extract<RenderNode, { kind: "unknown" }>): string {
  return `${node.label}\u0000${stableJson(node.payload)}`;
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
    source: node.source,
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
    node.kind === "resource" ||
    node.kind === "unknown"
  );
}

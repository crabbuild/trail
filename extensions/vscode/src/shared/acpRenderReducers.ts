import type {
  AgentMessageChunkUpdate,
  AgentThoughtChunkUpdate,
  AvailableCommand,
  AvailableCommandsUpdate,
  ConfigOptionUpdate,
  ContentBlock,
  CurrentModeUpdate,
  RequestPermissionParams,
  SessionUpdate,
  PlanUpdate,
  SessionConfigOption,
  SessionInfoUpdate,
  SessionMode,
  ToolCallContent,
  ToolCallLocation,
  ToolCallStatus,
  ToolTerminalBlock,
  ToolCallPatchUpdate,
  ToolCallUpdate,
  UsageUpdate,
  UserMessageChunkUpdate
} from "./acpTypes";
import { redactString } from "./securityRedaction";
import type {
  ApprovalNode,
  DiffNode,
  MessageNode,
  RenderNode,
  RenderPatch,
  RenderReduceContext,
  TerminalNode,
  ThoughtNode,
  ToolNode
} from "./renderModel";

type StreamNode = MessageNode | ThoughtNode;

export interface AcpUpdateRenderer<TUpdate extends SessionUpdate = SessionUpdate> {
  match(update: SessionUpdate): update is TUpdate;
  reduce(update: TUpdate, context: RenderReduceContext): RenderPatch[];
}

export interface AppliedRenderPatches {
  nodes: RenderNode[];
  patches: RenderPatch[];
}

export function reduceSessionUpdate(
  update: SessionUpdate,
  context: RenderReduceContext
): RenderPatch[] {
  const normalizedUpdate = normalizeSessionUpdate(update);
  const renderer = updateRenderers.find((candidate) => candidate.match(normalizedUpdate));
  if (!renderer) {
    return [upsertUnknown(normalizedUpdate, context, `Unsupported ACP update: ${String(normalizedUpdate.sessionUpdate)}`)];
  }
  return renderer.reduce(normalizedUpdate, context);
}

function normalizeSessionUpdate(update: SessionUpdate): SessionUpdate {
  const record = asPlainRecord(update);
  const sessionUpdate = stringField(record, "sessionUpdate") || stringField(record, "session_update");
  const canonicalSessionUpdate = sessionUpdate === "plan_update" ? "plan" : sessionUpdate;
  if (!canonicalSessionUpdate || record.sessionUpdate === canonicalSessionUpdate) {
    return update;
  }
  return {
    ...record,
    sessionUpdate: canonicalSessionUpdate
  } as SessionUpdate;
}

export function reducePermissionRequest(
  requestId: string,
  params: RequestPermissionParams,
  context: RenderReduceContext
): RenderPatch[] {
  const requestContext: RenderReduceContext = {
    ...context,
    acpSessionId: params.sessionId || context.acpSessionId
  };
  const tool = toolNodeFromCall(params.toolCall, requestContext);
  const node: ApprovalNode = {
    id: `approval:${requestId}`,
    kind: "approval",
    taskId: requestContext.taskId,
    lane: requestContext.lane,
    acpSessionId: requestContext.acpSessionId,
    acpToolCallId: tool.acpToolCallId || tool.toolCallId,
    turnId: requestContext.currentTurnId,
    provider: requestContext.provider,
    source: "acp-live",
    status: "pending",
    createdAt: requestContext.now(),
    updatedAt: requestContext.now(),
    raw: params,
    requestId,
    title: params.toolCall.title || "Permission required",
    tool,
    options: params.options.map((option) => {
      const mapped: { optionId: string; label: string; description?: string | undefined } = {
        optionId: option.optionId,
        label: option.name || option.optionId
      };
      if (option.description) {
        mapped.description = option.description;
      }
      return mapped;
    })
  };
  return [{ type: "upsert", node }];
}

export function applyRenderPatches(nodes: RenderNode[], patches: RenderPatch[]): RenderNode[] {
  return applyRenderPatchesAndCollect(nodes, patches).nodes;
}

export function renderNodeSnapshotPatches(before: RenderNode[], next: RenderNode[]): RenderPatch[] {
  const beforeById = new Map(before.map((node) => [node.id, node]));
  const nextIds = new Set(next.map((node) => node.id));
  const semanticReplacementMatches = snapshotSemanticReplacementMatches(before, next, nextIds);
  const semanticReplacementBeforeIds = new Set(
    [...semanticReplacementMatches.values()].map((node) => node.id)
  );
  const patches: RenderPatch[] = [];

  for (const node of before) {
    if (!nextIds.has(node.id) && !semanticReplacementBeforeIds.has(node.id)) {
      patches.push({ type: "remove", id: node.id });
    }
  }

  for (let index = 0; index < next.length; index += 1) {
    const node = next[index]!;
    const previous = beforeById.get(node.id);
    if (!previous) {
      patches.push({ type: semanticReplacementMatches.has(index) ? "replace" : "upsert", node });
    } else if (!sameRenderNodeSnapshot(previous, node)) {
      patches.push({ type: "replace", node });
    }
  }

  return patches;
}

function snapshotSemanticReplacementMatches(
  before: RenderNode[],
  next: RenderNode[],
  nextIds: ReadonlySet<string>
): Map<number, RenderNode> {
  const matches = new Map<number, RenderNode>();
  const beforeIds = new Set(before.map((node) => node.id));
  const usedBeforeIds = new Set<string>();
  for (let index = 0; index < next.length; index += 1) {
    const node = next[index]!;
    if (beforeIds.has(node.id)) {
      continue;
    }
    const match = findSameSnapshotSemanticNode(before, node, nextIds, usedBeforeIds);
    if (!match) {
      continue;
    }
    matches.set(index, match);
    usedBeforeIds.add(match.id);
  }
  return matches;
}

function findSameSnapshotSemanticNode(
  nodes: RenderNode[],
  node: RenderNode,
  retainedNodeIds: ReadonlySet<string>,
  usedNodeIds: ReadonlySet<string>
): RenderNode | undefined {
  return nodes.find((candidate) => (
    candidate.id !== node.id &&
    !retainedNodeIds.has(candidate.id) &&
    !usedNodeIds.has(candidate.id) &&
    sameSnapshotReplacementNode(candidate, node)
  ));
}

export function applyRenderPatchesAndCollect(nodes: RenderNode[], patches: RenderPatch[]): AppliedRenderPatches {
  let next = [...nodes];
  let nextTimelineOrder = maxTimelineOrder(next);
  const appliedPatches: RenderPatch[] = [];
  const normalizedIdsByRawId = new Map<string, string[]>();
  for (let patchIndex = 0; patchIndex < patches.length; patchIndex += 1) {
    const patch = patches[patchIndex]!;
    const patchNode = patch.node ? normalizePatchNodeForTimeline(next, patch, laterPatchNodes(patches, patchIndex)) : undefined;
    if (patch.type === "append" && patchNode) {
      const rawPatchId = patch.node?.id;
      const ordered = ensureTimelineOrder(patchNode, () => {
        nextTimelineOrder += 1;
        return nextTimelineOrder;
      });
      nextTimelineOrder = Math.max(nextTimelineOrder, ordered.timelineOrder ?? 0);
      next.push(ordered);
      appliedPatches.push({ type: "upsert", node: ordered });
      if (rawPatchId) {
        trackNormalizedPatchId(normalizedIdsByRawId, rawPatchId, ordered.id);
      }
      const movedFinalMarkers = moveTurnFinalMarkersAfterNode(next, ordered, nextTimelineOrder);
      next = movedFinalMarkers.nodes;
      nextTimelineOrder = movedFinalMarkers.nextTimelineOrder;
      appliedPatches.push(...movedFinalMarkers.patches);
      continue;
    }
    if ((patch.type === "replace" || patch.type === "upsert") && patchNode) {
      const rawPatchId = patch.node?.id;
      const index = next.findIndex((node) => node.id === patchNode.id);
      let appliedNode: RenderNode | undefined;
      let existingNode: RenderNode | undefined;
      if (index >= 0) {
        const existing = next[index]!;
        existingNode = existing;
        const orderedPatchNode = preserveTimelineOrder(patchNode, existing);
        next[index] = patch.type === "upsert" ? mergeRenderNode(existing, orderedPatchNode) : orderedPatchNode;
        appliedNode = next[index];
      } else {
        const ordered = ensureTimelineOrder(patchNode, () => {
          nextTimelineOrder += 1;
          return nextTimelineOrder;
        });
        nextTimelineOrder = Math.max(nextTimelineOrder, ordered.timelineOrder ?? 0);
        next.push(ordered);
        appliedNode = ordered;
      }
      if (appliedNode && appliedNode !== existingNode) {
        appliedPatches.push({ type: "upsert", node: appliedNode });
        if (rawPatchId) {
          trackNormalizedPatchId(normalizedIdsByRawId, rawPatchId, appliedNode.id);
        }
      }
      if (appliedNode?.kind === "tool") {
        const synced = syncExpandedToolChildNodesAndCollect(next, appliedNode);
        next = synced.nodes;
        appliedPatches.push(...synced.patches);
      }
      if (appliedNode) {
        const movedFinalMarkers = moveTurnFinalMarkersAfterNode(next, appliedNode, nextTimelineOrder);
        next = movedFinalMarkers.nodes;
        nextTimelineOrder = movedFinalMarkers.nextTimelineOrder;
        appliedPatches.push(...movedFinalMarkers.patches);
      }
      continue;
    }
    if (patch.type === "remove" && patch.id) {
      const removeId = consumeNormalizedPatchId(normalizedIdsByRawId, patch.id) || patch.id;
      const beforeLength = next.length;
      next = next.filter((node) => node.id !== removeId);
      if (next.length !== beforeLength) {
        appliedPatches.push({ type: "remove", id: removeId });
      }
    }
  }
  return { nodes: next, patches: appliedPatches };
}

function moveTurnFinalMarkersAfterNode(
  nodes: RenderNode[],
  node: RenderNode,
  currentTimelineOrder: number
): { nodes: RenderNode[]; patches: RenderPatch[]; nextTimelineOrder: number } {
  if (!node.turnId || isTurnFinalMarker(node)) {
    return { nodes, patches: [], nextTimelineOrder: currentTimelineOrder };
  }
  const nodeIndex = nodes.findIndex((candidate) => candidate.id === node.id);
  if (nodeIndex < 0) {
    return { nodes, patches: [], nextTimelineOrder: currentTimelineOrder };
  }
  let nextTimelineOrder = currentTimelineOrder;
  const kept: RenderNode[] = [];
  const moved: RenderNode[] = [];
  const patches: RenderPatch[] = [];
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
      patches.push({ type: "upsert", node: movedNode });
      return;
    }
    kept.push(candidate);
  });
  return patches.length
    ? { nodes: [...kept, ...moved], patches, nextTimelineOrder }
    : { nodes, patches, nextTimelineOrder };
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

function sameRenderNodeSnapshot(left: RenderNode, right: RenderNode): boolean {
  if (left === right) {
    return true;
  }
  return stableJson(left) === stableJson(right);
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

function maxTimelineOrder(nodes: RenderNode[]): number {
  return nodes.reduce((max, node, index) => Math.max(max, node.timelineOrder ?? index + 1), 0);
}

function ensureTimelineOrder<TNode extends RenderNode>(node: TNode, allocate: () => number): TNode {
  return node.timelineOrder === undefined ? ({ ...node, timelineOrder: allocate() } as TNode) : node;
}

function preserveTimelineOrder<TNode extends RenderNode>(incoming: TNode, existing: RenderNode): TNode {
  if (incoming.timelineOrder !== undefined) {
    return incoming;
  }
  const timelineOrder = existing.timelineOrder;
  return timelineOrder === undefined ? incoming : ({ ...incoming, timelineOrder } as TNode);
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

function normalizePatchNodeForTimeline(
  nodes: RenderNode[],
  patch: RenderPatch,
  laterBatchNodes: RenderNode[]
): RenderNode | undefined {
  const node = patch.node;
  if (!node) {
    return node;
  }
  const laterBatchNodeIds = new Set(laterBatchNodes.map((candidate) => candidate.id));
  if (patch.type === "append") {
    return normalizeAppendNodeForTimeline(nodes, node);
  }
  if (patch.type === "replace") {
    return normalizeReplacementNodeForTimeline(nodes, node, laterBatchNodes);
  }
  if (patch.type !== "upsert") {
    return node;
  }
  if (isStreamNode(node)) {
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
  return normalizeCollidingUpsertNode(nodes, node);
}

function normalizeReplacementNodeForTimeline<TNode extends RenderNode>(
  nodes: RenderNode[],
  node: TNode,
  laterBatchNodes: RenderNode[]
): TNode {
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

function normalizeAppendNodeForTimeline<TNode extends RenderNode>(nodes: RenderNode[], node: TNode): TNode {
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

function isTextOnlyStreamNode(node: StreamNode): boolean {
  return node.content.length > 0 && node.content.every((block) => block.type === "text");
}

function streamNodeWithContent(node: StreamNode, content: ContentBlock[]): StreamNode {
  return node.kind === "message"
    ? { ...node, content, text: contentBlocksToText(content) }
    : { ...node, content };
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

function normalizeCollidingUpsertNode<TNode extends RenderNode>(nodes: RenderNode[], node: TNode): TNode {
  const existingIndex = nodes.findIndex((candidate) => candidate.id === node.id);
  const existingById = existingIndex >= 0 ? nodes[existingIndex] : undefined;
  if (!existingById || canUpdateExistingNodeById(nodes, existingIndex, node)) {
    return node;
  }
  const semanticMatch = findSameSemanticNode(nodes, node);
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

function rawSessionUpdate(node: RenderNode): string | undefined {
  const raw = node.raw;
  if (!raw || typeof raw !== "object") {
    return undefined;
  }
  const value = (raw as { sessionUpdate?: unknown }).sessionUpdate;
  return typeof value === "string" ? value : undefined;
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
    compatibleOptionalScopeValue(left.turnId, right.turnId) &&
    compatibleOptionalScopeValue(left.acpSessionId, right.acpSessionId) &&
    compatibleOptionalScopeValue(left.provider, right.provider)
  );
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

function sanitizeIdSegment(value: string): string {
  return value
    .trim()
    .replace(/[^A-Za-z0-9_.:-]+/g, "-")
    .replace(/^-+|-+$/g, "");
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

export function sessionControlsToPatches(session: unknown, context: RenderReduceContext): RenderPatch[] {
  const record = asRecord(session);
  const patches: RenderPatch[] = [];
  const modes = asRecord(record.modes);
  const availableModes = firstArrayField(modes, "availableModes", "available_modes").filter(isSessionMode);
  const currentModeId = stringField(modes, "currentModeId") || stringField(modes, "current_mode_id") || stringField(modes, "modeId");
  if (currentModeId || availableModes.length) {
    patches.push({
      type: "upsert",
      node: {
        id: `mode:${context.taskId}`,
        kind: "mode",
        taskId: context.taskId,
        lane: context.lane,
        acpSessionId: context.acpSessionId,
        provider: context.provider,
        source: "acp-live",
        status: "completed",
        updatedAt: context.now(),
        raw: modes,
        modeId: currentModeId || availableModes[0]?.id || "unknown",
        availableModes
      }
    });
  }

  const rawConfigOptions = firstArrayField(record, "configOptions", "config_options");
  const configOptions = normalizeSessionConfigOptions(rawConfigOptions);
  if (configOptions.length) {
    patches.push({
      type: "upsert",
      node: {
        id: `config:${context.taskId}`,
        kind: "config",
        taskId: context.taskId,
        lane: context.lane,
        acpSessionId: context.acpSessionId,
        provider: context.provider,
        source: "acp-live",
        status: "completed",
        updatedAt: context.now(),
        raw: rawConfigOptions,
        configOptions
      }
    });
  }

  return patches;
}

export function contentToText(content: ContentBlock): string {
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

function streamContentBlocks(value: unknown): ContentBlock[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => streamContentBlocks(item));
  }
  const content = streamContentBlock(value);
  return content ? [content] : [];
}

function streamContentBlock(value: unknown): ContentBlock | undefined {
  if (typeof value === "string") {
    return { type: "text", text: value };
  }
  const record = asPlainRecord(value);
  if (!Object.keys(record).length) {
    return undefined;
  }
  if (record.type === "text" && typeof record.text !== "string") {
    const text = stringField(record, "content") || stringField(record, "value");
    if (text !== undefined) {
      return { ...record, type: "text", text } as ContentBlock;
    }
  }
  if (typeof record.type === "string") {
    return record as ContentBlock;
  }
  const text = stringField(record, "text") || stringField(record, "content") || stringField(record, "value");
  return text === undefined ? undefined : { type: "text", text };
}

function textContentValue(content: ContentBlock): string | undefined {
  if (content.type !== "text") {
    return undefined;
  }
  const record = content as Record<string, unknown>;
  const fallbackValues: string[] = [];
  for (const key of ["text", "content", "value"]) {
    const value = stringField(record, key);
    if (value === undefined) {
      continue;
    }
    if (value.length > 0) {
      return value;
    }
    fallbackValues.push(value);
  }
  return fallbackValues[0];
}

function contentBlocksToText(blocks: ContentBlock[]): string {
  return blocks.map(contentToText).join("");
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

function mergeRenderNode(existing: RenderNode, incoming: RenderNode): RenderNode {
  if (existing.kind === "message" && incoming.kind === "message") {
    const content = mergeStreamContentBlocks(existing.content, incoming.content);
    const status = mergedStreamStatus(existing, incoming);
    return {
      ...incoming,
      status,
      createdAt: existing.createdAt,
      timelineOrder: existing.timelineOrder,
      acpSessionId: incoming.acpSessionId || existing.acpSessionId,
      provider: incoming.provider || existing.provider,
      acpMessageId: existing.acpMessageId || incoming.acpMessageId,
      content,
      text: contentBlocksToText(content),
      streaming: mergedMessageStreaming(existing, incoming, status)
    };
  }
  if (existing.kind === "thought" && incoming.kind === "thought") {
    const content = mergeStreamContentBlocks(existing.content, incoming.content);
    return {
      ...incoming,
      status: mergedStreamStatus(existing, incoming),
      createdAt: existing.createdAt,
      timelineOrder: existing.timelineOrder,
      acpSessionId: incoming.acpSessionId || existing.acpSessionId,
      provider: incoming.provider || existing.provider,
      acpMessageId: existing.acpMessageId || incoming.acpMessageId,
      content
    };
  }
  if (existing.kind === "tool" && incoming.kind === "tool") {
    const explicitStatus = hasExplicitToolStatus(incoming);
    return syncToolTerminalContent({
      ...incoming,
      createdAt: existing.createdAt,
      timelineOrder: existing.timelineOrder,
      status: explicitStatus ? incoming.status : existing.status,
      title: incoming.title && incoming.title !== "Tool call" ? incoming.title : existing.title,
      toolKind: incoming.toolKind !== "other" ? incoming.toolKind : existing.toolKind,
      toolStatus: explicitStatus ? incoming.toolStatus : existing.toolStatus,
      locations: incoming.locations.length ? mergeToolLocations(existing.locations, incoming.locations) : existing.locations,
      content: incoming.content.length ? mergeToolContent(existing.content, incoming.content) : existing.content,
      rawInput: incoming.rawInput ?? existing.rawInput,
      rawOutput: incoming.rawOutput ?? existing.rawOutput
    });
  }
  if (existing.kind === "terminal" && incoming.kind === "terminal") {
    return mergeTerminalNode(existing, incoming);
  }
  return incoming;
}

function mergedStreamStatus(existing: StreamNode, incoming: StreamNode): RenderNode["status"] {
  return isFinalRenderStatus(existing.status) && isActiveRenderStatus(incoming.status) ? existing.status : incoming.status;
}

function mergedMessageStreaming(existing: MessageNode, incoming: MessageNode, status: RenderNode["status"]): boolean {
  return isActiveRenderStatus(status) && (existing.streaming || incoming.streaming);
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

function hasExplicitToolStatus(node: ToolNode): boolean {
  const raw = node.raw as Record<string, unknown> | undefined;
  return typeof raw?.status === "string" || typeof raw?.state === "string";
}

function mergeToolContent(existing: ToolCallContent[], incoming: ToolCallContent[]): ToolCallContent[] {
  const merged = [...existing];
  for (const item of incoming) {
    const key = stableContentKey(item);
    const index = merged.findIndex((candidate) => stableContentKey(candidate) === key);
    if (index >= 0) {
      const mergedItem = mergeToolContentItem(merged[index]!, item);
      if (mergedItem) {
        merged[index] = mergedItem;
      }
      continue;
    }
    merged.push(item);
  }
  return merged;
}

function mergeToolContentItem(existing: ToolCallContent, incoming: ToolCallContent): ToolCallContent | undefined {
  const existingRecord = existing as Record<string, unknown>;
  const incomingRecord = incoming as Record<string, unknown>;
  if (
    existingRecord.type === "terminal" &&
    incomingRecord.type === "terminal" &&
    typeof existingRecord.terminalId === "string" &&
    existingRecord.terminalId === incomingRecord.terminalId
  ) {
    return mergeTerminalContentRecord(existingRecord, incomingRecord) as ToolCallContent;
  }
  return undefined;
}

function mergeTerminalContentRecord(
  existing: Record<string, unknown>,
  incoming: Record<string, unknown>
): Record<string, unknown> {
  const merged = {
    ...existing,
    ...incoming
  };
  assignMergedTerminalText(merged, existing, incoming, "output", ["outputDelta", "output_delta"]);
  assignMergedTerminalText(merged, existing, incoming, "stdout", ["stdoutDelta", "stdout_delta"]);
  assignMergedTerminalText(merged, existing, incoming, "stderr", ["stderrDelta", "stderr_delta"]);
  return merged;
}

function assignMergedTerminalText(
  target: Record<string, unknown>,
  existing: Record<string, unknown>,
  incoming: Record<string, unknown>,
  key: "output" | "stdout" | "stderr",
  deltaKeys: string[]
): void {
  const existingValue = stringField(existing, key);
  const incomingValue = stringField(incoming, key);
  const merged = mergeTerminalText(existingValue, incomingValue, incoming, deltaKeys);
  if (merged !== undefined) {
    target[key] = merged;
  }
}

function syncToolTerminalContent(tool: ToolNode): ToolNode {
  let changed = false;
  const content = tool.content.map((item) => {
    const record = item as Record<string, unknown>;
    if (record.type !== "terminal") {
      return item;
    }
    const current =
      stringRecordField(record, "terminalStatus") ||
      stringRecordField(record, "status") ||
      stringRecordField(record, "state");
    const terminalStatus = syncTerminalStatusFromTool(current, tool.toolStatus);
    if (!terminalStatus || terminalStatus === current) {
      return item;
    }
    changed = true;
    const next: Record<string, unknown> = {
      ...record,
      status: terminalStatus
    };
    if (typeof record.terminalStatus === "string") {
      next.terminalStatus = terminalStatus;
    }
    return next as ToolCallContent;
  });
  return changed ? { ...tool, content } : tool;
}

function stringRecordField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function mergeToolLocations<TLocation extends { path: string; line?: number | null | undefined }>(
  existing: TLocation[],
  incoming: TLocation[]
): TLocation[] {
  const seen = new Set(existing.map((location) => `${location.path}:${location.line ?? ""}`));
  const merged = [...existing];
  for (const location of incoming) {
    const key = `${location.path}:${location.line ?? ""}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    merged.push(location);
  }
  return merged;
}

function stableContentKey(content: ToolCallContent): string {
  const record = content as Record<string, unknown>;
  if (record.type === "terminal" && typeof record.terminalId === "string") {
    return `terminal:${record.terminalId}`;
  }
  try {
    return JSON.stringify(content);
  } catch {
    return String(content);
  }
}

const userMessageRenderer: AcpUpdateRenderer<UserMessageChunkUpdate> = {
  match: (update): update is UserMessageChunkUpdate => update.sessionUpdate === "user_message_chunk",
  reduce(update, context) {
    return [messagePatch("user", messageChunkId(update), messageChunkContent(update), context, true, update)];
  }
};

const agentMessageRenderer: AcpUpdateRenderer<AgentMessageChunkUpdate> = {
  match: (update): update is AgentMessageChunkUpdate => update.sessionUpdate === "agent_message_chunk",
  reduce(update, context) {
    return [messagePatch("assistant", messageChunkId(update), messageChunkContent(update), context, true, update)];
  }
};

const thoughtRenderer: AcpUpdateRenderer<AgentThoughtChunkUpdate> = {
  match: (update): update is AgentThoughtChunkUpdate => update.sessionUpdate === "agent_thought_chunk",
  reduce(update, context) {
    const messageId = messageChunkId(update);
    const id = `thought:${messageId || "anonymous"}`;
    const content = streamContentBlocks(messageChunkContent(update));
    const node: ThoughtNode = {
      id,
      kind: "thought",
      taskId: context.taskId,
      lane: context.lane,
      turnId: context.currentTurnId,
      acpSessionId: context.acpSessionId,
      acpMessageId: messageId,
      provider: context.provider,
      source: "acp-live",
      status: "in_progress",
      createdAt: context.now(),
      updatedAt: context.now(),
      raw: update,
      content,
      ephemeral: true
    };
    return [{ type: "upsert", node }];
  }
};

const planRenderer: AcpUpdateRenderer<PlanUpdate> = {
  match: (update): update is PlanUpdate =>
    update.sessionUpdate === "plan",
  reduce(update, context) {
    const record = update as unknown as Record<string, unknown>;
    return [
      {
        type: "upsert",
        node: {
          id: `plan:${context.currentTurnId || context.taskId}`,
          kind: "plan",
          taskId: context.taskId,
          lane: context.lane,
          turnId: context.currentTurnId,
          acpSessionId: context.acpSessionId,
          provider: context.provider,
          source: "acp-live",
          status: "in_progress",
          createdAt: context.now(),
          updatedAt: context.now(),
          raw: update,
          entries: normalizePlanEntries(record)
        }
      }
    ];
  }
};

const toolCallRenderer: AcpUpdateRenderer<ToolCallUpdate> = {
  match: (update): update is ToolCallUpdate => update.sessionUpdate === "tool_call",
  reduce(update, context) {
    return expandToolContent(toolNodeFromCall(update, context), context);
  }
};

const toolCallPatchRenderer: AcpUpdateRenderer<ToolCallPatchUpdate> = {
  match: (update): update is ToolCallPatchUpdate => update.sessionUpdate === "tool_call_update",
  reduce(update, context) {
    const status = toolCallStatusFromRecord(update as unknown as Record<string, unknown>);
    const toolCallId = toolCallIdFromCall(update);
    const rawInput = rawInputFromCall(update);
    const rawOutput = rawOutputFromCall(update);
    const base: ToolCallUpdate = {
      sessionUpdate: "tool_call",
      toolCallId,
      title: update.title || "Tool call",
      kind: update.kind || "other",
      locations: normalizedToolLocations(update),
      content: normalizedToolContent(update)
    };
    if (status) {
      base.status = status;
    }
    if (rawInput) {
      base.rawInput = rawInput;
    }
    if (rawOutput) {
      base.rawOutput = rawOutput;
    }
    if (update._meta) {
      base._meta = update._meta;
    }
    return expandToolContent({ ...toolNodeFromCall(base, context), raw: update }, context);
  }
};

const modeRenderer: AcpUpdateRenderer<CurrentModeUpdate> = {
  match: (update): update is CurrentModeUpdate =>
    update.sessionUpdate === "current_mode_update",
  reduce(update, context) {
    const record = update as unknown as Record<string, unknown>;
    const availableModes = firstArrayField(record, "availableModes", "available_modes").filter(isSessionMode);
    const modeId = update.currentModeId || update.modeId || stringField(record, "current_mode_id") || "unknown";
    return [
      {
        type: "upsert",
        node: {
          id: `mode:${context.taskId}`,
          kind: "mode",
          taskId: context.taskId,
          lane: context.lane,
          acpSessionId: context.acpSessionId,
          provider: context.provider,
          source: "acp-live",
          status: "completed",
          updatedAt: context.now(),
          raw: update,
          modeId,
          availableModes
        }
      }
    ];
  }
};

const usageRenderer: AcpUpdateRenderer<UsageUpdate> = {
  match: (update): update is UsageUpdate =>
    update.sessionUpdate === "usage_update",
  reduce(update, context) {
    const record = update as unknown as Record<string, unknown>;
    return [
      {
        type: "upsert",
        node: {
          id: `usage:${context.taskId}`,
          kind: "usage",
          taskId: context.taskId,
          lane: context.lane,
          acpSessionId: context.acpSessionId,
          provider: context.provider,
          source: "acp-live",
          status: "completed",
          updatedAt: context.now(),
          raw: update,
          used: usageMetric(record, ["used", "usedTokens", "used_tokens", "tokensUsed", "tokens_used", "totalTokens", "total_tokens"], update.used),
          size: usageMetric(record, ["size", "contextSize", "context_size", "contextWindow", "context_window", "limit", "maxTokens", "max_tokens"], update.size),
          cost: typeof update.cost === "object" && !Array.isArray(update.cost) ? update.cost : undefined
        }
      }
    ];
  }
};

const configRenderer: AcpUpdateRenderer<ConfigOptionUpdate> = {
  match: (update): update is ConfigOptionUpdate =>
    update.sessionUpdate === "config_option_update",
  reduce(update, context) {
    const record = update as unknown as Record<string, unknown>;
    const configOptions = normalizeSessionConfigOptions(firstArrayField(record, "configOptions", "config_options"));
    return [
      {
        type: "upsert",
        node: {
          id: `config:${context.taskId}`,
          kind: "config",
          taskId: context.taskId,
          lane: context.lane,
          acpSessionId: context.acpSessionId,
          provider: context.provider,
          source: "acp-live",
          status: "completed",
          updatedAt: context.now(),
          raw: update,
          configOptions
        }
      }
    ];
  }
};

const commandsRenderer: AcpUpdateRenderer<AvailableCommandsUpdate> = {
  match: (update): update is AvailableCommandsUpdate =>
    update.sessionUpdate === "available_commands_update",
  reduce(update, context) {
    const record = update as unknown as Record<string, unknown>;
    const availableCommands = normalizeAvailableCommands(record);
    return [
      {
        type: "upsert",
        node: {
          id: `commands:${context.taskId}`,
          kind: "commands",
          taskId: context.taskId,
          lane: context.lane,
          acpSessionId: context.acpSessionId,
          provider: context.provider,
          source: "acp-live",
          status: "completed",
          updatedAt: context.now(),
          raw: update,
          availableCommands
        }
      }
    ];
  }
};

const sessionInfoRenderer: AcpUpdateRenderer<SessionInfoUpdate> = {
  match: (update): update is SessionInfoUpdate =>
    update.sessionUpdate === "session_info_update",
  reduce(update, context) {
    const record = update as unknown as Record<string, unknown>;
    return [
      {
        type: "upsert",
        node: {
          id: `session:${context.taskId}`,
          kind: "session",
          taskId: context.taskId,
          lane: context.lane,
          acpSessionId: context.acpSessionId,
          provider: context.provider,
          source: "acp-live",
          status: "completed",
          updatedAt: context.now(),
          raw: update,
          title: stringField(record, "title") || stringField(record, "name"),
          sessionUpdatedAt: stringField(record, "updatedAt") || stringField(record, "updated_at")
        }
      }
    ];
  }
};

export const updateRenderers: AcpUpdateRenderer[] = [
  userMessageRenderer,
  agentMessageRenderer,
  thoughtRenderer,
  planRenderer,
  toolCallRenderer,
  toolCallPatchRenderer,
  modeRenderer,
  usageRenderer,
  configRenderer,
  commandsRenderer,
  sessionInfoRenderer
];

function messagePatch(
  role: "user" | "assistant",
  messageId: string | undefined,
  content: unknown,
  context: RenderReduceContext,
  streaming: boolean,
  raw: unknown = content
): RenderPatch {
  const id = `message:${role}:${messageId || "anonymous"}`;
  const contentBlocks = streamContentBlocks(content);
  const node: MessageNode = {
    id,
    kind: "message",
    taskId: context.taskId,
    lane: context.lane,
    turnId: context.currentTurnId,
    acpSessionId: context.acpSessionId,
    acpMessageId: messageId,
    provider: context.provider,
    source: "acp-live",
    status: streaming ? "in_progress" : "completed",
    createdAt: context.now(),
    updatedAt: context.now(),
    raw,
    role,
    content: contentBlocks,
    text: contentBlocksToText(contentBlocks),
    streaming
  };
  return { type: "upsert", node };
}

function messageChunkId(update: unknown): string | undefined {
  const record = asPlainRecord(update);
  return messageChunkIdFromRecord(record) || messageChunkIdFromRecord(asPlainRecord(record.message));
}

function messageChunkIdFromRecord(record: Record<string, unknown>): string | undefined {
  return recordIdField(record, "messageId") || recordIdField(record, "message_id") || recordIdField(record, "id");
}

function recordIdField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  const direct = recordIdValue(value);
  if (direct !== undefined) {
    return direct;
  }
  const nested = asPlainRecord(value);
  return recordIdValue(nested.id) || recordIdValue(nested["0"]);
}

function recordIdValue(value: unknown): string | undefined {
  if (typeof value === "string" && value) {
    return value;
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    return String(value);
  }
  return undefined;
}

function messageChunkContent(update: unknown): unknown {
  return messageChunkContentFromRecord(asPlainRecord(update));
}

function messageChunkContentFromRecord(record: Record<string, unknown>): unknown {
  const content = renderableMessageChunkContent(record.content);
  if (content !== undefined) {
    return content;
  }
  const nestedMessage = asPlainRecord(record.message);
  if (Object.keys(nestedMessage).length) {
    const nestedContent = messageChunkContentFromRecord(nestedMessage);
    if (nestedContent !== undefined) {
      return nestedContent;
    }
  }
  for (const key of [
    "delta",
    "content_delta",
    "contentDelta",
    "chunk",
    "part",
    "text",
    "body",
    "content_text",
    "contentText",
    "value",
    "message"
  ]) {
    const aliasContent = renderableMessageChunkContent(record[key]);
    if (aliasContent !== undefined) {
      return aliasContent;
    }
  }
  return undefined;
}

function renderableMessageChunkContent(value: unknown): unknown {
  if (value === undefined) {
    return undefined;
  }
  return streamContentBlocks(value).length ? value : undefined;
}

function toolNodeFromCall(call: ToolCallUpdate, context: RenderReduceContext): ToolNode {
  const timestamp = context.now();
  const content = normalizedToolContent(call);
  const locations = normalizedToolLocations(call);
  const status = toolCallStatusFromRecord(call as unknown as Record<string, unknown>) || mapToolStatus(call.status);
  const toolCallId = toolCallIdFromCall(call);
  const rawInput = rawInputFromCall(call);
  const rawOutput = rawOutputFromCall(call);
  return syncToolTerminalContent({
    id: `tool:${toolCallId}`,
    kind: "tool",
    taskId: context.taskId,
    lane: context.lane,
    turnId: context.currentTurnId,
    acpSessionId: context.acpSessionId,
    acpToolCallId: toolCallId,
    provider: context.provider,
    source: "acp-live",
    status,
    createdAt: timestamp,
    updatedAt: timestamp,
    raw: call,
    toolCallId,
    title: call.title,
    toolKind: call.kind || "other",
    toolStatus: status,
    locations,
    content,
    rawInput,
    rawOutput
  });
}

function normalizedToolContent(call: ToolCallUpdate | ToolCallPatchUpdate): ToolCallContent[] {
  const callRecord = call as unknown as Record<string, unknown>;
  const content = toolContentArray(callRecord.content).filter(isRenderableToolContent);
  if (content.length) {
    return content;
  }
  const output = recoveredRawToolOutput(rawOutputFromCall(call));
  if (!output) {
    return [];
  }
  if (isCommandToolCall(call)) {
    const command = commandField(rawInputFromCall(call) || {}) || call.title || "Command";
    const toolCallId = toolCallIdFromCall(call);
    const terminal: ToolTerminalBlock = {
      type: "terminal",
      terminalId: toolCallId,
      command,
      stdout: output.text
    };
    if (call.title !== undefined) {
      terminal.title = call.title;
    }
    if (typeof output.exitCode === "number") {
      terminal.exitCode = output.exitCode;
    }
    if (output.stderr) {
      terminal.stderr = output.stderr;
    }
    return [terminal];
  }
  return [
    {
      type: "content",
      content: {
        type: "text",
        text: output.text
      }
    }
  ];
}

function toolCallIdFromCall(call: ToolCallUpdate | ToolCallPatchUpdate): string {
  const record = call as unknown as Record<string, unknown>;
  const nestedToolCall = asPlainRecord(record.toolCall);
  const nestedSnakeToolCall = asPlainRecord(record.tool_call);
  return (
    recordIdField(record, "toolCallId") ||
    recordIdField(record, "tool_call_id") ||
    recordIdField(nestedToolCall, "toolCallId") ||
    recordIdField(nestedToolCall, "tool_call_id") ||
    recordIdField(nestedToolCall, "id") ||
    recordIdField(nestedSnakeToolCall, "toolCallId") ||
    recordIdField(nestedSnakeToolCall, "tool_call_id") ||
    recordIdField(nestedSnakeToolCall, "id") ||
    recordIdField(record, "id") ||
    "unknown"
  );
}

function rawInputFromCall(call: ToolCallUpdate | ToolCallPatchUpdate): Record<string, unknown> | undefined {
  const record = call as unknown as Record<string, unknown>;
  return jsonObjectField(record, "rawInput") || jsonObjectField(record, "raw_input");
}

function rawOutputFromCall(call: ToolCallUpdate | ToolCallPatchUpdate): Record<string, unknown> | undefined {
  const record = call as unknown as Record<string, unknown>;
  return jsonObjectField(record, "rawOutput") || jsonObjectField(record, "raw_output");
}

function jsonObjectField(record: Record<string, unknown>, key: string): Record<string, unknown> | undefined {
  const value = asPlainRecord(record[key]);
  return Object.keys(value).length ? value : undefined;
}

function toolContentArray(value: unknown): ToolCallContent[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      const content = toolContentBlock(item);
      return content ? [content] : [];
    });
  }
  const content = toolContentBlock(value);
  return content ? [content] : [];
}

function toolContentBlock(value: unknown): ToolCallContent | undefined {
  if (typeof value === "string") {
    return {
      type: "content",
      content: {
        type: "text",
        text: value
      }
    };
  }
  const record = asPlainRecord(value);
  if (!Object.keys(record).length) {
    return undefined;
  }
  if (record.type === "terminal") {
    return normalizedTerminalToolContent(record);
  }
  if (record.type === "diff") {
    return normalizedDiffToolContent(record);
  }
  if (typeof record.type === "string") {
    return record as ToolCallContent;
  }
  const nestedContent = firstStreamContentBlock(record.content);
  if (nestedContent) {
    return {
      type: "content",
      content: nestedContent
    };
  }
  const text = stringField(record, "text") || stringField(record, "content") || stringField(record, "value");
  if (text !== undefined) {
    return {
      type: "content",
      content: {
        type: "text",
        text
      }
    };
  }
  return undefined;
}

function normalizedTerminalToolContent(record: Record<string, unknown>): ToolCallContent {
  const terminalId =
    recordIdField(record, "terminalId") ||
    recordIdField(record, "terminal_id") ||
    recordIdField(record, "id");
  return terminalId ? ({ ...record, type: "terminal", terminalId } as ToolCallContent) : (record as ToolCallContent);
}

function normalizedDiffToolContent(record: Record<string, unknown>): ToolCallContent {
  const next: Record<string, unknown> = { ...record, type: "diff" };
  const path = stringField(record, "path") || stringField(record, "file") || stringField(record, "filename");
  if (path !== undefined) {
    next.path = path;
  }
  const oldText = stringField(record, "oldText") ?? stringField(record, "old_text");
  if (oldText !== undefined) {
    next.oldText = oldText;
  }
  const newText = stringField(record, "newText") ?? stringField(record, "new_text");
  if (newText !== undefined) {
    next.newText = newText;
  }
  return next as ToolCallContent;
}

function firstStreamContentBlock(value: unknown): ContentBlock | undefined {
  return streamContentBlocks(value)[0];
}

function isRenderableToolContent(content: ToolCallContent): boolean {
  const record = content as Record<string, unknown>;
  if (record.type === "content") {
    return isRenderableContentBlock(record.content);
  }
  if (record.type === "terminal") {
    return [
      record.terminalId,
      record.title,
      record.name,
      record.command,
      record.commandLine,
      record.command_line,
      record.cwd,
      record.workingDirectory,
      record.working_directory,
      record.status,
      record.state,
      record.output,
      record.stdout,
      record.stderr,
      record.outputDelta,
      record.output_delta,
      record.stdoutDelta,
      record.stdout_delta,
      record.stderrDelta,
      record.stderr_delta,
      record.stdoutPreview,
      record.stdout_preview,
      record.stderrPreview,
      record.stderr_preview,
      record.exitCode,
      record.exit_code
    ].some(hasRenderableToolValue);
  }
  if (record.type === "diff") {
    return [record.path, record.oldText, record.newText].some(hasRenderableToolValue);
  }
  return Object.keys(record).some((key) => key !== "type" && hasRenderableToolValue(record[key]));
}

function isRenderableContentBlock(value: unknown): boolean {
  if (typeof value === "string") {
    return value.length > 0;
  }
  const record = asPlainRecord(value);
  if (!Object.keys(record).length) {
    return false;
  }
  if (record.type === "text") {
    return ["text", "content", "value"].some((key) => hasRenderableToolValue(record[key]));
  }
  return typeof record.type === "string";
}

function hasRenderableToolValue(value: unknown): boolean {
  if (typeof value === "string") {
    return value.length > 0;
  }
  if (typeof value === "number") {
    return Number.isFinite(value);
  }
  return Array.isArray(value) ? value.length > 0 : Boolean(value);
}

function normalizedToolLocations(call: ToolCallUpdate | ToolCallPatchUpdate): ToolCallLocation[] {
  const record = call as unknown as Record<string, unknown>;
  const locations = toolLocationArray(record.locations);
  if (locations.length) {
    return locations;
  }
  const location = toolLocationArray(record.location);
  if (location.length) {
    return location;
  }
  const inline = toolLocation(record);
  return inline ? [inline] : [];
}

function toolLocationArray(value: unknown): ToolCallLocation[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      const location = toolLocation(item);
      return location ? [location] : [];
    });
  }
  const location = toolLocation(value);
  return location ? [location] : [];
}

function toolLocation(value: unknown): ToolCallLocation | undefined {
  const record = asPlainRecord(value);
  const path = stringField(record, "path") || stringField(record, "file") || stringField(record, "filename");
  if (!path) {
    return undefined;
  }
  const line = numberField(record, "line") ?? numberField(record, "lineNumber") ?? numberField(record, "line_number");
  const location: ToolCallLocation = { ...record, path };
  if (line !== undefined) {
    location.line = line;
  }
  return location;
}

function recoveredRawToolOutput(rawOutput: Record<string, unknown> | undefined): { text: string; stderr?: string; exitCode?: number } | undefined {
  const root = asPlainRecord(rawOutput);
  const nested = asPlainRecord(root.output);
  const records = Object.keys(nested).length ? [nested, root] : [root];
  const formatted = stringFromRecords(records, ["formatted_output", "formattedOutput", "output", "stdout", "stdoutPreview", "stdout_preview", "text"]);
  const stderr = cleanRecoveredOutput(stringFromRecords(records, ["stderr", "stderrPreview", "stderr_preview", "error"]) || "");
  const text = cleanRecoveredOutput(formatted || stderr);
  if (!text) {
    return undefined;
  }
  const recovered: { text: string; stderr?: string; exitCode?: number } = { text };
  if (stderr) {
    recovered.stderr = stderr;
  }
  const exitCode = numberFromRecords(records, ["exit_code", "exitCode"]);
  if (typeof exitCode === "number") {
    recovered.exitCode = exitCode;
  }
  return recovered;
}

function isCommandToolCall(call: ToolCallUpdate | ToolCallPatchUpdate): boolean {
  if (call.kind === "execute") {
    return true;
  }
  if (commandField(rawInputFromCall(call) || {})) {
    return true;
  }
  const title = typeof call.title === "string" ? call.title : "";
  return /^(bash|shell|terminal|execute|command|run)$/i.test(title.trim());
}

function expandToolContent(tool: ToolNode, context: RenderReduceContext): RenderPatch[] {
  const patches: RenderPatch[] = [{ type: "upsert", node: tool }];
  for (const item of tool.content) {
    const record = item as Record<string, unknown>;
    if (item.type === "diff") {
      const path = stringField(record, "path") || stringField(record, "file") || stringField(record, "filename") || "unknown";
      const newText = stringField(record, "newText") ?? stringField(record, "new_text") ?? "";
      const oldText = stringField(record, "oldText") ?? stringField(record, "old_text") ?? null;
      patches.push({
        type: "upsert",
        node: {
          id: `diff:${tool.toolCallId}:${path}`,
          kind: "diff",
          taskId: context.taskId,
          lane: context.lane,
          turnId: context.currentTurnId,
          acpSessionId: context.acpSessionId,
          acpToolCallId: tool.toolCallId,
          provider: context.provider,
          source: "acp-live",
          status: tool.status,
          createdAt: context.now(),
          updatedAt: context.now(),
          raw: item,
          path,
          oldText,
          newText
        }
      });
    } else if (item.type === "terminal") {
      const terminalId =
        recordIdField(record, "terminalId") ||
        recordIdField(record, "terminal_id") ||
        recordIdField(record, "id") ||
        "unknown";
      const terminal = terminalDetails(record);
      patches.push({
        type: "upsert",
        node: {
          id: terminalNodeId(tool.toolCallId, terminalId),
          kind: "terminal",
          taskId: context.taskId,
          lane: context.lane,
          turnId: context.currentTurnId,
          acpSessionId: context.acpSessionId,
          acpToolCallId: tool.toolCallId,
          provider: context.provider,
          source: "acp-live",
          status: tool.status,
          createdAt: context.now(),
          updatedAt: context.now(),
          raw: item,
          terminalId,
          title: terminal.title || tool.title,
          command: terminal.command,
          cwd: terminal.cwd,
          terminalStatus: terminal.status || tool.toolStatus,
          exitCode: terminal.exitCode,
          elapsedMs: terminal.elapsedMs,
          output: terminal.output,
          stdout: terminal.stdout,
          stderr: terminal.stderr
        }
      });
    }
  }
  return patches;
}

function terminalNodeId(toolCallId: string, terminalId: string): string {
  return `terminal:${toolCallId}:${terminalId}`;
}

type ExpandedToolChildNode = DiffNode | TerminalNode;

function syncExpandedToolChildNodesAndCollect(nodes: RenderNode[], tool: ToolNode): AppliedRenderPatches {
  let changed = false;
  const patches: RenderPatch[] = [];
  const next = nodes.map((node) => {
    if (!isExpandedToolChildNode(node) || !childBelongsToToolScope(node, tool)) {
      return node;
    }
    const updated = syncExpandedToolChildNode(node, tool);
    if (updated === node) {
      return node;
    }
    changed = true;
    patches.push({ type: "upsert", node: updated });
    return updated;
  });
  return { nodes: changed ? next : nodes, patches };
}

function isExpandedToolChildNode(node: RenderNode): node is ExpandedToolChildNode {
  return node.kind === "diff" || node.kind === "terminal";
}

function syncExpandedToolChildNode(node: ExpandedToolChildNode, tool: ToolNode): ExpandedToolChildNode {
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

function childBelongsToToolScope(child: ExpandedToolChildNode, tool: ToolNode): boolean {
  if (!childMatchesToolCallId(child, tool) || !sameTimelineScope(child, tool)) {
    return false;
  }
  const suffix = scopedToolNodeSuffix(tool);
  return suffix ? child.id.endsWith(`:${suffix}`) : true;
}

function childMatchesToolCallId(child: ExpandedToolChildNode, tool: ToolNode): boolean {
  return child.acpToolCallId === tool.toolCallId || child.acpToolCallId === tool.acpToolCallId;
}

function scopedToolNodeSuffix(tool: ToolNode): string | undefined {
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

function mergeTerminalNode(existing: TerminalNode, incoming: TerminalNode): TerminalNode {
  const preserveFinalStatus = isFinalRenderStatus(existing.status) && isActiveRenderStatus(incoming.status);
  const merged: TerminalNode = {
    ...incoming,
    createdAt: existing.createdAt,
    timelineOrder: existing.timelineOrder,
    status: preserveFinalStatus ? existing.status : incoming.status,
    terminalStatus: terminalStatusForMerge(existing, incoming, preserveFinalStatus),
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
  const sourceRecord = asPlainRecord(source);
  const rawRecord = asPlainRecord(sourceRecord.raw);
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

function terminalStatusForMerge(
  existing: TerminalNode,
  incoming: TerminalNode,
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

function isActiveRenderStatus(status: string | undefined): boolean {
  return status === "pending" || status === "in_progress";
}

function isFinalRenderStatus(status: string | undefined): boolean {
  return status === "completed" || status === "failed" || status === "cancelled";
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

function terminalDetails(record: Record<string, unknown>): {
  title?: string | undefined;
  command?: string | undefined;
  cwd?: string | undefined;
  status?: string | undefined;
  exitCode?: number | undefined;
  elapsedMs?: number | undefined;
  output?: string | undefined;
  stdout?: string | undefined;
  stderr?: string | undefined;
} {
  return {
    title: stringField(record, "title") || stringField(record, "name"),
    command: commandField(record),
    cwd: stringField(record, "cwd") || stringField(record, "workingDirectory") || stringField(record, "working_directory"),
    status: stringField(record, "status") || stringField(record, "state"),
    exitCode: numberField(record, "exitCode") ?? numberField(record, "exit_code"),
    elapsedMs: numberField(record, "elapsedMs") ?? numberField(record, "elapsed_ms") ?? numberField(record, "durationMs"),
    output: stringField(record, "output") || stringField(record, "outputDelta") || stringField(record, "output_delta"),
    stdout: stringField(record, "stdout") || stringField(record, "stdoutDelta") || stringField(record, "stdout_delta") || stringField(record, "stdoutPreview") || stringField(record, "stdout_preview"),
    stderr: stringField(record, "stderr") || stringField(record, "stderrDelta") || stringField(record, "stderr_delta") || stringField(record, "stderrPreview") || stringField(record, "stderr_preview")
  };
}

function commandField(record: Record<string, unknown>): string | undefined {
  const value = record.command;
  if (typeof value === "string") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((part) => String(part)).join(" ");
  }
  return stringField(record, "commandLine") || stringField(record, "command_line");
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function firstArrayField(record: Record<string, unknown>, ...keys: string[]): unknown[] {
  for (const key of keys) {
    const value = record[key];
    if (Array.isArray(value)) {
      return value;
    }
  }
  return [];
}

function normalizeAvailableCommands(record: Record<string, unknown>): AvailableCommand[] {
  const commands = firstArrayField(record, "availableCommands", "available_commands", "commands").filter(isAvailableCommand);
  if (commands.length) {
    return commands;
  }
  return firstArrayField(record, "commandNames", "command_names")
    .filter((name): name is string => typeof name === "string" && name.length > 0)
    .map((name) => ({ name, description: "" }));
}

function normalizeSessionConfigOptions(values: unknown[]): SessionConfigOption[] {
  return values.flatMap((value) => {
    if (!isSessionConfigOption(value)) {
      return [];
    }
    const record = value as Record<string, unknown>;
    const currentValue = configOptionScalarField(record, "current_value");
    if (value.currentValue !== undefined || currentValue === undefined) {
      return [value];
    }
    return [{ ...value, currentValue }];
  });
}

function configOptionScalarField(record: Record<string, unknown>, key: string): string | number | boolean | null | undefined {
  const value = record[key];
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return value;
  }
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function normalizePlanEntries(record: Record<string, unknown>): PlanUpdate["entries"] {
  return firstArrayField(record, "entries", "planEntries", "plan_entries", "items", "steps").flatMap((entry) => {
    if (typeof entry === "string" && entry.trim()) {
      return [{ title: entry }];
    }
    const entryRecord = asPlainRecord(entry);
    if (!Object.keys(entryRecord).length) {
      return [];
    }
    const normalized: Record<string, unknown> = { ...entryRecord };
    const title = stringField(entryRecord, "title") || stringField(entryRecord, "name") || stringField(entryRecord, "label");
    if (title) {
      normalized.title = title;
    }
    const content = stringField(entryRecord, "content") || stringField(entryRecord, "text") || stringField(entryRecord, "description");
    if (content) {
      normalized.content = content;
    }
    const status = stringField(entryRecord, "status") || stringField(entryRecord, "state");
    if (status) {
      normalized.status = status;
    }
    return [normalized as PlanUpdate["entries"][number]];
  });
}

function usageMetric(record: Record<string, unknown>, keys: string[], fallback: unknown): number {
  return numberFromRecords([record, asPlainRecord(record.usage)], keys) ?? (typeof fallback === "number" && Number.isFinite(fallback) ? fallback : 0);
}

function numberField(record: Record<string, unknown>, key: string): number | undefined {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function stringFromRecords(records: Record<string, unknown>[], keys: string[]): string | undefined {
  for (const record of records) {
    for (const key of keys) {
      const value = record[key];
      if (typeof value === "string" && value) {
        return value;
      }
    }
  }
  return undefined;
}

function numberFromRecords(records: Record<string, unknown>[], keys: string[]): number | undefined {
  for (const record of records) {
    for (const key of keys) {
      const value = numberField(record, key);
      if (typeof value === "number") {
        return value;
      }
    }
  }
  return undefined;
}

function cleanRecoveredOutput(value: string): string {
  return redactString(value.replace(/\r\n/g, "\n").replace(/\r/g, "\n")).trimEnd();
}

function asPlainRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function toolCallStatusFromRecord(record: Record<string, unknown>): ToolCallStatus | undefined {
  return normalizedToolCallStatus(stringField(record, "status") || stringField(record, "state"));
}

function mapToolStatus(status: string | undefined): ToolCallStatus {
  return normalizedToolCallStatus(status) || "in_progress";
}

function normalizedToolCallStatus(status: string | null | undefined): ToolCallStatus | undefined {
  switch (normalizeStatus(status || undefined)) {
    case "pending":
      return "pending";
    case "active":
    case "in-progress":
    case "in_progress":
    case "running":
      return "in_progress";
    case "completed":
    case "succeeded":
    case "success":
    case "passed":
      return "completed";
    case "failed":
    case "error":
      return "failed";
    case "cancelled":
    case "canceled":
      return "cancelled";
    default:
      return undefined;
  }
}

function isAvailableCommand(value: unknown): value is AvailableCommand {
  const record = value as Record<string, unknown> | undefined;
  return Boolean(record && typeof record.name === "string" && typeof record.description === "string");
}

function isSessionConfigOption(value: unknown): value is SessionConfigOption {
  const record = value as Record<string, unknown> | undefined;
  return Boolean(record && typeof record.id === "string" && typeof record.name === "string" && typeof record.type === "string");
}

function isSessionMode(value: unknown): value is SessionMode {
  const record = value as Record<string, unknown> | undefined;
  return Boolean(record && typeof record.id === "string" && typeof record.name === "string");
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function upsertUnknown(
  payload: unknown,
  context: RenderReduceContext,
  label: string
): RenderPatch {
  return {
    type: "upsert",
    node: {
      id: `unknown:${context.currentTurnId || context.taskId}:${stablePayloadKey(payload)}`,
      kind: "unknown",
      taskId: context.taskId,
      lane: context.lane,
      turnId: context.currentTurnId,
      acpSessionId: context.acpSessionId,
      provider: context.provider,
      source: "acp-live",
      status: "completed",
      updatedAt: context.now(),
      raw: payload,
      label,
      payload
    }
  };
}

function stablePayloadKey(payload: unknown): string {
  const text = safeStringify(payload);
  let hash = 0;
  for (let index = 0; index < text.length; index += 1) {
    hash = (hash * 31 + text.charCodeAt(index)) >>> 0;
  }
  return hash.toString(16);
}

function safeStringify(payload: unknown): string {
  try {
    return JSON.stringify(payload);
  } catch {
    return String(payload);
  }
}

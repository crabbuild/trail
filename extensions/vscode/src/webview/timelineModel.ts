import type { ContentBlock, ToolCallContent } from "../shared/acpTypes";
import type { RenderNode, ToolPermissionRequest } from "../shared/renderModel";
import type { RenderPatchChanges } from "./renderPatchModel";
import { buildToolPresentation } from "./toolModel";

export const TIMELINE_FILTERS = [
  { id: "all", label: "All" },
  { id: "chat", label: "Chat" },
  { id: "tools", label: "Tools" },
  { id: "diffs", label: "Diffs" },
  { id: "approvals", label: "Approvals" },
  { id: "events", label: "Events" }
] as const;

export type TimelineFilter = (typeof TIMELINE_FILTERS)[number]["id"];

export type TimelineCounts = Record<TimelineFilter, number>;

export type ToolActivityTone = "empty" | "ok" | "warning" | "risk" | "active";

export interface ToolActivityMetric {
  label: string;
  value: string;
  tone: Exclude<ToolActivityTone, "empty">;
}

export interface ToolActivityPath {
  path: string;
  count: number;
  detail: string;
  tone: Exclude<ToolActivityTone, "empty" | "active">;
}

export interface ToolActivitySummary {
  total: number;
  label: string;
  detail: string;
  tone: ToolActivityTone;
  metrics: ToolActivityMetric[];
  paths: ToolActivityPath[];
}

export interface TimelineConversationGroup {
  key: string;
  baseKey: string;
  turnId?: string | undefined;
  sessionId?: string | undefined;
  lane: string;
  status: RenderNode["status"];
  index: number;
  nodes: RenderNode[];
}

export function isTimelineFilter(value: unknown): value is TimelineFilter {
  return typeof value === "string" && TIMELINE_FILTERS.some((filter) => filter.id === value);
}

export function filterTimelineNodes(nodes: RenderNode[], filter: TimelineFilter, query: string): RenderNode[] {
  const tokens = timelineSearchTokens(query);
  return nodes.filter((node) => {
    if (!nodeMatchesTimelineFilter(node, filter)) {
      return false;
    }
    if (!tokens.length) {
      return true;
    }
    const searchable = normalizeTimelineSearchText(timelineNodeSearchText(node));
    return tokens.every((token) => searchable.includes(token));
  });
}

export function transcriptTimelineNodes(nodes: RenderNode[]): RenderNode[] {
  return nodes.filter(isTranscriptTimelineNode);
}

export function timelineDisplayNodes(nodes: RenderNode[]): RenderNode[] {
  const visible = transcriptTimelineNodes(nodes);
  const approvalsByTool = approvalsByScopedToolCallId(visible);
  const mergedApprovalIds = new Set([...approvalsByTool.values()].map((node) => node.id));
  const visibleToolKeys = new Set(
    visible
      .filter((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool")
      .flatMap(scopedToolCallKeysForTool)
  );
  return visible.flatMap<RenderNode>((node) => {
    if (node.kind === "tool") {
      const approval = approvalForTool(approvalsByTool, node);
      return [approval ? toolWithApproval(node, approval) : node];
    }
    if (node.kind === "approval") {
      const keys = scopedToolCallKeysForApproval(node);
      if (keys.some((key) => visibleToolKeys.has(key)) && mergedApprovalIds.has(node.id)) {
        return [];
      }
      return [approvalAsToolNode(node)];
    }
    return [node];
  });
}

export function sortTimelineNodes(nodes: RenderNode[]): RenderNode[] {
  return nodes
    .map((node, index) => ({ node, index }))
    .sort(
      (left, right) =>
        compareSortValue(sortOrder(left.node), sortOrder(right.node)) ||
        compareSortValue(sortTime(left.node), sortTime(right.node)) ||
        left.index - right.index
    )
    .map(({ node }) => node);
}

export function buildTimelineConversationGroups(nodes: RenderNode[]): TimelineConversationGroup[] {
  const groups: TimelineConversationGroup[] = [];
  const segmentCounts = new Map<string, number>();
  const groupByUserMessages = nodes.some(isTimelineUserMessageNode);
  let currentConversationBaseKey: string | undefined;
  let currentConversationTurnId: string | undefined;

  for (const node of nodes) {
    if (groupByUserMessages && isTimelineUserMessageNode(node)) {
      currentConversationTurnId = node.turnId || node.id;
      currentConversationBaseKey = `turn:${currentConversationTurnId}`;
    }
    const scopedGroup = timelineConversationGroupScope(
      node,
      groupByUserMessages,
      currentConversationBaseKey,
      currentConversationTurnId
    );
    let group = groups[groups.length - 1];
    if (group?.baseKey !== scopedGroup.baseKey) {
      const segmentCount = (segmentCounts.get(scopedGroup.baseKey) || 0) + 1;
      segmentCounts.set(scopedGroup.baseKey, segmentCount);
      const key = segmentCount === 1 ? scopedGroup.baseKey : `${scopedGroup.baseKey}:segment-${segmentCount}`;
      group = {
        key,
        baseKey: scopedGroup.baseKey,
        turnId: scopedGroup.turnId,
        sessionId: node.acpSessionId,
        lane: node.lane,
        status: node.status,
        index: groups.length,
        nodes: []
      };
      groups.push(group);
    }
    group.nodes.push(node);
    group.status = combinedTimelineGroupStatus(group.status, node.status);
    group.sessionId = group.sessionId || node.acpSessionId;
  }

  return groups;
}

function timelineConversationGroupScope(
  node: RenderNode,
  groupByUserMessages: boolean,
  currentConversationBaseKey: string | undefined,
  currentConversationTurnId: string | undefined
): { baseKey: string; turnId?: string | undefined } {
  if (groupByUserMessages) {
    return currentConversationBaseKey
      ? { baseKey: currentConversationBaseKey, turnId: currentConversationTurnId }
      : { baseKey: "session" };
  }
  return node.turnId ? { baseKey: `turn:${node.turnId}`, turnId: node.turnId } : { baseKey: "session" };
}

function isTimelineUserMessageNode(node: RenderNode): node is Extract<RenderNode, { kind: "message" }> {
  return node.kind === "message" && node.role === "user";
}

function combinedTimelineGroupStatus(
  current: RenderNode["status"],
  next: RenderNode["status"]
): RenderNode["status"] {
  const priority: Record<RenderNode["status"], number> = {
    failed: 5,
    cancelled: 4,
    pending: 3,
    in_progress: 3,
    completed: 1
  };
  return priority[next] > priority[current] ? next : current;
}

export function timelineDisplayPatchChanges(
  beforeNodes: RenderNode[],
  nextNodes: RenderNode[],
  changes: RenderPatchChanges
): RenderPatchChanges {
  const beforeDisplay = timelineDisplayNodes(beforeNodes);
  const nextDisplay = timelineDisplayNodes(nextNodes);
  const beforeDisplayIds = new Set(beforeDisplay.map((node) => node.id));
  const nextDisplayIds = new Set(nextDisplay.map((node) => node.id));
  const beforeById = new Map(beforeNodes.map((node) => [node.id, node]));
  const nextById = new Map(nextNodes.map((node) => [node.id, node]));
  const displayChanges: RenderPatchChanges = {
    changedNodeIds: new Set(),
    addedNodeIds: new Set(),
    removedNodeIds: new Set()
  };

  for (const id of changes.changedNodeIds) {
    addDisplayChangeTransition(
      displayChanges,
      displayIdsForRawNode(id, beforeById.get(id), beforeDisplay),
      displayIdsForRawNode(id, nextById.get(id), nextDisplay),
      beforeDisplayIds,
      nextDisplayIds
    );
  }
  for (const id of changes.addedNodeIds) {
    const rawNode = nextById.get(id);
    addDisplayChangeTransition(
      displayChanges,
      displayIdsForRawNode(id, rawNode, beforeDisplay),
      displayIdsForRawNode(id, rawNode, nextDisplay),
      beforeDisplayIds,
      nextDisplayIds
    );
  }
  for (const id of changes.removedNodeIds) {
    const rawNode = beforeById.get(id);
    addDisplayChangeTransition(
      displayChanges,
      displayIdsForRawNode(id, rawNode, beforeDisplay),
      displayIdsForRawNode(id, rawNode, nextDisplay),
      beforeDisplayIds,
      nextDisplayIds
    );
  }

  return displayChanges;
}

export function hasTimelineDisplayStructuralChange(
  beforeNodes: RenderNode[],
  nextNodes: RenderNode[],
  changes: RenderPatchChanges
): boolean {
  if (changes.addedNodeIds.size || changes.removedNodeIds.size) {
    return true;
  }
  const beforeDisplayById = new Map(timelineDisplayNodes(beforeNodes).map((node) => [node.id, node]));
  const nextDisplayById = new Map(timelineDisplayNodes(nextNodes).map((node) => [node.id, node]));
  for (const id of changes.changedNodeIds) {
    const before = beforeDisplayById.get(id);
    const next = nextDisplayById.get(id);
    if (!before || !next) {
      return true;
    }
    if (timelineDisplayStructureKey(before) !== timelineDisplayStructureKey(next)) {
      return true;
    }
  }
  return false;
}

function timelineDisplayStructureKey(node: RenderNode): string {
  return JSON.stringify({
    kind: node.kind,
    taskId: node.taskId,
    lane: node.lane,
    turnId: node.turnId,
    acpSessionId: node.acpSessionId,
    source: node.source,
    status: node.status,
    role: node.kind === "message" ? node.role : undefined,
    toolStatus: node.kind === "tool" ? node.toolStatus : undefined,
    permissionRequestId: node.kind === "tool" ? node.permission?.requestId : undefined,
    permissionStatus: node.kind === "tool" ? node.permission?.status : undefined,
    sortOrder: sortOrder(node),
    sortTime: sortTime(node)
  });
}

function addDisplayChangeTransition(
  changes: RenderPatchChanges,
  beforeIds: string[],
  nextIds: string[],
  beforeDisplayIds: Set<string>,
  nextDisplayIds: Set<string>
): void {
  const nextIdSet = new Set(nextIds);
  for (const id of nextIds) {
    if (beforeDisplayIds.has(id)) {
      changes.changedNodeIds.add(id);
    } else {
      changes.addedNodeIds.add(id);
    }
  }
  for (const id of beforeIds) {
    if (nextIdSet.has(id)) {
      continue;
    }
    if (nextDisplayIds.has(id)) {
      changes.changedNodeIds.add(id);
    } else {
      changes.removedNodeIds.add(id);
    }
  }
}

function displayIdsForRawNode(
  rawId: string,
  rawNode: RenderNode | undefined,
  displayNodes: RenderNode[]
): string[] {
  if (displayNodes.some((node) => node.id === rawId)) {
    return [rawId];
  }
  if (rawNode?.kind === "tool") {
    return displayNodes
      .filter((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool")
      .filter((node) => displayToolRepresentsRawTool(node, rawNode))
      .map((node) => node.id);
  }
  if (rawNode?.kind === "approval") {
    return displayNodes
      .filter((node): node is Extract<RenderNode, { kind: "tool" }> => node.kind === "tool")
      .filter((node) => displayToolCarriesApproval(node, rawNode))
      .map((node) => node.id);
  }
  return [];
}

function displayToolRepresentsRawTool(
  displayTool: Extract<RenderNode, { kind: "tool" }>,
  rawTool: Extract<RenderNode, { kind: "tool" }>
): boolean {
  return displayTool.id === rawTool.id || Boolean(displayTool.permission && hasOverlappingScopedToolCallKey(displayTool, rawTool));
}

function displayToolCarriesApproval(
  tool: Extract<RenderNode, { kind: "tool" }>,
  approval: Extract<RenderNode, { kind: "approval" }>
): boolean {
  return (
    tool.permission?.requestId === approval.requestId &&
    sameTimelineRenderScope(tool, approval) &&
    hasOverlappingScopedToolCallKey(tool, approval)
  );
}

function approvalsByScopedToolCallId(nodes: RenderNode[]): Map<string, Extract<RenderNode, { kind: "approval" }>> {
  const approvals = new Map<string, Extract<RenderNode, { kind: "approval" }>>();
  for (const node of nodes) {
    if (node.kind !== "approval") {
      continue;
    }
    for (const key of scopedToolCallKeysForApproval(node)) {
      const existing = approvals.get(key);
      if (!existing || shouldMergeApprovalIntoTool(node, existing)) {
        approvals.set(key, node);
      }
    }
  }
  return approvals;
}

function approvalForTool(
  approvalsByTool: Map<string, Extract<RenderNode, { kind: "approval" }>>,
  tool: Extract<RenderNode, { kind: "tool" }>
): Extract<RenderNode, { kind: "approval" }> | undefined {
  for (const key of scopedToolCallKeysForTool(tool)) {
    const approval = approvalsByTool.get(key);
    if (approval) {
      return approval;
    }
  }
  return undefined;
}

function shouldMergeApprovalIntoTool(
  candidate: Extract<RenderNode, { kind: "approval" }>,
  existing: Extract<RenderNode, { kind: "approval" }>
): boolean {
  const candidatePriority = approvalMergePriority(candidate);
  const existingPriority = approvalMergePriority(existing);
  if (candidatePriority !== existingPriority) {
    return candidatePriority > existingPriority;
  }
  const candidateOrder = sortOrder(candidate);
  const existingOrder = sortOrder(existing);
  if (Number.isFinite(candidateOrder) && Number.isFinite(existingOrder) && candidateOrder !== existingOrder) {
    return candidateOrder > existingOrder;
  }
  const candidateTime = sortTime(candidate);
  const existingTime = sortTime(existing);
  if (Number.isFinite(candidateTime) && Number.isFinite(existingTime) && candidateTime !== existingTime) {
    return candidateTime > existingTime;
  }
  return true;
}

function approvalMergePriority(node: Extract<RenderNode, { kind: "approval" }>): number {
  return node.status === "pending" || node.status === "in_progress" ? 2 : 1;
}

function scopedToolCallKeysForApproval(node: Extract<RenderNode, { kind: "approval" }>): string[] {
  return scopedToolCallKeys({
    taskId: node.taskId,
    lane: node.lane,
    turnId: node.turnId,
    acpSessionId: node.acpSessionId,
    source: node.source,
    toolCallIds: [node.acpToolCallId, node.tool.acpToolCallId, node.tool.toolCallId]
  });
}

function scopedToolCallKeysForTool(node: Extract<RenderNode, { kind: "tool" }>): string[] {
  return scopedToolCallKeys({
    taskId: node.taskId,
    lane: node.lane,
    turnId: node.turnId,
    acpSessionId: node.acpSessionId,
    source: node.source,
    toolCallIds: [node.toolCallId, node.acpToolCallId]
  });
}

function hasOverlappingScopedToolCallKey(
  left: Extract<RenderNode, { kind: "tool" }>,
  right: Extract<RenderNode, { kind: "tool" | "approval" }>
): boolean {
  if (!sameTimelineRenderScope(left, right)) {
    return false;
  }
  const rightKeys = new Set(right.kind === "approval" ? scopedToolCallKeysForApproval(right) : scopedToolCallKeysForTool(right));
  return scopedToolCallKeysForTool(left).some((key) => rightKeys.has(key));
}

function scopedToolCallKeys(node: {
  taskId: string;
  lane: string;
  turnId?: string | undefined;
  acpSessionId?: string | undefined;
  source: RenderNode["source"];
  toolCallIds: Array<string | undefined>;
}): string[] {
  return [
    ...new Set(
      node.toolCallIds
        .filter((toolCallId): toolCallId is string => typeof toolCallId === "string" && toolCallId.length > 0)
        .map((toolCallId) =>
          scopedToolCallKey({
            taskId: node.taskId,
            lane: node.lane,
            turnId: node.turnId,
            acpSessionId: node.acpSessionId,
            source: node.source,
            toolCallId
          })
        )
    )
  ];
}

function scopedToolCallKey(node: {
  taskId: string;
  lane: string;
  turnId?: string | undefined;
  acpSessionId?: string | undefined;
  source: RenderNode["source"];
  toolCallId: string;
}): string {
  return [
    node.taskId,
    node.lane,
    node.turnId || "",
    node.acpSessionId || "",
    node.source,
    node.toolCallId
  ].join("\u0000");
}

function sameTimelineRenderScope(
  left: Pick<RenderNode, "taskId" | "lane" | "turnId" | "acpSessionId" | "source">,
  right: Pick<RenderNode, "taskId" | "lane" | "turnId" | "acpSessionId" | "source">
): boolean {
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

function toolWithApproval(
  tool: Extract<RenderNode, { kind: "tool" }>,
  approval: Extract<RenderNode, { kind: "approval" }>
): Extract<RenderNode, { kind: "tool" }> {
  return {
    ...tool,
    timelineOrder: earliestTimelineOrder(tool.timelineOrder, approval.timelineOrder),
    createdAt: earliestTimestamp(tool.createdAt, approval.createdAt),
    permission: permissionFromApproval(approval)
  };
}

function earliestTimelineOrder(left: number | undefined, right: number | undefined): number | undefined {
  if (Number.isFinite(left) && Number.isFinite(right)) {
    return Math.min(left!, right!);
  }
  if (Number.isFinite(left)) {
    return left;
  }
  return Number.isFinite(right) ? right : undefined;
}

function earliestTimestamp(left: string | undefined, right: string | undefined): string | undefined {
  const leftTime = timestampMillis(left);
  const rightTime = timestampMillis(right);
  if (leftTime !== undefined && rightTime !== undefined) {
    return leftTime <= rightTime ? left : right;
  }
  if (leftTime !== undefined) {
    return left;
  }
  if (rightTime !== undefined) {
    return right;
  }
  return left || right;
}

function timestampMillis(value: string | undefined): number | undefined {
  if (!value) {
    return undefined;
  }
  const time = Date.parse(value);
  return Number.isNaN(time) ? undefined : time;
}

function approvalAsToolNode(node: Extract<RenderNode, { kind: "approval" }>): Extract<RenderNode, { kind: "tool" }> {
  const permission = permissionFromApproval(node);
  const toolStatus =
    node.status === "completed" || node.status === "cancelled" || node.status === "failed"
      ? node.status
      : node.tool.toolStatus;
  return {
    ...node.tool,
    id: node.id,
    taskId: node.taskId,
    lane: node.lane,
    turnId: node.turnId,
    acpSessionId: node.acpSessionId,
    acpToolCallId: node.acpToolCallId || node.tool.acpToolCallId,
    provider: node.provider,
    source: node.source,
    status: node.status,
    timelineOrder: node.timelineOrder ?? node.tool.timelineOrder,
    createdAt: node.createdAt || node.tool.createdAt,
    updatedAt: node.updatedAt,
    toolStatus,
    permission
  };
}

export function isInlineToolDiffNode(nodes: RenderNode[], node: RenderNode): boolean {
  if (node.kind !== "diff" || !node.acpToolCallId) {
    return false;
  }
  const nodePath = comparableDiffPath(node.path);
  return nodes.some((candidate) => {
    if (candidate.kind !== "tool" || !sameTimelineRenderScope(candidate, node)) {
      return false;
    }
    const toolCallIds = new Set([candidate.toolCallId, candidate.acpToolCallId].filter(Boolean));
    if (!toolCallIds.has(node.acpToolCallId)) {
      return false;
    }
    return candidate.content.some((item) => {
      const record = item as Record<string, unknown>;
      if (record.type !== "diff") {
        return false;
      }
      const recordPath = comparableDiffPath(String(record.path || "unknown"));
      return recordPath === nodePath || recordPath.endsWith(`/${nodePath}`) || nodePath.endsWith(`/${recordPath}`);
    });
  });
}

function comparableDiffPath(path: string): string {
  return path.replace(/^file:\/\//i, "").replace(/\\/g, "/").replace(/^\/+/, "").toLowerCase();
}

function permissionFromApproval(node: Extract<RenderNode, { kind: "approval" }>): ToolPermissionRequest {
  const permission: ToolPermissionRequest = {
    requestId: node.requestId,
    title: node.title,
    status: node.status,
    options: node.options,
    raw: node.raw
  };
  if (node.provider) {
    permission.provider = node.provider;
  }
  if (node.createdAt) {
    permission.createdAt = node.createdAt;
  }
  if (node.updatedAt) {
    permission.updatedAt = node.updatedAt;
  }
  return permission;
}

function sortOrder(node: RenderNode): number {
  return Number.isFinite(node.timelineOrder) ? node.timelineOrder! : Infinity;
}

function sortTime(node: RenderNode): number {
  for (const value of [node.createdAt, node.updatedAt]) {
    if (!value) {
      continue;
    }
    const time = Date.parse(value);
    if (!Number.isNaN(time)) {
      return time;
    }
  }
  return Infinity;
}

function compareSortValue(left: number, right: number): number {
  const leftFinite = Number.isFinite(left);
  const rightFinite = Number.isFinite(right);
  if (leftFinite && rightFinite) {
    return left - right;
  }
  if (leftFinite) {
    return -1;
  }
  if (rightFinite) {
    return 1;
  }
  return 0;
}

function isTranscriptTimelineNode(node: RenderNode): boolean {
  switch (node.kind) {
    case "commands":
    case "config":
    case "mode":
    case "session":
    case "usage":
      return false;
    case "tool":
      return !isRoutineInternalTool(node);
    case "unknown":
      return !isRoutineInternalUnknown(node);
    default:
      return true;
  }
}

export function timelineSearchTokens(query: string): string[] {
  return normalizeTimelineSearchText(query).split(" ").filter(Boolean);
}

export function timelineFilterCounts(nodes: RenderNode[]): TimelineCounts {
  const counts: TimelineCounts = {
    all: nodes.length,
    chat: 0,
    tools: 0,
    diffs: 0,
    approvals: 0,
    events: 0
  };
  for (const node of nodes) {
    const bucket = timelineNodeBucket(node);
    counts[bucket] += 1;
    if (node.kind === "tool" && node.permission) {
      counts.approvals += 1;
    }
  }
  return counts;
}

export function buildToolActivitySummary(nodes: RenderNode[], maxPaths = 5): ToolActivitySummary {
  const counts = {
    total: 0,
    readOnly: 0,
    changes: 0,
    commands: 0,
    approvals: 0,
    running: 0,
    failed: 0,
    warnings: 0,
    risks: 0
  };
  const paths = new Map<string, { count: number; tone: ToolActivityPath["tone"]; kinds: Set<string> }>();

  for (const node of nodes) {
    if (node.kind === "tool") {
      counts.total += 1;
      const model = buildToolPresentation({
        title: node.title,
        toolKind: node.toolKind,
        toolStatus: node.toolStatus,
        locations: node.locations,
        content: node.content,
        rawInput: node.rawInput,
        source: node.source
      });
      incrementOperationCounts(model.kind, counts);
      incrementRiskCounts(model.riskTone, counts);
      incrementStatusCounts(node.toolStatus, counts);
      for (const location of node.locations) {
        addActivityPath(paths, location.path, pathToneForTool(model.kind, model.riskTone), model.kind);
      }
      if (node.permission) {
        counts.approvals += 1;
        counts.risks += 1;
        incrementStatusCounts(node.permission.status, counts);
        for (const location of node.locations) {
          addActivityPath(paths, location.path, "risk", `approval ${model.kind}`);
        }
      }
      continue;
    }

    if (node.kind === "diff") {
      counts.total += 1;
      counts.changes += 1;
      counts.warnings += 1;
      incrementStatusCounts(node.status, counts);
      addActivityPath(paths, node.path, "warning", "diff");
      continue;
    }

    if (node.kind === "terminal") {
      counts.total += 1;
      counts.commands += 1;
      counts.warnings += 1;
      incrementStatusCounts(node.status, counts);
      continue;
    }

    if (node.kind === "approval") {
      counts.total += 1;
      counts.approvals += 1;
      counts.risks += 1;
      incrementStatusCounts(node.status, counts);
      for (const location of node.tool.locations) {
        addActivityPath(paths, location.path, "risk", `approval ${node.tool.toolKind}`);
      }
    }
  }

  const tone = toolActivityTone(counts);
  return {
    total: counts.total,
    label: toolActivityLabel(tone),
    detail: toolActivityDetail(counts),
    tone,
    metrics: toolActivityMetrics(counts),
    paths: toolActivityPaths(paths, maxPaths)
  };
}

export function nodeMatchesTimelineFilter(node: RenderNode, filter: TimelineFilter): boolean {
  if (filter === "all") {
    return true;
  }
  if (filter === "approvals" && node.kind === "tool" && node.permission) {
    return true;
  }
  return timelineNodeBucket(node) === filter;
}

export function timelineNodeBucket(node: RenderNode): Exclude<TimelineFilter, "all"> {
  switch (node.kind) {
    case "message":
    case "thought":
    case "plan":
      return "chat";
    case "tool":
    case "terminal":
    case "resource":
      return "tools";
    case "diff":
      return "diffs";
    case "approval":
      return "approvals";
    default:
      return "events";
  }
}

export function timelineNodeSearchText(node: RenderNode): string {
  const parts: Array<string | null | undefined> = [node.kind, node.status, node.provider, node.lane, node.turnId, node.acpSessionId];
  switch (node.kind) {
    case "message":
      parts.push(node.role, node.text, ...contentBlocksText(node.content));
      break;
    case "thought":
      parts.push(...contentBlocksText(node.content));
      break;
    case "plan":
      parts.push(...node.entries.flatMap((entry) => [entry.title, entry.content, entry.status, entry.priority]));
      break;
    case "tool":
      parts.push(
        node.title,
        node.toolKind,
        node.toolStatus,
        node.permission?.title,
        node.permission?.requestId,
        node.permission?.status,
        ...node.locations.map((location) => [location.path, String(location.line || "")]).flat(),
        ...node.content.flatMap(toolContentText),
        ...(node.permission?.options || []).flatMap((option) => [option.label, option.description, option.optionId])
      );
      break;
    case "diff":
      parts.push(node.path, node.oldText || "", node.newText);
      break;
    case "terminal":
      parts.push(node.title, node.command, node.cwd, node.terminalStatus, node.output, node.stdout, node.stderr);
      break;
    case "approval":
      parts.push(
        node.title,
        node.requestId,
        node.tool.title,
        node.tool.toolKind,
        ...node.tool.locations.map((location) => [location.path, String(location.line || "")]).flat(),
        ...node.options.flatMap((option) => [option.label, option.description, option.optionId])
      );
      break;
    case "checkpoint":
      parts.push(node.label, node.checkpointId);
      break;
    case "completion":
      parts.push(node.label, node.stopReason);
      break;
    case "usage":
      parts.push(String(node.used), String(node.size));
      break;
    case "mode":
      parts.push(node.modeId, ...node.availableModes.flatMap((mode) => [mode.id, mode.name, mode.description]));
      break;
    case "config":
      parts.push(
        ...node.configOptions.flatMap((option) => [
          option.id,
          option.name,
          option.description,
          option.category,
          String(option.currentValue || "")
        ])
      );
      break;
    case "commands":
      parts.push(...node.availableCommands.flatMap((command) => [command.name, command.description, command.input?.hint]));
      break;
    case "session":
      parts.push(node.title || "", node.sessionUpdatedAt || "");
      break;
    case "resource":
      parts.push(...contentBlocksText([node.content]));
      break;
    case "unknown":
      parts.push(node.label);
      break;
  }
  return parts.filter((part): part is string => typeof part === "string" && part.length > 0).join(" ");
}

function contentBlocksText(blocks: ContentBlock[]): string[] {
  return blocks.flatMap((block) => {
    const record = asRecord(block);
    const resource = asRecord(record.resource);
    return [
      record.type,
      record.text,
      record.uri,
      record.name,
      record.title,
      record.description,
      resource.uri,
      resource.mimeType,
      resource.text
    ].filter((part): part is string => typeof part === "string" && part.length > 0);
  });
}

function normalizeTimelineSearchText(value: string): string {
  return value.toLowerCase().replace(/\s+/g, " ").trim();
}

function isRoutineInternalTool(node: Extract<RenderNode, { kind: "tool" }>): boolean {
  if (node.toolKind !== "other" || node.locations.length || node.content.length) {
    return false;
  }
  const title = normalizeTimelineSearchText(node.title);
  return title === "acp prompt turn" || title.startsWith("acp prompt turn (") || title.startsWith("span_started") || title.startsWith("span_ended");
}

function isRoutineInternalUnknown(node: Extract<RenderNode, { kind: "unknown" }>): boolean {
  const label = normalizeTimelineSearchText(node.label);
  return label === "span_started" || label === "span_ended" || label.startsWith("span_started (") || label.startsWith("span_ended (");
}

function toolContentText(content: ToolCallContent): string[] {
  const record = asRecord(content);
  return [
    record.type,
    record.path,
    record.terminalId,
    record.title,
    record.name,
    terminalCommand(record),
    record.cwd,
    record.workingDirectory,
    record.working_directory,
    record.status,
    record.state,
    record.output,
    record.stdout,
    record.stderr,
    ...contentBlocksText(record.type === "content" ? [record.content as ContentBlock] : [])
  ].filter((part): part is string => typeof part === "string" && part.length > 0);
}

function incrementOperationCounts(
  kind: string,
  counts: {
    readOnly: number;
    changes: number;
    commands: number;
  }
): void {
  switch (kind) {
    case "edit":
    case "delete":
    case "move":
      counts.changes += 1;
      return;
    case "execute":
      counts.commands += 1;
      return;
    default:
      counts.readOnly += 1;
  }
}

function incrementRiskCounts(
  tone: "ok" | "warning" | "risk",
  counts: {
    warnings: number;
    risks: number;
  }
): void {
  if (tone === "risk") {
    counts.risks += 1;
  } else if (tone === "warning") {
    counts.warnings += 1;
  }
}

function incrementStatusCounts(
  status: string,
  counts: {
    running: number;
    failed: number;
  }
): void {
  if (status === "pending" || status === "in_progress") {
    counts.running += 1;
  }
  if (status === "failed" || status === "cancelled") {
    counts.failed += 1;
  }
}

function pathToneForTool(kind: string, riskTone: "ok" | "warning" | "risk"): ToolActivityPath["tone"] {
  if (riskTone === "risk" || kind === "delete") {
    return "risk";
  }
  if (riskTone === "warning" || kind === "edit" || kind === "move") {
    return "warning";
  }
  return "ok";
}

function addActivityPath(
  paths: Map<string, { count: number; tone: ToolActivityPath["tone"]; kinds: Set<string> }>,
  path: string | undefined,
  tone: ToolActivityPath["tone"],
  kind: string
): void {
  const normalized = typeof path === "string" ? path.trim() : "";
  if (!normalized) {
    return;
  }
  const current = paths.get(normalized) || { count: 0, tone: "ok" as const, kinds: new Set<string>() };
  current.count += 1;
  current.tone = strongestPathTone(current.tone, tone);
  current.kinds.add(kind);
  paths.set(normalized, current);
}

function strongestPathTone(current: ToolActivityPath["tone"], next: ToolActivityPath["tone"]): ToolActivityPath["tone"] {
  const priority: Record<ToolActivityPath["tone"], number> = {
    ok: 1,
    warning: 2,
    risk: 3
  };
  return priority[next] > priority[current] ? next : current;
}

function toolActivityTone(counts: { total: number; failed: number; risks: number; running: number; warnings: number; changes: number; commands: number }): ToolActivityTone {
  if (!counts.total) {
    return "empty";
  }
  if (counts.failed || counts.risks) {
    return "risk";
  }
  if (counts.running) {
    return "active";
  }
  if (counts.warnings || counts.changes || counts.commands) {
    return "warning";
  }
  return "ok";
}

function toolActivityLabel(tone: ToolActivityTone): string {
  switch (tone) {
    case "empty":
      return "No visible tool activity";
    case "risk":
      return "Needs inspection";
    case "active":
      return "Agent is working";
    case "warning":
      return "Review tool activity";
    default:
      return "Read-only activity";
  }
}

function toolActivityDetail(counts: { total: number; readOnly: number; changes: number; commands: number; approvals: number; running: number; failed: number }): string {
  if (!counts.total) {
    return "The current transcript filter does not include tool, diff, terminal, or approval items.";
  }
  const parts = [
    activityCountLabel(counts.readOnly, "read-only"),
    activityCountLabel(counts.changes, "change"),
    activityCountLabel(counts.commands, "command"),
    activityCountLabel(counts.approvals, "approval"),
    activityCountLabel(counts.running, "running"),
    activityCountLabel(counts.failed, "failed")
  ].filter(Boolean);
  return parts.length ? parts.join(" / ") : `${counts.total} operation${counts.total === 1 ? "" : "s"}`;
}

function toolActivityMetrics(counts: {
  total: number;
  readOnly: number;
  changes: number;
  commands: number;
  approvals: number;
  running: number;
  failed: number;
}): ToolActivityMetric[] {
  if (!counts.total) {
    return [];
  }
  const metrics: ToolActivityMetric[] = [
    { label: "operations", value: formatCount(counts.total), tone: counts.failed ? "risk" : counts.running ? "active" : "ok" },
    { label: "read-only", value: formatCount(counts.readOnly), tone: "ok" },
    { label: "changes", value: formatCount(counts.changes), tone: counts.changes ? "warning" : "ok" },
    { label: "commands", value: formatCount(counts.commands), tone: counts.commands ? "warning" : "ok" },
    { label: "approvals", value: formatCount(counts.approvals), tone: counts.approvals ? "risk" : "ok" },
    { label: counts.failed ? "failed" : "running", value: formatCount(counts.failed || counts.running), tone: counts.failed ? "risk" : "active" }
  ];
  return metrics.filter((metric) => metric.value !== "0" || metric.label === "operations").slice(0, 6);
}

function toolActivityPaths(paths: Map<string, { count: number; tone: ToolActivityPath["tone"]; kinds: Set<string> }>, maxPaths: number): ToolActivityPath[] {
  return Array.from(paths.entries())
    .map(([path, summary]) => ({
      path,
      count: summary.count,
      detail: `${formatCount(summary.count)} ${summary.count === 1 ? "reference" : "references"} · ${Array.from(summary.kinds).slice(0, 3).join(", ")}`,
      tone: summary.tone
    }))
    .sort((left, right) => right.count - left.count || tonePriority(right.tone) - tonePriority(left.tone) || left.path.localeCompare(right.path))
    .slice(0, Math.max(0, maxPaths));
}

function tonePriority(tone: ToolActivityPath["tone"]): number {
  switch (tone) {
    case "risk":
      return 3;
    case "warning":
      return 2;
    default:
      return 1;
  }
}

function formatCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

function activityCountLabel(count: number, label: string): string {
  if (!count) {
    return "";
  }
  return `${formatCount(count)} ${label}${count === 1 || label === "read-only" || label === "running" ? "" : "s"}`;
}

function terminalCommand(record: Record<string, unknown>): string | undefined {
  const command = record.command || record.commandLine || record.command_line;
  if (Array.isArray(command)) {
    return command.map((part) => String(part)).join(" ");
  }
  return typeof command === "string" ? command : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

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
  ToolTerminalBlock,
  ToolCallPatchUpdate,
  ToolCallUpdate,
  UsageUpdate,
  UserMessageChunkUpdate
} from "./acpTypes";
import { redactString } from "./securityRedaction";
import type {
  ApprovalNode,
  MessageNode,
  RenderNode,
  RenderPatch,
  RenderReduceContext,
  TerminalNode,
  ThoughtNode,
  ToolNode
} from "./renderModel";

type AnonymousStreamNode = MessageNode | ThoughtNode;

export interface AcpUpdateRenderer<TUpdate extends SessionUpdate = SessionUpdate> {
  match(update: SessionUpdate): update is TUpdate;
  reduce(update: TUpdate, context: RenderReduceContext): RenderPatch[];
}

export function reduceSessionUpdate(
  update: SessionUpdate,
  context: RenderReduceContext
): RenderPatch[] {
  const renderer = updateRenderers.find((candidate) => candidate.match(update));
  if (!renderer) {
    return [upsertUnknown(update, context, `Unsupported ACP update: ${String(update.sessionUpdate)}`)];
  }
  return renderer.reduce(update, context);
}

export function reducePermissionRequest(
  requestId: string,
  params: RequestPermissionParams,
  context: RenderReduceContext
): RenderPatch[] {
  const tool = toolNodeFromCall(params.toolCall, context);
  const node: ApprovalNode = {
    id: `approval:${requestId}`,
    kind: "approval",
    taskId: context.taskId,
    lane: context.lane,
    acpSessionId: params.sessionId,
    turnId: context.currentTurnId,
    provider: context.provider,
    source: "acp-live",
    status: "pending",
    createdAt: context.now(),
    updatedAt: context.now(),
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
  let next = [...nodes];
  let nextTimelineOrder = maxTimelineOrder(next);
  for (const patch of patches) {
    const patchNode = patch.node ? normalizePatchNodeForTimeline(next, patch) : undefined;
    if (patch.type === "append" && patchNode) {
      const ordered = ensureTimelineOrder(patchNode, () => {
        nextTimelineOrder += 1;
        return nextTimelineOrder;
      });
      nextTimelineOrder = Math.max(nextTimelineOrder, ordered.timelineOrder ?? 0);
      next.push(ordered);
      continue;
    }
    if ((patch.type === "replace" || patch.type === "upsert") && patchNode) {
      const index = next.findIndex((node) => node.id === patchNode.id);
      let appliedNode: RenderNode | undefined;
      if (index >= 0) {
        const existing = next[index]!;
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
      if (appliedNode?.kind === "tool") {
        next = syncExpandedTerminalNodes(next, appliedNode);
      }
      continue;
    }
    if (patch.type === "remove" && patch.id) {
      next = next.filter((node) => node.id !== patch.id);
    }
  }
  return next;
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

function normalizePatchNodeForTimeline(nodes: RenderNode[], patch: RenderPatch): RenderNode | undefined {
  const node = patch.node;
  if (!node || patch.type !== "upsert" || !isAnonymousStreamNode(node)) {
    return node;
  }
  const appendable = latestNodeInSameTurn(nodes, node);
  if (appendable && canAppendAnonymousStreamNode(appendable, node)) {
    return {
      ...node,
      id: appendable.id,
      createdAt: appendable.createdAt,
      timelineOrder: appendable.timelineOrder,
      acpMessageId: appendable.acpMessageId
    };
  }
  return {
    ...node,
    id: nextAnonymousStreamNodeId(nodes, node)
  };
}

function isAnonymousStreamNode(node: RenderNode): node is AnonymousStreamNode {
  return isAnonymousMessageNode(node) || isAnonymousThoughtNode(node);
}

function isAnonymousMessageNode(node: RenderNode): node is MessageNode {
  return node.kind === "message" && !node.acpMessageId && node.id.startsWith(`message:${node.role}:anonymous`);
}

function isAnonymousThoughtNode(node: RenderNode): node is ThoughtNode {
  return (
    node.kind === "thought" &&
    !node.acpMessageId &&
    (node.id === "thought:current" || node.id.startsWith("thought:anonymous"))
  );
}

function canAppendAnonymousStreamNode(existing: RenderNode, incoming: AnonymousStreamNode): existing is AnonymousStreamNode {
  if (incoming.kind === "message") {
    return isAnonymousMessageNode(existing) && existing.role === incoming.role;
  }
  return isAnonymousThoughtNode(existing);
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

function sameTimelineScope(left: RenderNode, right: RenderNode): boolean {
  return (
    left.taskId === right.taskId &&
    left.lane === right.lane &&
    left.turnId === right.turnId &&
    left.acpSessionId === right.acpSessionId &&
    left.source === right.source
  );
}

function nextAnonymousStreamNodeId(nodes: RenderNode[], node: AnonymousStreamNode): string {
  const base = node.kind === "message" ? `message:${node.role}:anonymous` : "thought:anonymous";
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
  const availableModes = Array.isArray(modes.availableModes) ? modes.availableModes.filter(isSessionMode) : [];
  const currentModeId = typeof modes.currentModeId === "string" ? modes.currentModeId : undefined;
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

  const configOptions = Array.isArray(record.configOptions)
    ? record.configOptions.filter(isSessionConfigOption)
    : [];
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
        raw: record.configOptions,
        configOptions
      }
    });
  }

  return patches;
}

export function contentToText(content: ContentBlock): string {
  const record = content as Record<string, unknown>;
  if (content.type === "text" && typeof record.text === "string") {
    return record.text;
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

function contentBlocksToText(blocks: ContentBlock[]): string {
  return blocks.map(contentToText).join("");
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

function mergeRenderNode(existing: RenderNode, incoming: RenderNode): RenderNode {
  if (existing.kind === "message" && incoming.kind === "message") {
    const content = mergeAdjacentTextContentBlocks([...existing.content, ...incoming.content]);
    return {
      ...incoming,
      createdAt: existing.createdAt,
      timelineOrder: existing.timelineOrder,
      content,
      text: contentBlocksToText(content),
      streaming: existing.streaming || incoming.streaming
    };
  }
  if (existing.kind === "thought" && incoming.kind === "thought") {
    const content = mergeAdjacentTextContentBlocks([...existing.content, ...incoming.content]);
    return {
      ...incoming,
      createdAt: existing.createdAt,
      timelineOrder: existing.timelineOrder,
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

function hasExplicitToolStatus(node: ToolNode): boolean {
  const raw = node.raw as Record<string, unknown> | undefined;
  return typeof raw?.status === "string";
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
    return {
      ...existingRecord,
      ...incomingRecord
    } as ToolCallContent;
  }
  return undefined;
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
    return [messagePatch("user", update.messageId || undefined, update.content, context, true)];
  }
};

const agentMessageRenderer: AcpUpdateRenderer<AgentMessageChunkUpdate> = {
  match: (update): update is AgentMessageChunkUpdate => update.sessionUpdate === "agent_message_chunk",
  reduce(update, context) {
    return [messagePatch("assistant", update.messageId || undefined, update.content, context, true)];
  }
};

const thoughtRenderer: AcpUpdateRenderer<AgentThoughtChunkUpdate> = {
  match: (update): update is AgentThoughtChunkUpdate => update.sessionUpdate === "agent_thought_chunk",
  reduce(update, context) {
    const id = `thought:${update.messageId || "anonymous"}`;
    const node: ThoughtNode = {
      id,
      kind: "thought",
      taskId: context.taskId,
      lane: context.lane,
      turnId: context.currentTurnId,
      acpSessionId: context.acpSessionId,
      acpMessageId: update.messageId || undefined,
      provider: context.provider,
      source: "acp-live",
      status: "in_progress",
      createdAt: context.now(),
      updatedAt: context.now(),
      raw: update,
      content: [update.content],
      ephemeral: true
    };
    return [{ type: "upsert", node }];
  }
};

const planRenderer: AcpUpdateRenderer<PlanUpdate> = {
  match: (update): update is PlanUpdate =>
    update.sessionUpdate === "plan",
  reduce(update, context) {
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
          entries: Array.isArray(update.entries) ? update.entries : []
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
    const base: ToolCallUpdate = {
      sessionUpdate: "tool_call",
      toolCallId: update.toolCallId,
      title: update.title || "Tool call",
      kind: update.kind || "other",
      locations: update.locations || [],
      content: update.content || []
    };
    if (update.status) {
      base.status = update.status;
    }
    if (update.rawInput) {
      base.rawInput = update.rawInput;
    }
    if (update.rawOutput) {
      base.rawOutput = update.rawOutput;
    }
    if (update._meta) {
      base._meta = update._meta;
    }
    return expandToolContent(toolNodeFromCall(base, context), context);
  }
};

const modeRenderer: AcpUpdateRenderer<CurrentModeUpdate> = {
  match: (update): update is CurrentModeUpdate =>
    update.sessionUpdate === "current_mode_update",
  reduce(update, context) {
    const modeId = update.currentModeId || update.modeId || "unknown";
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
          availableModes: []
        }
      }
    ];
  }
};

const usageRenderer: AcpUpdateRenderer<UsageUpdate> = {
  match: (update): update is UsageUpdate =>
    update.sessionUpdate === "usage_update",
  reduce(update, context) {
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
          used: update.used,
          size: update.size,
          cost: typeof update.cost === "object" ? update.cost : undefined
        }
      }
    ];
  }
};

const configRenderer: AcpUpdateRenderer<ConfigOptionUpdate> = {
  match: (update): update is ConfigOptionUpdate =>
    update.sessionUpdate === "config_option_update",
  reduce(update, context) {
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
          configOptions: Array.isArray(update.configOptions) ? update.configOptions.filter(isSessionConfigOption) : []
        }
      }
    ];
  }
};

const commandsRenderer: AcpUpdateRenderer<AvailableCommandsUpdate> = {
  match: (update): update is AvailableCommandsUpdate =>
    update.sessionUpdate === "available_commands_update",
  reduce(update, context) {
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
          availableCommands: Array.isArray(update.availableCommands) ? update.availableCommands.filter(isAvailableCommand) : []
        }
      }
    ];
  }
};

const sessionInfoRenderer: AcpUpdateRenderer<SessionInfoUpdate> = {
  match: (update): update is SessionInfoUpdate =>
    update.sessionUpdate === "session_info_update",
  reduce(update, context) {
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
          title: typeof update.title === "string" ? update.title : undefined,
          sessionUpdatedAt: typeof update.updatedAt === "string" ? update.updatedAt : undefined
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
  content: ContentBlock,
  context: RenderReduceContext,
  streaming: boolean
): RenderPatch {
  const id = `message:${role}:${messageId || "anonymous"}`;
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
    raw: content,
    role,
    content: [content],
    text: contentToText(content),
    streaming
  };
  return { type: "upsert", node };
}

function toolNodeFromCall(call: ToolCallUpdate, context: RenderReduceContext): ToolNode {
  const timestamp = context.now();
  const content = normalizedToolContent(call);
  return syncToolTerminalContent({
    id: `tool:${call.toolCallId}`,
    kind: "tool",
    taskId: context.taskId,
    lane: context.lane,
    turnId: context.currentTurnId,
    acpSessionId: context.acpSessionId,
    acpToolCallId: call.toolCallId,
    provider: context.provider,
    source: "acp-live",
    status: mapToolStatus(call.status),
    createdAt: timestamp,
    updatedAt: timestamp,
    raw: call,
    toolCallId: call.toolCallId,
    title: call.title,
    toolKind: call.kind || "other",
    toolStatus: call.status || "in_progress",
    locations: call.locations || [],
    content,
    rawInput: call.rawInput,
    rawOutput: call.rawOutput
  });
}

function normalizedToolContent(call: ToolCallUpdate): ToolCallContent[] {
  if (call.content?.length) {
    return call.content;
  }
  const output = recoveredRawToolOutput(call.rawOutput);
  if (!output) {
    return [];
  }
  if (isCommandToolCall(call)) {
    const command = commandField(asRecord(call.rawInput)) || call.title || "Command";
    const terminal: ToolTerminalBlock = {
      type: "terminal",
      terminalId: call.toolCallId,
      title: call.title,
      command,
      stdout: output.text
    };
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

function isCommandToolCall(call: ToolCallUpdate): boolean {
  if (call.kind === "execute") {
    return true;
  }
  if (commandField(asRecord(call.rawInput))) {
    return true;
  }
  return /^(bash|shell|terminal|execute|command|run)$/i.test(call.title.trim());
}

function expandToolContent(tool: ToolNode, context: RenderReduceContext): RenderPatch[] {
  const patches: RenderPatch[] = [{ type: "upsert", node: tool }];
  for (const item of tool.content) {
    const record = item as Record<string, unknown>;
    if (item.type === "diff") {
      const path = typeof record.path === "string" ? record.path : "unknown";
      const newText = typeof record.newText === "string" ? record.newText : "";
      const oldText = typeof record.oldText === "string" ? record.oldText : null;
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
      const terminalId = typeof record.terminalId === "string" ? record.terminalId : "unknown";
      const terminal = terminalDetails(record);
      patches.push({
        type: "upsert",
        node: {
          id: `terminal:${terminalId}`,
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

function syncExpandedTerminalNodes(nodes: RenderNode[], tool: ToolNode): RenderNode[] {
  let changed = false;
  const next = nodes.map((node) => {
    if (node.kind !== "terminal" || node.acpToolCallId !== tool.toolCallId) {
      return node;
    }
    const terminalStatus = syncTerminalStatusFromTool(node.terminalStatus, tool.toolStatus);
    if (node.status === tool.status && node.terminalStatus === terminalStatus) {
      return node;
    }
    changed = true;
    return {
      ...node,
      status: tool.status,
      terminalStatus,
      updatedAt: tool.updatedAt
    };
  });
  return changed ? next : nodes;
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
    output: incoming.output ?? existing.output,
    stdout: incoming.stdout ?? existing.stdout,
    stderr: incoming.stderr ?? existing.stderr
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
    output: stringField(record, "output"),
    stdout: stringField(record, "stdout") || stringField(record, "stdoutPreview") || stringField(record, "stdout_preview"),
    stderr: stringField(record, "stderr") || stringField(record, "stderrPreview") || stringField(record, "stderr_preview")
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

function mapToolStatus(status: string | undefined): "pending" | "in_progress" | "completed" | "failed" | "cancelled" {
  if (status === "pending" || status === "completed" || status === "failed" || status === "cancelled") {
    return status;
  }
  return "in_progress";
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

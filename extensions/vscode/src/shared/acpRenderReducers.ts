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
  ToolCallPatchUpdate,
  ToolCallUpdate,
  UsageUpdate,
  UserMessageChunkUpdate
} from "./acpTypes";
import type {
  ApprovalNode,
  MessageNode,
  RenderNode,
  RenderPatch,
  RenderReduceContext,
  ThoughtNode,
  ToolNode
} from "./renderModel";

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
  for (const patch of patches) {
    if (patch.type === "append" && patch.node) {
      next.push(patch.node);
      continue;
    }
    if ((patch.type === "replace" || patch.type === "upsert") && patch.node) {
      const index = next.findIndex((node) => node.id === patch.node?.id);
      if (index >= 0) {
        const existing = next[index];
        next[index] = patch.type === "upsert" && existing ? mergeRenderNode(existing, patch.node) : patch.node;
      } else {
        next.push(patch.node);
      }
      continue;
    }
    if (patch.type === "remove" && patch.id) {
      next = next.filter((node) => node.id !== patch.id);
    }
  }
  return next;
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

function mergeRenderNode(existing: RenderNode, incoming: RenderNode): RenderNode {
  if (existing.kind === "message" && incoming.kind === "message") {
    const content = [...existing.content, ...incoming.content];
    return {
      ...incoming,
      createdAt: existing.createdAt,
      content,
      text: contentBlocksToText(content),
      streaming: existing.streaming || incoming.streaming
    };
  }
  if (existing.kind === "thought" && incoming.kind === "thought") {
    return {
      ...incoming,
      createdAt: existing.createdAt,
      content: [...existing.content, ...incoming.content]
    };
  }
  return incoming;
}

const userMessageRenderer: AcpUpdateRenderer<UserMessageChunkUpdate> = {
  match: (update): update is UserMessageChunkUpdate => update.sessionUpdate === "user_message_chunk",
  reduce(update, context) {
    return [messagePatch("user", update.messageId || "current", update.content, context, true)];
  }
};

const agentMessageRenderer: AcpUpdateRenderer<AgentMessageChunkUpdate> = {
  match: (update): update is AgentMessageChunkUpdate => update.sessionUpdate === "agent_message_chunk",
  reduce(update, context) {
    return [messagePatch("assistant", update.messageId || "current", update.content, context, true)];
  }
};

const thoughtRenderer: AcpUpdateRenderer<AgentThoughtChunkUpdate> = {
  match: (update): update is AgentThoughtChunkUpdate => update.sessionUpdate === "agent_thought_chunk",
  reduce(update, context) {
    const id = `thought:${update.messageId || "current"}`;
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
      status: update.status || "in_progress",
      kind: update.kind || "other",
      locations: update.locations || [],
      content: update.content || []
    };
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
  messageId: string,
  content: ContentBlock,
  context: RenderReduceContext,
  streaming: boolean
): RenderPatch {
  const id = `message:${role}:${messageId}`;
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
  return {
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
    updatedAt: context.now(),
    raw: call,
    toolCallId: call.toolCallId,
    title: call.title,
    toolKind: call.kind || "other",
    toolStatus: call.status || "in_progress",
    locations: call.locations || [],
    content: call.content || [],
    rawInput: call.rawInput,
    rawOutput: call.rawOutput
  };
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

function mapToolStatus(status: string | undefined): "pending" | "in_progress" | "completed" | "failed" {
  if (status === "pending" || status === "completed" || status === "failed") {
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

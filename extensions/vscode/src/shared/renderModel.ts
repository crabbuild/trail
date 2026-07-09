import type {
  ContentBlock,
  AvailableCommand,
  JsonObject,
  PlanEntry,
  SessionConfigOption,
  SessionMode,
  ToolCallContent,
  ToolCallLocation,
  ToolCallStatus,
  ToolKind
} from "./acpTypes";

export type RenderSource = "acp-live" | "trail" | "merged";
export type RenderStatus = "pending" | "in_progress" | "completed" | "failed" | "cancelled";

export interface RenderNodeBase {
  id: string;
  kind: RenderNodeKind;
  taskId: string;
  lane: string;
  turnId?: string | undefined;
  acpSessionId?: string | undefined;
  acpMessageId?: string | undefined;
  acpToolCallId?: string | undefined;
  provider?: string | undefined;
  source: RenderSource;
  status: RenderStatus;
  timelineOrder?: number | undefined;
  createdAt?: string | undefined;
  updatedAt?: string | undefined;
  raw?: unknown;
}

export type RenderNodeKind =
  | "message"
  | "thought"
  | "plan"
  | "tool"
  | "diff"
  | "terminal"
  | "approval"
  | "checkpoint"
  | "completion"
  | "usage"
  | "mode"
  | "config"
  | "resource"
  | "commands"
  | "session"
  | "unknown";

export interface MessageNode extends RenderNodeBase {
  kind: "message";
  role: "user" | "assistant";
  content: ContentBlock[];
  text: string;
  streaming: boolean;
}

export interface ThoughtNode extends RenderNodeBase {
  kind: "thought";
  content: ContentBlock[];
  ephemeral: true;
}

export interface PlanNode extends RenderNodeBase {
  kind: "plan";
  entries: PlanEntry[];
}

export interface ToolNode extends RenderNodeBase {
  kind: "tool";
  toolCallId: string;
  title: string;
  toolKind: ToolKind;
  toolStatus: ToolCallStatus;
  locations: ToolCallLocation[];
  content: ToolCallContent[];
  rawInput?: JsonObject | undefined;
  rawOutput?: JsonObject | undefined;
  permission?: ToolPermissionRequest | undefined;
}

export interface ToolPermissionRequest {
  requestId: string;
  title: string;
  status: RenderStatus;
  options: Array<{ optionId: string; label: string; description?: string | undefined }>;
  raw?: unknown;
  provider?: string | undefined;
  createdAt?: string | undefined;
  updatedAt?: string | undefined;
}

export interface DiffNode extends RenderNodeBase {
  kind: "diff";
  path: string;
  oldText?: string | null;
  newText: string;
}

export interface TerminalNode extends RenderNodeBase {
  kind: "terminal";
  terminalId: string;
  title?: string;
  command?: string | undefined;
  cwd?: string | undefined;
  terminalStatus?: string | undefined;
  exitCode?: number | undefined;
  elapsedMs?: number | undefined;
  output?: string | undefined;
  stdout?: string | undefined;
  stderr?: string | undefined;
}

export interface ApprovalNode extends RenderNodeBase {
  kind: "approval";
  requestId: string;
  title: string;
  tool: ToolNode;
  options: Array<{ optionId: string; label: string; description?: string | undefined }>;
}

export interface CheckpointNode extends RenderNodeBase {
  kind: "checkpoint";
  checkpointId?: string | undefined;
  label: string;
}

export interface CompletionNode extends RenderNodeBase {
  kind: "completion";
  stopReason: string;
  label: string;
  checkpointPending: boolean;
}

export interface UsageNode extends RenderNodeBase {
  kind: "usage";
  used: number;
  size: number;
  cost?: JsonObject | null | undefined;
}

export interface ModeNode extends RenderNodeBase {
  kind: "mode";
  modeId: string;
  availableModes: SessionMode[];
}

export interface ConfigNode extends RenderNodeBase {
  kind: "config";
  configOptions: SessionConfigOption[];
}

export interface ResourceNode extends RenderNodeBase {
  kind: "resource";
  content: ContentBlock;
}

export interface CommandsNode extends RenderNodeBase {
  kind: "commands";
  availableCommands: AvailableCommand[];
}

export interface SessionNode extends RenderNodeBase {
  kind: "session";
  title?: string | null | undefined;
  sessionUpdatedAt?: string | null | undefined;
}

export interface UnknownNode extends RenderNodeBase {
  kind: "unknown";
  label: string;
  payload: unknown;
}

export type RenderNode =
  | MessageNode
  | ThoughtNode
  | PlanNode
  | ToolNode
  | DiffNode
  | TerminalNode
  | ApprovalNode
  | CheckpointNode
  | CompletionNode
  | UsageNode
  | ModeNode
  | ConfigNode
  | ResourceNode
  | CommandsNode
  | SessionNode
  | UnknownNode;

export interface RenderState {
  taskId: string;
  lane: string;
  provider?: string;
  acpSessionId?: string;
  currentTurnId?: string;
  nodes: RenderNode[];
}

export interface RenderPatch {
  type: "append" | "replace" | "upsert" | "remove";
  node?: RenderNode;
  id?: string;
}

export interface RenderReduceContext {
  taskId: string;
  lane: string;
  acpSessionId?: string | undefined;
  currentTurnId?: string | undefined;
  provider?: string | undefined;
  now(): string;
}

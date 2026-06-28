export type JsonObject = Record<string, unknown>;

export interface ContentBlockBase {
  type: string;
  _meta?: JsonObject | null;
  annotations?: JsonObject | null;
}

export interface TextContentBlock extends ContentBlockBase {
  type: "text";
  text: string;
}

export interface ImageContentBlock extends ContentBlockBase {
  type: "image";
  data: string;
  mimeType: string;
  uri?: string | null;
}

export interface AudioContentBlock extends ContentBlockBase {
  type: "audio";
  data: string;
  mimeType: string;
}

export interface ResourceLinkBlock extends ContentBlockBase {
  type: "resource_link";
  uri: string;
  name: string;
  title?: string | null;
  description?: string | null;
  mimeType?: string | null;
  size?: number | null;
}

export interface EmbeddedResourceBlock extends ContentBlockBase {
  type: "resource";
  resource: {
    uri: string;
    mimeType?: string | null;
    text?: string;
    blob?: string;
    _meta?: JsonObject | null;
  };
}

export type ContentBlock =
  | TextContentBlock
  | ImageContentBlock
  | AudioContentBlock
  | ResourceLinkBlock
  | EmbeddedResourceBlock
  | (ContentBlockBase & JsonObject);

export interface ToolCallContentBase {
  type: string;
  _meta?: JsonObject | null;
}

export interface ToolContentBlock extends ToolCallContentBase {
  type: "content";
  content: ContentBlock;
}

export interface ToolDiffBlock extends ToolCallContentBase {
  type: "diff";
  path: string;
  oldText?: string | null;
  newText: string;
}

export interface ToolTerminalBlock extends ToolCallContentBase {
  type: "terminal";
  terminalId: string;
  title?: string | null;
  name?: string | null;
  command?: string | unknown[];
  commandLine?: string | null;
  command_line?: string | null;
  cwd?: string | null;
  workingDirectory?: string | null;
  working_directory?: string | null;
  status?: string | null;
  state?: string | null;
  exitCode?: number | null;
  exit_code?: number | null;
  elapsedMs?: number | null;
  elapsed_ms?: number | null;
  durationMs?: number | null;
  output?: string | null;
  stdout?: string | null;
  stderr?: string | null;
  stdoutPreview?: string | null;
  stdout_preview?: string | null;
  stderrPreview?: string | null;
  stderr_preview?: string | null;
}

export type ToolCallContent =
  | ToolContentBlock
  | ToolDiffBlock
  | ToolTerminalBlock
  | (ToolCallContentBase & JsonObject);

export type SessionUpdate =
  | UserMessageChunkUpdate
  | AgentMessageChunkUpdate
  | AgentThoughtChunkUpdate
  | ToolCallUpdate
  | ToolCallPatchUpdate
  | PlanUpdate
  | AvailableCommandsUpdate
  | CurrentModeUpdate
  | ConfigOptionUpdate
  | SessionInfoUpdate
  | UsageUpdate
  | (JsonObject & { sessionUpdate?: string });

export interface UserMessageChunkUpdate {
  sessionUpdate: "user_message_chunk";
  content: ContentBlock;
  messageId?: string | null;
  _meta?: JsonObject | null;
}

export interface AgentMessageChunkUpdate {
  sessionUpdate: "agent_message_chunk";
  content: ContentBlock;
  messageId?: string | null;
  _meta?: JsonObject | null;
}

export interface AgentThoughtChunkUpdate {
  sessionUpdate: "agent_thought_chunk";
  content: ContentBlock;
  messageId?: string | null;
  _meta?: JsonObject | null;
}

export interface ToolCallUpdate {
  sessionUpdate: "tool_call";
  toolCallId: string;
  title: string;
  status?: ToolCallStatus;
  kind?: ToolKind;
  locations?: ToolCallLocation[];
  content?: ToolCallContent[];
  rawInput?: JsonObject;
  rawOutput?: JsonObject;
  _meta?: JsonObject | null;
}

export interface ToolCallPatchUpdate {
  sessionUpdate: "tool_call_update";
  toolCallId: string;
  title?: string | null;
  status?: ToolCallStatus | null;
  kind?: ToolKind | null;
  locations?: ToolCallLocation[] | null;
  content?: ToolCallContent[] | null;
  rawInput?: JsonObject;
  rawOutput?: JsonObject;
  _meta?: JsonObject | null;
}

export type ToolCallStatus = "pending" | "in_progress" | "completed" | "failed" | "cancelled";

export type ToolKind =
  | "read"
  | "edit"
  | "delete"
  | "move"
  | "search"
  | "execute"
  | "think"
  | "fetch"
  | "switch_mode"
  | "other";

export interface ToolCallLocation {
  path: string;
  line?: number | null;
  _meta?: JsonObject | null;
}

export interface PlanUpdate {
  sessionUpdate: "plan";
  entries: PlanEntry[];
  _meta?: JsonObject | null;
}

export interface PlanEntry {
  content?: string;
  title?: string;
  status?: string;
  priority?: string | null;
  _meta?: JsonObject | null;
}

export interface AvailableCommandsUpdate {
  sessionUpdate: "available_commands_update";
  availableCommands: AvailableCommand[];
  _meta?: JsonObject | null;
}

export interface AvailableCommand extends JsonObject {
  name: string;
  description: string;
  input?: {
    hint: string;
    _meta?: JsonObject | null;
  } | null;
}

export interface CurrentModeUpdate {
  sessionUpdate: "current_mode_update";
  currentModeId?: string;
  modeId?: string;
  _meta?: JsonObject | null;
}

export interface ConfigOptionUpdate {
  sessionUpdate: "config_option_update";
  configOptions: SessionConfigOption[];
  _meta?: JsonObject | null;
}

export interface SessionModeState extends JsonObject {
  currentModeId: string;
  availableModes: SessionMode[];
}

export interface SessionMode extends JsonObject {
  id: string;
  name: string;
  description?: string | null;
}

export interface SessionConfigOption extends JsonObject {
  id: string;
  name: string;
  type: string;
  currentValue?: string | number | boolean | null;
  options?: SessionConfigOptionValue[] | null;
  category?: string | null;
  description?: string | null;
}

export interface SessionConfigOptionValue extends JsonObject {
  value: string | number | boolean;
  name?: string | null;
  description?: string | null;
}

export interface NewSessionResponse extends JsonObject {
  sessionId: string;
  modes?: SessionModeState | null;
  configOptions?: SessionConfigOption[] | null;
}

export type StopReason =
  | "end_turn"
  | "max_tokens"
  | "max_turn_requests"
  | "refusal"
  | "cancelled"
  | (string & {});

export interface PromptResponse extends JsonObject {
  stopReason?: StopReason;
}

export interface SessionInfoUpdate {
  sessionUpdate: "session_info_update";
  title?: string | null;
  updatedAt?: string | null;
  _meta?: JsonObject | null;
}

export interface UsageUpdate {
  sessionUpdate: "usage_update";
  size: number;
  used: number;
  cost?: JsonObject | null;
  _meta?: JsonObject | null;
}

export interface PermissionOption {
  optionId: string;
  name?: string;
  kind?: string;
  description?: string;
}

export interface RequestPermissionParams {
  sessionId: string;
  toolCall: ToolCallUpdate;
  options: PermissionOption[];
  _meta?: JsonObject | null;
}

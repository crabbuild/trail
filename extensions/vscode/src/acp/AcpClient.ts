import { createReadStream } from "node:fs";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import * as readline from "node:readline";
import type * as vscode from "vscode";
import type {
  ContentBlock,
  NewSessionResponse,
  PromptResponse,
  RequestPermissionParams,
  SessionUpdate
} from "../shared/acpTypes";
import { capabilitiesFromInitializeResponse, type AgentCapabilities } from "./AcpCapabilities";
import type { AcpProviderProfile } from "./ProviderRegistry";
import { JsonRpcProcess, JsonRpcRequestError, type JsonRpcMessage } from "./JsonRpcProcess";

const MAX_READ_TEXT_FILE_BYTES = 1024 * 1024;
const MAX_READ_TEXT_FILE_CHARS = 1024 * 1024;
const ACP_PROTOCOL_VERSION = 1;

export interface AcpClientEvents {
  update(update: SessionUpdate, sessionId?: string | undefined): void;
  permission(requestId: string, params: RequestPermissionParams): void;
  completed(response: unknown): void;
  error(error: Error): void;
  stderr?(line: string): void;
  exit(code: number | null, signal: NodeJS.Signals | null): void;
  authenticate?(methods: AcpAuthMethod[]): Promise<string | undefined> | string | undefined;
  authenticated?(method: AcpAuthMethod): void;
}

export interface AcpAuthMethod {
  id: string;
  name: string;
  description?: string | null | undefined;
  type?: string | null | undefined;
  raw: Record<string, unknown>;
}

export interface AcpSessionInfo {
  sessionId: string;
  initialize: unknown;
  session: NewSessionResponse;
  capabilities: AgentCapabilities;
  startMode: "new" | "load" | "resume";
  requestedSessionId?: string | undefined;
  authMethods: AcpAuthMethod[];
  authenticatedMethod?: AcpAuthMethod | undefined;
}

export interface AcpStartOptions {
  taskName?: string | undefined;
  existingSessionId?: string | undefined;
  fromRef?: string | undefined;
}

export interface AcpClientWorkspaceAccess {
  readOpenTextDocument?(filePath: string): string | undefined;
  additionalWorkspaceRoots?: string[] | undefined;
}

export class AcpClient {
  private readonly rpc: JsonRpcProcess;
  private listeners?: AcpClientEvents;
  private sessionId?: string;
  private authMethods: AcpAuthMethod[] = [];
  private activeAdditionalWorkspaceRoots: string[] = [];
  private readonly pendingPermissionRequests = new Map<string, JsonRpcMessage["id"]>();

  constructor(
    private readonly workspaceRoot: string,
    private readonly provider: AcpProviderProfile,
    output: vscode.OutputChannel,
    private readonly workspaceAccess: AcpClientWorkspaceAccess = {}
  ) {
    this.rpc = new JsonRpcProcess(output);
    this.rpc.on("notification", (message) => this.handleNotification(message));
    this.rpc.on("request", (message) => this.handleRequest(message));
    this.rpc.on("stderr", (line) => this.listeners?.stderr?.(line));
    this.rpc.on("exit", (code, signal) => this.listeners?.exit(code, signal));
  }

  async start(listeners: AcpClientEvents, options: AcpStartOptions = {}): Promise<AcpSessionInfo> {
    this.listeners = listeners;
    const args = [...this.provider.args];
    if (options.taskName && this.provider.supportsTaskName) {
      args.push("--name", options.taskName);
    }
    if (options.fromRef && this.provider.supportsFromRef) {
      args.push("--from", options.fromRef);
    }
    this.rpc.start(this.provider.command, args, this.workspaceRoot);

    const initialize = await this.rpc.request("initialize", {
      protocolVersion: ACP_PROTOCOL_VERSION,
      clientInfo: {
        name: "CrabDB VS Code",
        version: "0.1.0"
      },
      clientCapabilities: {
        fs: {
          readTextFile: true,
          writeTextFile: false
        },
        terminal: false
      },
      _meta: {
        crabdb: {
          client: "vscode"
        }
      }
    });
    ensureSupportedProtocolVersion(initialize);

    const capabilities = capabilitiesFromInitializeResponse(initialize);
    const authMethods = authMethodsFromInitializeResponse(initialize);
    this.authMethods = authMethods;
    const startMode = this.resolveStartMode(capabilities, options.existingSessionId);
    const { session, authenticatedMethod } = await this.startSessionWithAuth(
      startMode,
      options.existingSessionId,
      capabilities,
      authMethods
    );

    this.sessionId = session.sessionId;
    return {
      sessionId: session.sessionId,
      initialize,
      session,
      capabilities,
      startMode,
      requestedSessionId: options.existingSessionId,
      authMethods,
      authenticatedMethod
    };
  }

  private resolveStartMode(
    capabilities: AgentCapabilities,
    existingSessionId: string | undefined
  ): "new" | "load" | "resume" {
    if (!existingSessionId) {
      return "new";
    }
    if (supportsSessionCapability(capabilities, "resume")) {
      return "resume";
    }
    if (capabilities.loadSession) {
      return "load";
    }
    return "new";
  }

  private async startSession(
    startMode: "new" | "load" | "resume",
    existingSessionId: string | undefined,
    capabilities: AgentCapabilities
  ): Promise<NewSessionResponse> {
    const base: Record<string, unknown> = {
      cwd: this.workspaceRoot,
      mcpServers: [],
      _meta: {
        crabdb: {
          client: "vscode"
        }
      }
    };
    if (supportsSessionCapability(capabilities, "additionalDirectories")) {
      this.activeAdditionalWorkspaceRoots = sanitizeAdditionalDirectories(
        this.workspaceRoot,
        this.workspaceAccess.additionalWorkspaceRoots ?? []
      );
      base.additionalDirectories = this.activeAdditionalWorkspaceRoots;
    } else {
      this.activeAdditionalWorkspaceRoots = [];
    }

    if (startMode === "resume" && existingSessionId) {
      const response = await this.rpc.request<Partial<NewSessionResponse> | null>("session/resume", {
        ...base,
        sessionId: existingSessionId
      });
      return normalizeLoadedSession(existingSessionId, response);
    }

    if (startMode === "load" && existingSessionId) {
      const response = await this.rpc.request<Partial<NewSessionResponse> | null>("session/load", {
        ...base,
        sessionId: existingSessionId
      });
      return normalizeLoadedSession(existingSessionId, response);
    }

    const response = await this.rpc.request<NewSessionResponse>("session/new", base);
    return normalizeNewSessionResponse(response);
  }

  private async startSessionWithAuth(
    startMode: "new" | "load" | "resume",
    existingSessionId: string | undefined,
    capabilities: AgentCapabilities,
    authMethods: AcpAuthMethod[]
  ): Promise<{ session: NewSessionResponse; authenticatedMethod?: AcpAuthMethod | undefined }> {
    try {
      return {
        session: await this.startSession(startMode, existingSessionId, capabilities)
      };
    } catch (error) {
      if (!isAuthRequiredError(error) || authMethods.length === 0) {
        throw error;
      }
      const method = await this.authenticateAgent(authMethods);
      return {
        session: await this.startSession(startMode, existingSessionId, capabilities),
        authenticatedMethod: method
      };
    }
  }

  private async requestWithAuthRetry<T>(
    method: string,
    params: unknown,
    timeoutMs?: number | null
  ): Promise<T> {
    try {
      return await this.rpc.request<T>(method, params, timeoutMs);
    } catch (error) {
      if (!isAuthRequiredError(error) || this.authMethods.length === 0) {
        throw error;
      }
      await this.authenticateAgent(this.authMethods);
      return this.rpc.request<T>(method, params, timeoutMs);
    }
  }

  private async authenticateAgent(authMethods: AcpAuthMethod[]): Promise<AcpAuthMethod> {
    const method = await this.pickAuthMethod(authMethods);
    await this.rpc.request("authenticate", { methodId: method.id }, 10 * 60 * 1000);
    this.listeners?.authenticated?.(method);
    return method;
  }

  private async pickAuthMethod(authMethods: AcpAuthMethod[]): Promise<AcpAuthMethod> {
    const pickedId = this.listeners?.authenticate
      ? await this.listeners.authenticate(authMethods)
      : authMethods[0]?.id;
    if (!pickedId) {
      throw new Error("Agent authentication was cancelled.");
    }
    const method = authMethods.find((candidate) => candidate.id === pickedId);
    if (!method) {
      throw new Error(`Agent authentication method is unavailable: ${pickedId}`);
    }
    return method;
  }

  async prompt(content: ContentBlock[] | string): Promise<PromptResponse> {
    if (!this.sessionId) {
      throw new Error("ACP session is not initialized.");
    }
    const prompt = typeof content === "string" ? [{ type: "text" as const, text: content }] : content;
    const response = await this.requestWithAuthRetry<PromptResponse>(
      "session/prompt",
      {
        sessionId: this.sessionId,
        prompt
      },
      null
    );
    this.listeners?.completed(response);
    return response;
  }

  async setMode(modeId: string): Promise<unknown> {
    if (!this.sessionId) {
      throw new Error("ACP session is not initialized.");
    }
    return this.requestWithAuthRetry("session/set_mode", {
      sessionId: this.sessionId,
      modeId
    });
  }

  async setConfigOption(configId: string, value: string): Promise<unknown> {
    if (!this.sessionId) {
      throw new Error("ACP session is not initialized.");
    }
    return this.requestWithAuthRetry("session/set_config_option", {
      sessionId: this.sessionId,
      configId,
      value
    });
  }

  cancel(): string[] {
    if (!this.sessionId) {
      return [];
    }
    this.rpc.notify("session/cancel", { sessionId: this.sessionId });
    return this.cancelPendingPermissionRequests();
  }

  approve(requestId: string, optionId: string): void {
    const rpcId = this.pendingPermissionRequests.has(requestId)
      ? this.pendingPermissionRequests.get(requestId)
      : requestId;
    this.rpc.respond(rpcId, {
      outcome: {
        outcome: "selected",
        optionId
      }
    });
    this.pendingPermissionRequests.delete(requestId);
  }

  reject(requestId: string): void {
    const rpcId = this.pendingPermissionRequests.has(requestId)
      ? this.pendingPermissionRequests.get(requestId)
      : requestId;
    this.rpc.respond(rpcId, {
      outcome: {
        outcome: "cancelled"
      }
    });
    this.pendingPermissionRequests.delete(requestId);
  }

  dispose(): void {
    this.rpc.dispose();
  }

  private handleNotification(message: JsonRpcMessage): void {
    if (message.method === "session/update") {
      const params = asRecord(message.params);
      const update = sessionUpdateFromNotificationParams(params);
      const sessionId = sessionIdFromRecord(params);
      if (update) {
        this.listeners?.update(update, sessionId);
      }
    }
  }

  private handleRequest(message: JsonRpcMessage): void {
    if (message.method === "session/request_permission") {
      const params = normalizeRequestPermissionParams(message.params, this.sessionId);
      const requestId = String(message.id);
      this.pendingPermissionRequests.set(requestId, message.id);
      this.listeners?.permission(requestId, params);
      return;
    }

    if (message.method === "fs/read_text_file") {
      void this.handleReadTextFile(message);
      return;
    }

    if (message.method?.startsWith("fs/")) {
      this.rpc.respondError(message.id, "CrabDB VS Code does not expose direct filesystem mutation in this build.");
      return;
    }

    if (message.method?.startsWith("terminal/")) {
      this.rpc.respondError(message.id, "CrabDB VS Code does not expose direct terminal execution in this build.");
      return;
    }

    this.rpc.respondError(message.id, `Unsupported ACP client request: ${message.method || "unknown"}`);
  }

  private cancelPendingPermissionRequests(): string[] {
    const cancelled = [...this.pendingPermissionRequests.keys()];
    for (const requestId of cancelled) {
      this.reject(requestId);
    }
    return cancelled;
  }

  private async handleReadTextFile(message: JsonRpcMessage): Promise<void> {
    try {
      const content = await readWorkspaceTextFile(
        this.workspaceRoot,
        asRecord(message.params),
        this.workspaceAccess.readOpenTextDocument,
        this.activeAdditionalWorkspaceRoots
      );
      this.rpc.respond(message.id, { content });
    } catch (error) {
      this.rpc.respondError(message.id, error instanceof Error ? error.message : String(error));
    }
  }
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function asPlainRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" && value ? value : undefined;
}

function sessionIdFromRecord(record: Record<string, unknown>, fallback?: string | undefined): string | undefined {
  return stringField(record, "sessionId") || stringField(record, "session_id") || fallback;
}

function sessionUpdateFromNotificationParams(params: Record<string, unknown>): SessionUpdate | undefined {
  const explicit = firstRecord(params.update, params.sessionUpdate, params.session_update);
  if (Object.keys(explicit).length) {
    return normalizeSessionUpdatePayload(explicit);
  }
  return stringField(params, "sessionUpdate") || stringField(params, "session_update")
    ? normalizeSessionUpdatePayload(params)
    : undefined;
}

function normalizeSessionUpdatePayload(record: Record<string, unknown>): SessionUpdate {
  const sessionUpdate = stringField(record, "sessionUpdate") || stringField(record, "session_update");
  return sessionUpdate
    ? ({ ...record, sessionUpdate } as SessionUpdate)
    : (record as SessionUpdate);
}

function supportsSessionCapability(capabilities: AgentCapabilities, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(capabilities.sessionCapabilities, key);
}

function normalizeLoadedSession(sessionId: string, response: Partial<NewSessionResponse> | null): NewSessionResponse {
  return normalizeNewSessionResponse(response, sessionId);
}

function normalizeNewSessionResponse(
  response: Partial<NewSessionResponse> | null,
  fallbackSessionId?: string | undefined
): NewSessionResponse {
  const record = asPlainRecord(response);
  const sessionId = sessionIdFromRecord(record, fallbackSessionId);
  if (!sessionId) {
    throw new Error("ACP session response did not include a session id.");
  }
  return {
    ...record,
    sessionId
  } as NewSessionResponse;
}

function normalizeRequestPermissionParams(
  params: unknown,
  fallbackSessionId?: string | undefined
): RequestPermissionParams {
  const record = asPlainRecord(params);
  return {
    ...record,
    sessionId: sessionIdFromRecord(record, fallbackSessionId) || "",
    toolCall: normalizePermissionToolCall(record),
    options: normalizePermissionOptions(record.options)
  };
}

function normalizePermissionToolCall(record: Record<string, unknown>): RequestPermissionParams["toolCall"] {
  const toolCall = firstRecord(record.toolCall, record.tool_call);
  const toolCallId = stringField(toolCall, "toolCallId") || stringField(toolCall, "tool_call_id") || stringField(toolCall, "id");
  return {
    ...toolCall,
    sessionUpdate: "tool_call",
    toolCallId: toolCallId || "unknown",
    title: stringField(toolCall, "title") || stringField(toolCall, "name") || "Tool call"
  } as RequestPermissionParams["toolCall"];
}

function normalizePermissionOptions(value: unknown): RequestPermissionParams["options"] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.map((option, index) => {
    const record = asPlainRecord(option);
    const normalized: RequestPermissionParams["options"][number] = {
      ...record,
      optionId: stringField(record, "optionId") || stringField(record, "option_id") || stringField(record, "id") || `option-${index + 1}`
    };
    const name = stringField(record, "name") || stringField(record, "label");
    if (name) {
      normalized.name = name;
    }
    const kind = stringField(record, "kind");
    if (kind) {
      normalized.kind = kind;
    }
    const description = stringField(record, "description");
    if (description) {
      normalized.description = description;
    }
    return normalized;
  });
}

function firstRecord(...values: unknown[]): Record<string, unknown> {
  for (const value of values) {
    const record = asPlainRecord(value);
    if (Object.keys(record).length) {
      return record;
    }
  }
  return {};
}

function ensureSupportedProtocolVersion(initialize: unknown): void {
  const value = asRecord(initialize).protocolVersion;
  const version = typeof value === "number" ? value : typeof value === "string" ? Number(value) : Number.NaN;
  if (!Number.isInteger(version)) {
    throw new Error("ACP initialize response did not include a valid protocol version.");
  }
  if (version !== ACP_PROTOCOL_VERSION) {
    throw new Error(`Unsupported ACP protocol version ${version}; CrabDB VS Code supports ${ACP_PROTOCOL_VERSION}.`);
  }
}

function authMethodsFromInitializeResponse(response: unknown): AcpAuthMethod[] {
  const methods = asRecord(response).authMethods;
  if (!Array.isArray(methods)) {
    return [];
  }
  return methods.map(normalizeAuthMethod).filter((method): method is AcpAuthMethod => method !== undefined);
}

function normalizeAuthMethod(value: unknown): AcpAuthMethod | undefined {
  const record = asRecord(value);
  if (typeof record.id !== "string" || typeof record.name !== "string") {
    return undefined;
  }
  const method: AcpAuthMethod = {
    id: record.id,
    name: record.name,
    raw: record
  };
  if (typeof record.description === "string" || record.description === null) {
    method.description = record.description;
  }
  if (typeof record.type === "string" || record.type === null) {
    method.type = record.type;
  }
  return method;
}

function isAuthRequiredError(error: unknown): boolean {
  if (!(error instanceof JsonRpcRequestError)) {
    return false;
  }
  const message = error.message.toLowerCase();
  const data = asRecord(error.data);
  const dataCode = typeof data.code === "string" ? data.code.toLowerCase() : "";
  return (
    message.includes("auth_required") ||
    message.includes("authentication required") ||
    dataCode === "auth_required"
  );
}

async function readWorkspaceTextFile(
  workspaceRoot: string,
  params: Record<string, unknown>,
  openTextDocument?: ((filePath: string) => string | undefined) | undefined,
  additionalWorkspaceRoots: string[] = []
): Promise<string> {
  const requestedPath = typeof params.path === "string" ? params.path : "";
  if (!requestedPath) {
    throw new Error("fs/read_text_file requires a path.");
  }
  if (!path.isAbsolute(requestedPath)) {
    throw new Error("fs/read_text_file path must be absolute.");
  }

  const allowedRoots = await realAllowedRoots(workspaceRoot, additionalWorkspaceRoots);
  const resolvedRequestedPath = path.resolve(requestedPath);
  const openText =
    openTextDocument?.(requestedPath) ??
    (resolvedRequestedPath === requestedPath ? undefined : openTextDocument?.(resolvedRequestedPath));
  let targetRealPath: string | undefined;
  try {
    targetRealPath = await fs.realpath(requestedPath);
  } catch (error) {
    if (openText === undefined) {
      throw error;
    }
    const parentRealPath = await nearestExistingParentRealPath(resolvedRequestedPath);
    if (!isInsideAnyWorkspace(allowedRoots, parentRealPath)) {
      throw new Error("fs/read_text_file path is outside the workspace.");
    }
    return boundedTextResponse(openText, positiveInteger(params.line) ?? 1, nonNegativeInteger(params.limit));
  }

  if (!isInsideAnyWorkspace(allowedRoots, targetRealPath)) {
    throw new Error("fs/read_text_file path is outside the workspace.");
  }

  const stat = await fs.stat(targetRealPath);
  if (!stat.isFile()) {
    throw new Error("fs/read_text_file path is not a file.");
  }

  const startLine = positiveInteger(params.line) ?? 1;
  const requestedLimit = nonNegativeInteger(params.limit);
  const lineRangeRequested = startLine > 1 || requestedLimit !== undefined;
  const realOpenText = openText ?? openTextDocument?.(targetRealPath);
  if (realOpenText !== undefined) {
    return boundedTextResponse(realOpenText, startLine, requestedLimit);
  }

  if (!lineRangeRequested) {
    if (stat.size > MAX_READ_TEXT_FILE_BYTES) {
      throw new Error("fs/read_text_file file is too large; request a smaller line range.");
    }
    return fs.readFile(targetRealPath, "utf8");
  }

  return readTextFileRange(targetRealPath, startLine, requestedLimit);
}

async function realAllowedRoots(workspaceRoot: string, additionalWorkspaceRoots: string[]): Promise<string[]> {
  const roots = sanitizeAdditionalDirectories(workspaceRoot, additionalWorkspaceRoots);
  const realRoots = await Promise.all([workspaceRoot, ...roots].map((root) => fs.realpath(root)));
  return uniqueStrings(realRoots.map((root) => path.resolve(root)));
}

async function nearestExistingParentRealPath(filePath: string): Promise<string> {
  let current = path.dirname(filePath);
  while (current && current !== path.dirname(current)) {
    try {
      return await fs.realpath(current);
    } catch {
      current = path.dirname(current);
    }
  }
  return fs.realpath(current || path.parse(filePath).root);
}

function boundedTextResponse(text: string, startLine: number, requestedLimit: number | undefined): string {
  if (startLine > 1 || requestedLimit !== undefined) {
    return sliceTextRange(text, startLine, requestedLimit);
  }
  if (text.length > MAX_READ_TEXT_FILE_CHARS) {
    throw new Error("fs/read_text_file open editor buffer is too large; request a smaller line range.");
  }
  return text;
}

async function readTextFileRange(
  targetPath: string,
  startLine: number,
  requestedLimit: number | undefined
): Promise<string> {
  const stream = createReadStream(targetPath, { encoding: "utf8" });
  const lines = readline.createInterface({ input: stream, crlfDelay: Infinity });
  const content: string[] = [];
  let lineNumber = 0;
  let chars = 0;

  try {
    for await (const line of lines) {
      lineNumber += 1;
      if (lineNumber < startLine) {
        continue;
      }
      if (requestedLimit !== undefined && content.length >= requestedLimit) {
        break;
      }
      chars += line.length + (content.length ? 1 : 0);
      if (chars > MAX_READ_TEXT_FILE_CHARS) {
        throw new Error("fs/read_text_file response is too large; request a smaller line range.");
      }
      content.push(line);
    }
  } finally {
    lines.close();
    stream.destroy();
  }

  return content.join("\n");
}

function sliceTextRange(text: string, startLine: number, requestedLimit: number | undefined): string {
  if (requestedLimit === 0) {
    return "";
  }
  const startIndex = Math.max(0, startLine - 1);
  const lines = text.split(/\r\n|\r|\n/);
  const selected =
    requestedLimit === undefined
      ? lines.slice(startIndex)
      : lines.slice(startIndex, startIndex + requestedLimit);
  const content = selected.join("\n");
  if (content.length > MAX_READ_TEXT_FILE_CHARS) {
    throw new Error("fs/read_text_file response is too large; request a smaller line range.");
  }
  return content;
}

function positiveInteger(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return undefined;
  }
  return Math.max(1, Math.floor(value));
}

function nonNegativeInteger(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return undefined;
  }
  return Math.max(0, Math.floor(value));
}

function isInsideWorkspace(workspaceRoot: string, targetPath: string): boolean {
  const relative = path.relative(workspaceRoot, targetPath);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function isInsideAnyWorkspace(workspaceRoots: string[], targetPath: string): boolean {
  return workspaceRoots.some((root) => isInsideWorkspace(root, targetPath));
}

function sanitizeAdditionalDirectories(workspaceRoot: string, additionalWorkspaceRoots: string[]): string[] {
  const primary = normalizedPath(workspaceRoot);
  const seen = new Set<string>([primary]);
  const result: string[] = [];
  for (const root of additionalWorkspaceRoots) {
    if (!root || !path.isAbsolute(root)) {
      continue;
    }
    const resolved = path.resolve(root);
    const normalized = normalizedPath(resolved);
    if (seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    result.push(resolved);
  }
  return result;
}

function normalizedPath(value: string): string {
  const resolved = path.resolve(value);
  return process.platform === "win32" ? resolved.toLowerCase() : resolved;
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const normalized = normalizedPath(value);
    if (seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    result.push(value);
  }
  return result;
}

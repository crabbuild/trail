import * as path from "node:path";
import * as vscode from "vscode";
import { AcpClient, type AcpAuthMethod } from "../acp/AcpClient";
import { defaultAgentCapabilities, type AgentCapabilities } from "../acp/AcpCapabilities";
import { ProviderRegistry, type AcpProviderProfile } from "../acp/ProviderRegistry";
import type { AgentTask, TaskRepository, TaskView } from "../crabdb/TaskRepository";
import { attachmentToContentBlock, type PromptAttachment } from "../model/PromptAttachment";
import {
  attachmentFromActiveFile,
  attachmentFromDiagnostics,
  attachmentFromSelection
} from "../model/VsCodePromptAttachments";
import type { RequestPermissionParams, SessionUpdate } from "../shared/acpTypes";
import {
  applyRenderPatches,
  reducePermissionRequest,
  reduceSessionUpdate,
  sessionControlsToPatches
} from "../shared/acpRenderReducers";
import { promptCompletionNode } from "../shared/promptCompletion";
import type { RenderNode, RenderReduceContext } from "../shared/renderModel";
import { redactedJson, redactString } from "../shared/securityRedaction";
import { findTaskOverlaps, type TaskOverlap } from "../shared/taskOverlaps";
import { hydrateTaskView, mergeHydratedNodes } from "../state/crabDbHydration";
import type { DiffContentProvider } from "./DiffContentProvider";
import { laneGateLabel, promptLaneGateRequest } from "./LaneGatePrompts";
import { ResourceOpener } from "./ResourceOpener";

interface WebviewMessage {
  type: string;
  text?: string;
  title?: string;
  language?: string;
  selector?: string;
  target?: string;
  requestId?: string;
  optionId?: string;
  attachmentId?: string;
  nodeId?: string;
  uri?: string;
  path?: string;
  line?: number;
  modeId?: string;
  configId?: string;
  value?: string;
  providerId?: string;
  conflictId?: string;
}

type ApprovalStatus = "completed" | "cancelled";
interface ProviderFailureState {
  message: string;
  detail?: string | undefined;
  code?: number | null | undefined;
  occurredAt: string;
}

const MAX_TEXT_PREVIEW_CHARS = 120_000;
const MAX_TERMINAL_ATTACHMENT_CHARS = 128 * 1024;

export class ChatPanel {
  private static readonly panels = new Map<string, ChatPanel>();

  static async open(
    extensionUri: vscode.Uri,
    repository: TaskRepository,
    output: vscode.OutputChannel,
    diffProvider: DiffContentProvider,
    provider: AcpProviderProfile,
    task?: AgentTask,
    attachments: PromptAttachment[] = []
  ): Promise<ChatPanel> {
    const key = task?.lane || `new:${provider.id}`;
    const existing = ChatPanel.panels.get(key);
    if (existing) {
      existing.addAttachments(attachments);
      existing.panel.reveal(vscode.ViewColumn.Beside);
      return existing;
    }

    const panel = vscode.window.createWebviewPanel(
      "crabdb.agentChat",
      task ? `Agent: ${task.title}` : "New Agent Task",
      vscode.ViewColumn.Beside,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [vscode.Uri.joinPath(extensionUri, "dist")]
      }
    );

    const chat = new ChatPanel(extensionUri, panel, repository, output, diffProvider, provider, task, attachments);
    ChatPanel.panels.set(key, chat);
    panel.onDidDispose(() => {
      chat.dispose();
      ChatPanel.panels.delete(key);
    });
    await chat.initialize();
    return chat;
  }

  addAttachments(attachments: PromptAttachment[]): void {
    for (const attachment of attachments) {
      if (!this.attachments.some((existing) => existing.id === attachment.id)) {
        this.attachments.push(attachment);
      }
    }
    this.postState();
  }

  private nodes: RenderNode[] = [];
  private acp: AcpClient | undefined;
  private acpSessionId: string | undefined;
  private currentTurnId: string | undefined;
  private taskView?: TaskView;
  private taskOverlaps: TaskOverlap[] = [];
  private readonly attachments: PromptAttachment[] = [];
  private readonly resourceOpener: ResourceOpener;
  private capabilities: AgentCapabilities = defaultAgentCapabilities();
  private sending = false;
  private statePostTimer: ReturnType<typeof setTimeout> | undefined;
  private statePostPending = false;
  private acpStartMode: "new" | "load" | "resume" | undefined;
  private requestedAcpSessionId: string | undefined;
  private providerSwitchFrom: string | undefined;
  private forceCheckpointFollowUp = false;
  private providerFailure: ProviderFailureState | undefined;
  private recentAcpStderr: string[] = [];

  private constructor(
    private readonly extensionUri: vscode.Uri,
    private readonly panel: vscode.WebviewPanel,
    private readonly repository: TaskRepository,
    private readonly output: vscode.OutputChannel,
    private readonly diffProvider: DiffContentProvider,
    private provider: AcpProviderProfile,
    private task?: AgentTask,
    attachments: PromptAttachment[] = []
  ) {
    this.attachments.push(...attachments);
    this.resourceOpener = new ResourceOpener(repository.workspaceRoot);
  }

  private async initialize(): Promise<void> {
    this.panel.webview.html = this.html();
    this.panel.webview.onDidReceiveMessage((message: WebviewMessage) => {
      void this.handleMessage(message);
    });
    await this.refresh();
  }

  private async refresh(): Promise<void> {
    let listedTasks: AgentTask[] | undefined;
    if (!this.task) {
      try {
        listedTasks = await this.repository.listTasks();
        this.task = listedTasks[0];
      } catch {
        // No task may exist yet for a new chat.
      }
    }

    if (this.task) {
      try {
        this.taskView = await this.repository.viewTask(this.task.lane);
        this.task = this.taskView.task;
        if (!this.acpSessionId && this.task.acpSessionId) {
          this.acpSessionId = this.task.acpSessionId;
        }
        this.nodes = mergeHydratedNodes(hydrateTaskView(this.taskView), this.nodes);
      } catch (error) {
        this.post({
          type: "error",
          message: error instanceof Error ? error.message : String(error)
        });
      }
    }
    await this.refreshTaskOverlaps(listedTasks);
    this.postState();
  }

  private async handleMessage(message: WebviewMessage): Promise<void> {
    switch (message.type) {
      case "ready":
        this.postState();
        break;
      case "refresh":
        await this.refresh();
        break;
      case "sendPrompt":
        await this.sendPrompt(message.text || "");
        break;
      case "removeAttachment":
        if (message.attachmentId) {
          const index = this.attachments.findIndex((attachment) => attachment.id === message.attachmentId);
          if (index >= 0) {
            this.attachments.splice(index, 1);
            this.postState();
          }
        }
        break;
      case "attachSelection":
        this.addRequestedAttachment(attachmentFromSelection(), "Select text in an editor before attaching a selection.");
        break;
      case "attachFile":
        this.addRequestedAttachment(attachmentFromActiveFile(), "Open a workspace file before attaching the current file.");
        break;
      case "attachDiagnostics":
        this.addRequestedAttachment(attachmentFromDiagnostics(), "No diagnostics are available for the active file.");
        break;
      case "attachTerminalOutput":
        this.addRequestedAttachment(
          this.latestTerminalOutputAttachment(),
          "No terminal output is available in this chat transcript yet."
        );
        break;
      case "attachChangedFiles":
        this.addRequestedAttachment(this.changedFilesAttachment(), "No changed files are recorded for this task yet.");
        break;
      case "attachHistory":
        await this.attachHistory();
        break;
      case "switchProvider":
        if (message.providerId) {
          await this.switchProvider(message.providerId);
        }
        break;
      case "startFollowUp":
        this.startFollowUpFromFailure();
        break;
      case "showAcpLogs":
        this.output.show(true);
        break;
      case "openSettings":
        await vscode.commands.executeCommand("crabdb.openSettings");
        break;
      case "cancel":
        this.cancelCurrentTurn();
        break;
      case "approve":
        if (message.requestId && message.optionId) {
          if (this.acp) {
            this.acp.approve(message.requestId, message.optionId);
            this.resolveApproval(message.requestId, "completed");
          }
        }
        break;
      case "reject":
        if (message.requestId) {
          if (this.acp) {
            this.acp.reject(message.requestId);
            this.resolveApproval(message.requestId, "cancelled");
          }
        }
        break;
      case "dryRunApply":
        await this.runAndShow("applyDryRun", () => this.repository.dryRunApply(this.task?.lane || "latest"));
        break;
      case "queueMerge":
        await this.runAndShow("queueMerge", () => this.repository.queueMerge(this.task?.lane || "latest"));
        break;
      case "rewind":
        await this.runAndShow("rewind", () =>
          this.repository.rewind(this.task?.lane || "latest", message.target || "before-last-turn")
        );
        break;
      case "preserveFailedAttempt":
        await vscode.commands.executeCommand("crabdb.preserveFailedAttempt", this.task);
        await this.refresh();
        break;
      case "removeTask":
        await vscode.commands.executeCommand("crabdb.removeAgentTask", this.task);
        break;
      case "openDiff":
        await this.runAndShow("diff", () => this.repository.diffTask(this.task?.lane || "latest"));
        break;
      case "compareTasks":
        await this.compareTasks();
        break;
      case "showConflict": {
        const conflictId = message.conflictId;
        if (conflictId) {
          await this.runAndShow("conflictDetails", () => this.repository.showConflict(conflictId));
        }
        break;
      }
      case "runTests":
        await this.runLaneGate("test");
        break;
      case "runEvals":
        await this.runLaneGate("eval");
        break;
      case "openWorkdir":
        await this.openLaneWorkdir();
        break;
      case "openNodeDiff":
        await this.openNodeDiff(message.nodeId);
        break;
      case "openTerminal":
        await this.openTerminal(message.nodeId);
        break;
      case "openTextPreview":
        await this.openTextPreview(message);
        break;
      case "openLocation":
        if (message.path) {
          await this.openSafely(() => this.resourceOpener.openPath(message.path || "", message.line));
        }
        break;
      case "openResource":
        if (message.uri) {
          await this.openSafely(() => this.resourceOpener.openResource(message.uri || ""));
        }
        break;
      case "setMode":
        if (message.modeId) {
          await this.setMode(message.modeId);
        }
        break;
      case "setConfigOption":
        if (message.configId && message.value !== undefined) {
          await this.setConfigOption(message.configId, message.value);
        }
        break;
      default:
        break;
    }
  }

  private addRequestedAttachment(attachment: PromptAttachment | undefined, emptyMessage: string): void {
    if (!attachment) {
      this.post({ type: "status", message: emptyMessage });
      return;
    }
    this.addAttachments([attachment]);
  }

  private changedFilesAttachment(): PromptAttachment | undefined {
    const paths = this.changedPaths();
    if (!paths.length || !this.task) {
      return undefined;
    }
    return {
      id: stableAttachmentId("changed-files", this.task.lane, paths.join("\n")),
      kind: "changed-files",
      label: `Changed files for ${this.task.lane}`,
      mimeType: "text/plain",
      text: `Changed files for ${this.task.title} (${this.task.lane}):\n\n${paths.map((path) => `- ${path}`).join("\n")}`
    };
  }

  private latestTerminalOutputAttachment(): PromptAttachment | undefined {
    const terminal = [...this.nodes].reverse().find((node) => {
      if (node.kind !== "terminal") {
        return false;
      }
      return Boolean(node.output || node.stdout || node.stderr);
    }) as Extract<RenderNode, { kind: "terminal" }> | undefined;
    if (!terminal) {
      return undefined;
    }
    const text = terminalAttachmentText(terminal);
    return {
      id: stableAttachmentId("terminal-output", terminal.id, text.slice(0, 512)),
      kind: "terminal-output",
      label: `Terminal output: ${terminal.title || terminal.command || terminal.terminalId}`,
      mimeType: "text/plain",
      text
    };
  }

  private async attachHistory(): Promise<void> {
    const active = activeRelativeFilePath();
    if (!active) {
      this.post({ type: "status", message: "Open a workspace file before attaching CrabDB history." });
      return;
    }
    try {
      const history = await this.repository.history(active);
      this.addAttachments([
        {
          id: stableAttachmentId("history", active, redactedJson(history)),
          kind: "history",
          label: `CrabDB history for ${active}`,
          mimeType: "application/json",
          text: redactedJson(history)
        }
      ]);
    } catch (error) {
      this.post({ type: "error", message: error instanceof Error ? error.message : String(error) });
    }
  }

  private async switchProvider(providerId: string): Promise<void> {
    if (providerId === this.provider.id) {
      return;
    }
    if (this.sending || this.hasPendingApproval()) {
      this.post({ type: "status", message: "Finish the current turn or permission request before switching providers." });
      this.postState();
      return;
    }
    const next = new ProviderRegistry(this.repository.workspaceRoot)
      .profiles()
      .find((profile) => profile.id === providerId);
    if (!next) {
      this.post({ type: "error", message: `Provider ${providerId} is not configured.` });
      this.postState();
      return;
    }
    const previous = this.provider.label;
    this.acp?.dispose();
    this.acp = undefined;
    this.provider = next;
    this.capabilities = defaultAgentCapabilities();
    this.acpSessionId = undefined;
    this.acpStartMode = undefined;
    this.requestedAcpSessionId = undefined;
    this.providerSwitchFrom = previous;
    this.forceCheckpointFollowUp = true;
    this.providerFailure = undefined;
    this.post({ type: "status", message: `Switched provider to ${next.label}. The next prompt starts from the current CrabDB checkpoint.` });
    this.postState();
  }

  private async pickAuthMethod(methods: AcpAuthMethod[]): Promise<string | undefined> {
    if (methods.length === 0) {
      return undefined;
    }
    if (methods.length === 1) {
      const method = methods[0];
      if (!method) {
        return undefined;
      }
      const picked = await vscode.window.showInformationMessage(
        `${this.provider.label} needs authentication: ${method.name}`,
        {
          modal: true,
          detail: method.description || "The agent handles the sign-in flow."
        },
        "Continue"
      );
      return picked === "Continue" ? method.id : undefined;
    }
    const picked = await vscode.window.showQuickPick(
      methods.map((method) => ({
        label: method.name,
        description: method.id,
        detail: method.description || "The agent handles the sign-in flow.",
        method
      })),
      {
        title: `Authenticate ${this.provider.label}`,
        placeHolder: "Choose an authentication method"
      }
    );
    return picked?.method.id;
  }

  private async sendPrompt(text: string): Promise<void> {
    if (this.sending) {
      this.post({ type: "status", message: "A prompt is already running for this task." });
      return;
    }
    if (this.hasPendingApproval()) {
      this.post({ type: "status", message: "Resolve the pending permission request before sending another prompt." });
      return;
    }
    const trimmed = text.trim();
    if (!trimmed && this.attachments.length === 0) {
      return;
    }
    if (this.provider.crabdbBacked && !(await this.ensureCrabDbWorkspaceInitialized())) {
      return;
    }
    this.sending = true;
    this.postState();

    const lane = this.task?.lane || "new-task";
    this.currentTurnId = `turn-${Date.now()}`;

    try {
      if (!this.acp) {
        if (!this.provider.crabdbBacked) {
          this.post({
            type: "status",
            message: "This custom ACP command is not recognized as CrabDB-backed. Use a CrabDB relay command to keep durable task state."
          });
        }
        this.recentAcpStderr = [];
        const client = new AcpClient(this.repository.workspaceRoot, this.provider, this.output, {
          readOpenTextDocument: openTextDocumentContent,
          additionalWorkspaceRoots: additionalWorkspaceFolderPaths(this.repository.workspaceRoot)
        });
        this.acp = client;
        try {
          const session = await client.start(
            {
              update: (update) => this.handleAcpUpdate(update),
              permission: (requestId, params) => this.handlePermission(requestId, params),
              completed: (response) => this.handlePromptComplete(response),
              error: (error) => this.markProviderFailure("The agent process reported an error before the turn completed.", error.message),
              stderr: (line) => this.recordAcpStderr(line),
              exit: (code, signal) => this.handleAcpExit(code, signal),
              authenticate: (methods) => this.pickAuthMethod(methods),
              authenticated: (method) => {
                this.post({ type: "status", message: `Authenticated ${this.provider.label} with ${method.name}.` });
              }
            },
            {
              taskName: this.task?.title,
              existingSessionId: this.forceCheckpointFollowUp ? undefined : this.task?.acpSessionId,
              fromRef: this.task?.latestCheckpoint || this.task?.lane
            }
          );
          this.acpSessionId = session.sessionId;
          this.acpStartMode = session.startMode;
          this.requestedAcpSessionId = session.requestedSessionId;
          this.forceCheckpointFollowUp = false;
          this.providerFailure = undefined;
          this.capabilities = session.capabilities;
          this.nodes = applyRenderPatches(this.nodes, sessionControlsToPatches(session.session, this.renderContext()));
          if (session.requestedSessionId && session.startMode === "new") {
            this.post({
              type: "status",
              message: "Provider cannot resume this ACP session; starting a follow-up from the latest CrabDB checkpoint."
            });
          } else if (session.startMode === "load") {
            this.post({ type: "status", message: "Loaded the existing ACP session." });
          } else if (session.startMode === "resume") {
            this.post({ type: "status", message: "Resumed the existing ACP session." });
          }
          this.postState();
        } catch (error) {
          client.dispose();
          if (this.acp === client) {
            this.acp = undefined;
          }
          throw error;
        }
      }

      const context = this.renderContext(lane);
      const content = [
        ...(trimmed ? [{ type: "text" as const, text: trimmed }] : []),
        ...this.attachments.map((attachment) =>
          attachmentToContentBlock(attachment, {
            embeddedContext: this.capabilities.promptCapabilities.embeddedContext
          })
        )
      ];
      this.nodes = applyRenderPatches(this.nodes, [
        {
          type: "upsert",
          node: {
            id: `message:user:${this.currentTurnId}`,
            kind: "message",
            taskId: this.task?.id || lane,
            lane,
            turnId: this.currentTurnId,
            acpSessionId: this.acpSessionId,
            provider: this.provider.id,
            source: "acp-live",
            status: "completed",
            updatedAt: context.now(),
            role: "user",
            content,
            text: trimmed || `[${this.attachments.length} attachment${this.attachments.length === 1 ? "" : "s"}]`,
            streaming: false
          }
        }
      ]);
      this.postState();

      await this.acp.prompt(content);
      this.attachments.splice(0);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.markProviderFailure("The agent turn failed before completion.", message);
      this.post({ type: "error", message });
    } finally {
      this.sending = false;
      this.postState();
    }
  }

  private handleAcpUpdate(update: SessionUpdate): void {
    this.nodes = applyRenderPatches(this.nodes, reduceSessionUpdate(update, this.renderContext()));
    this.postState();
  }

  private handlePermission(requestId: string, params: RequestPermissionParams): void {
    this.nodes = applyRenderPatches(this.nodes, reducePermissionRequest(requestId, params, this.renderContext()));
    const title = params.toolCall.title || params.toolCall.kind || "tool call";
    vscode.window.showWarningMessage(`Agent permission required: ${title}`);
    this.postState();
  }

  private async ensureCrabDbWorkspaceInitialized(): Promise<boolean> {
    if (this.repository.isWorkspaceInitialized()) {
      return true;
    }
    const message = `CrabDB is not initialized for ${this.repository.workspaceRoot}.`;
    const action = await vscode.window.showErrorMessage(
      `${message} Initialize the workspace before starting a CrabDB-backed agent.`,
      "Initialize CrabDB"
    );
    if (action === "Initialize CrabDB") {
      try {
        await vscode.commands.executeCommand("crabdb.initWorkspace");
        if (this.repository.isWorkspaceInitialized()) {
          return true;
        }
      } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        this.post({ type: "error", message: detail });
      }
    }
    this.post({
      type: "error",
      message: "Initialize the CrabDB workspace before sending a prompt."
    });
    this.providerFailure = {
      message: "CrabDB workspace is not initialized.",
      detail: `${message} Run the "CrabDB: Initialize Workspace" command, then retry the prompt.`,
      occurredAt: new Date().toISOString()
    };
    this.postState();
    return false;
  }

  private cancelCurrentTurn(): void {
    const cancelledRequests = this.acp?.cancel() ?? [];
    if (cancelledRequests.length) {
      this.resolveApprovals(cancelledRequests, "cancelled");
      return;
    }
    this.postState();
  }

  private resolveApproval(requestId: string, status: ApprovalStatus): void {
    this.resolveApprovals([requestId], status);
  }

  private resolveApprovals(requestIds: string[], status: ApprovalStatus): void {
    const requestSet = new Set(requestIds);
    const updatedAt = new Date().toISOString();
    this.nodes = this.nodes.map((node) =>
      node.kind === "approval" && requestSet.has(node.requestId)
        ? {
            ...node,
            status,
            updatedAt
          }
        : node
    );
    this.postState();
  }

  private hasPendingApproval(): boolean {
    return this.nodes.some((node) => node.kind === "approval" && node.status === "pending");
  }

  private handlePromptComplete(response: unknown): void {
    this.providerFailure = undefined;
    const completion = promptCompletionNode(response, this.renderContext());
    this.nodes = this.finalizeCurrentTurn(completion.status);
    this.nodes = applyRenderPatches(this.nodes, [
      {
        type: "upsert",
        node: completion
      }
    ]);
    this.postState();
    void this.refresh();
  }

  private finalizeCurrentTurn(status: RenderNode["status"]): RenderNode[] {
    const turnId = this.currentTurnId;
    if (!turnId) {
      return this.nodes;
    }
    const nodeStatus = status === "pending" ? "completed" : status;
    return this.nodes.map((node) => {
      if (node.turnId !== turnId || node.source !== "acp-live") {
        return node;
      }
      if (node.kind === "message") {
        return {
          ...node,
          status: nodeStatus,
          streaming: false
        };
      }
      if (node.kind === "thought") {
        return {
          ...node,
          status: nodeStatus
        };
      }
      return node;
    });
  }

  private handleAcpExit(code: number | null, signal: NodeJS.Signals | null): void {
    const message = acpExitStatusMessage(code, signal);
    this.output.appendLine(message);
    const detail = this.acpExitFailureDetail(message);
    if (this.sending || this.hasPendingApproval()) {
      this.markProviderFailure("The agent process exited before the turn completed.", detail, code);
      return;
    }
    this.acp = undefined;
    this.post({ type: "status", message });
    this.postState();
  }

  private recordAcpStderr(line: string): void {
    const redacted = redactString(line.trim());
    if (!redacted) {
      return;
    }
    this.recentAcpStderr.push(redacted);
    if (this.recentAcpStderr.length > 20) {
      this.recentAcpStderr.splice(0, this.recentAcpStderr.length - 20);
    }
  }

  private acpExitFailureDetail(message: string): string {
    if (!this.recentAcpStderr.length) {
      return message;
    }
    return `${message}\n\nRecent stderr:\n${this.recentAcpStderr.slice(-8).join("\n")}`;
  }

  private markProviderFailure(message: string, detail?: string | undefined, code?: number | null | undefined): void {
    const occurredAt = new Date().toISOString();
    this.providerFailure = {
      message,
      detail: detail ? redactString(detail) : undefined,
      code,
      occurredAt
    };
    this.nodes = this.finalizeCurrentTurn("failed").map((node) =>
      node.kind === "approval" && node.status === "pending"
        ? {
            ...node,
            status: "cancelled",
            updatedAt: occurredAt
          }
        : node
    );
    this.sending = false;
    this.acp?.dispose();
    this.acp = undefined;
    this.forceCheckpointFollowUp = true;
    this.postState();
  }

  private startFollowUpFromFailure(): void {
    this.acp?.dispose();
    this.acp = undefined;
    this.forceCheckpointFollowUp = true;
    this.providerFailure = undefined;
    this.post({ type: "status", message: "The next prompt will start a follow-up from the latest CrabDB checkpoint." });
    this.postState();
  }

  private async openNodeDiff(nodeId: string | undefined): Promise<void> {
    if (!nodeId) {
      return;
    }
    const node = this.nodes.find((candidate) => candidate.id === nodeId);
    if (!node || node.kind !== "diff") {
      this.post({ type: "error", message: "The selected diff is no longer available." });
      return;
    }
    await this.diffProvider.openDiff(node.path, node.oldText || "", node.newText);
  }

  private async openTerminal(nodeId: string | undefined): Promise<void> {
    if (!nodeId) {
      return;
    }
    const node = this.nodes.find((candidate) => candidate.id === nodeId);
    if (!node || node.kind !== "terminal") {
      this.post({ type: "error", message: "The selected terminal output is no longer available." });
      return;
    }
    await this.openSafely(async () => {
      const cwd = node.cwd ? this.resolveTerminalCwd(node.cwd) : this.repository.workspaceRoot;
      if (!insideWorkspace(this.repository.workspaceRoot, cwd)) {
        const confirmed = await vscode.window.showWarningMessage(
          "Open terminal outside this workspace?",
          {
            modal: true,
            detail: cwd
          },
          "Open terminal"
        );
        if (confirmed !== "Open terminal") {
          return;
        }
      }
      await vscode.workspace.fs.stat(vscode.Uri.file(cwd));
      const terminal = vscode.window.createTerminal({
        name: node.title || node.terminalId || "CrabDB Agent",
        cwd
      });
      terminal.show();
      if (node.command) {
        terminal.sendText(redactString(node.command), false);
      }
    });
  }

  private resolveTerminalCwd(cwd: string): string {
    return path.isAbsolute(cwd) ? cwd : path.resolve(this.repository.workspaceRoot, cwd);
  }

  private async openTextPreview(message: WebviewMessage): Promise<void> {
    const text = typeof message.text === "string" ? message.text : "";
    if (!text) {
      this.post({ type: "error", message: "The selected preview is empty." });
      return;
    }
    const content =
      text.length > MAX_TEXT_PREVIEW_CHARS
        ? `${text.slice(0, MAX_TEXT_PREVIEW_CHARS)}\n\n[preview truncated]`
        : text;
    const document = await vscode.workspace.openTextDocument({
      content,
      language: safePreviewLanguage(message.language)
    });
    await vscode.window.showTextDocument(document, { preview: true });
    if (message.title) {
      this.post({ type: "status", message: `Opened preview: ${message.title}` });
    }
  }

  private async runAndShow(type: string, action: () => Promise<unknown>): Promise<void> {
    try {
      const result = await action();
      this.post({ type, result });
    } catch (error) {
      this.post({ type: "error", message: error instanceof Error ? error.message : String(error) });
    }
  }

  private async runLaneGate(kind: "test" | "eval"): Promise<void> {
    const label = laneGateLabel(kind);
    let request;
    try {
      request = await promptLaneGateRequest(kind);
    } catch (error) {
      this.post({ type: "error", message: error instanceof Error ? error.message : String(error) });
      return;
    }
    if (!request) {
      return;
    }
    const lane = this.task?.lane || "latest";
    const resultType = kind === "test" ? "laneTest" : "laneEval";
    this.post({ type: "status", message: `Running lane ${label} in ${lane}.` });
    await this.runAndShow(resultType, async () => {
      const result =
        kind === "test"
          ? await this.repository.runLaneTest(lane, request)
          : await this.repository.runLaneEval(lane, request);
      await this.refresh();
      return result;
    });
  }

  private async compareTasks(): Promise<void> {
    try {
      const tasks = await this.repository.listTasks();
      if (tasks.length < 2) {
        this.post({ type: "status", message: "At least two CrabDB agent tasks are required to compare." });
        return;
      }
      const left = this.task ?? (await pickTask(tasks, "Choose left agent task"));
      if (!left) {
        return;
      }
      const right = await pickTask(
        tasks.filter((candidate) => candidate.lane !== left.lane),
        `Compare ${left.title} with`
      );
      if (!right) {
        return;
      }
      const result = await this.repository.compareTasks(left.lane, right.lane);
      this.post({ type: "compareTasks", result });
    } catch (error) {
      this.post({ type: "error", message: error instanceof Error ? error.message : String(error) });
    }
  }

  private async refreshTaskOverlaps(listedTasks?: AgentTask[] | undefined): Promise<void> {
    if (!this.task) {
      this.taskOverlaps = [];
      return;
    }
    try {
      const tasks = listedTasks ?? (await this.repository.listTasks());
      this.taskOverlaps = findTaskOverlaps(this.task, tasks);
    } catch {
      this.taskOverlaps = [];
    }
  }

  private async openLaneWorkdir(): Promise<void> {
    const lane = this.task?.lane || "latest";
    await this.openSafely(async () => {
      const workdir = await this.repository.laneWorkdir(lane);
      if (!workdir) {
        this.post({ type: "status", message: `Lane ${lane} has no materialized workdir.` });
        return;
      }
      const uri = vscode.Uri.file(workdir);
      await vscode.workspace.fs.stat(uri);
      await vscode.commands.executeCommand("vscode.openFolder", uri, { forceNewWindow: true });
      this.post({ type: "status", message: `Opened lane workdir: ${workdir}` });
    });
  }

  private async openSafely(action: () => Promise<void>): Promise<void> {
    try {
      await action();
    } catch (error) {
      this.post({ type: "error", message: error instanceof Error ? error.message : String(error) });
    }
  }

  private async setMode(modeId: string): Promise<void> {
    if (!this.acp) {
      this.post({ type: "status", message: "Start an ACP session before changing mode." });
      return;
    }
    try {
      const result = await this.acp.setMode(modeId);
      const patches = sessionControlsToPatches(result, this.renderContext());
      this.nodes = patches.length ? applyRenderPatches(this.nodes, patches) : this.updateCurrentMode(modeId);
      this.postState();
    } catch (error) {
      this.post({ type: "error", message: error instanceof Error ? error.message : String(error) });
    }
  }

  private updateCurrentMode(modeId: string): RenderNode[] {
    return this.nodes.map((node) => (node.kind === "mode" ? { ...node, modeId, updatedAt: new Date().toISOString() } : node));
  }

  private async setConfigOption(configId: string, value: string): Promise<void> {
    if (!this.acp) {
      this.post({ type: "status", message: "Start an ACP session before changing configuration." });
      return;
    }
    try {
      const result = await this.acp.setConfigOption(configId, value);
      this.nodes = applyRenderPatches(this.nodes, sessionControlsToPatches(result, this.renderContext()));
      this.postState();
    } catch (error) {
      this.post({ type: "error", message: error instanceof Error ? error.message : String(error) });
    }
  }

  private postState(): void {
    if (this.statePostTimer) {
      this.statePostPending = true;
      return;
    }
    this.post(this.stateMessage());
    this.statePostTimer = setTimeout(() => {
      this.statePostTimer = undefined;
      if (this.statePostPending) {
        this.statePostPending = false;
        this.postState();
      }
    }, 50);
  }

  private stateMessage(): unknown {
    return {
      type: "state",
      task: this.task,
      taskView: this.taskView,
      taskOverlaps: this.taskOverlaps,
      nodes: this.nodes,
      attachments: this.attachments,
      sending: this.sending,
      provider: this.provider.label,
      providerId: this.provider.id,
      providers: new ProviderRegistry(this.repository.workspaceRoot).profiles().map((profile) => ({
        id: profile.id,
        label: profile.label,
        crabdbBacked: profile.crabdbBacked
      })),
      acpSessionId: this.acpSessionId,
      persistedAcpSessionId: this.task?.acpSessionId,
      acpStartMode: this.acpStartMode,
      requestedAcpSessionId: this.requestedAcpSessionId,
      providerSwitchFrom: this.providerSwitchFrom,
      providerFailure: this.providerFailure,
      capabilities: this.capabilities,
      permissionPending: this.hasPendingApproval()
    };
  }

  private renderContext(lane = this.task?.lane || "new-task"): RenderReduceContext {
    return {
      taskId: this.task?.id || lane,
      lane,
      acpSessionId: this.acpSessionId,
      currentTurnId: this.currentTurnId,
      provider: this.provider.id,
      now: () => new Date().toISOString()
    };
  }

  private post(message: unknown): void {
    void this.panel.webview.postMessage(message);
  }

  private html(): string {
    const script = this.panel.webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "dist", "webview.js"));
    const style = this.panel.webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "dist", "webview.css"));
    const nonce = nonceValue();
    const csp = [
      "default-src 'none'",
      `img-src ${this.panel.webview.cspSource} data:`,
      `style-src ${this.panel.webview.cspSource}`,
      `script-src 'nonce-${nonce}'`
    ].join("; ");

    return `<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta http-equiv="Content-Security-Policy" content="${escapeHtml(csp)}">
  <link rel="stylesheet" href="${style}">
  <title>CrabDB Agent Chat</title>
</head>
<body>
  <main id="app" aria-label="CrabDB agent chat"></main>
  <script nonce="${nonce}" src="${script}"></script>
</body>
</html>`;
  }

  private dispose(): void {
    if (this.statePostTimer) {
      clearTimeout(this.statePostTimer);
      this.statePostTimer = undefined;
    }
    this.acp?.dispose();
  }

  private changedPaths(): string[] {
    const taskPaths = this.task?.changedPaths ?? [];
    const view = this.taskView;
    const rawChanges = view?.changes ?? [];
    return uniqueStrings(
      taskPaths.concat(
        rawChanges
          .map((item) => {
            const record = asRecord(item);
            return typeof record.path === "string" ? record.path : typeof item === "string" ? item : "";
          })
          .filter(Boolean)
      )
    );
  }
}

function nonceValue(): string {
  const alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  let value = "";
  for (let index = 0; index < 32; index += 1) {
    value += alphabet[Math.floor(Math.random() * alphabet.length)];
  }
  return value;
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      default:
        return "&#39;";
    }
  });
}

function activeRelativeFilePath(): string | undefined {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.uri.scheme !== "file") {
    return undefined;
  }
  return vscode.workspace.asRelativePath(editor.document.uri, false);
}

function openTextDocumentContent(filePath: string): string | undefined {
  const requestedPath = normalizeFilePath(filePath);
  const document = vscode.workspace.textDocuments.find((candidate) => {
    if (candidate.uri.scheme !== "file") {
      return false;
    }
    return normalizeFilePath(candidate.uri.fsPath) === requestedPath;
  });
  return document?.getText();
}

function additionalWorkspaceFolderPaths(primaryRoot: string): string[] {
  const primary = normalizeFilePath(primaryRoot);
  return (
    vscode.workspace.workspaceFolders
      ?.map((folder) => folder.uri.fsPath)
      .filter((folderPath) => normalizeFilePath(folderPath) !== primary) ?? []
  );
}

function normalizeFilePath(filePath: string): string {
  const normalized = path.resolve(filePath);
  return process.platform === "win32" ? normalized.toLowerCase() : normalized;
}

function terminalAttachmentText(node: Extract<RenderNode, { kind: "terminal" }>): string {
  const lines = ["Terminal output from CrabDB ACP transcript"];
  if (node.title) {
    lines.push(`Title: ${node.title}`);
  }
  if (node.command) {
    lines.push(`Command: ${redactString(node.command)}`);
  }
  if (node.cwd) {
    lines.push(`Cwd: ${node.cwd}`);
  }
  lines.push(`Status: ${node.terminalStatus || node.status}`);
  if (typeof node.exitCode === "number") {
    lines.push(`Exit code: ${node.exitCode}`);
  }
  if (typeof node.elapsedMs === "number") {
    lines.push(`Elapsed: ${node.elapsedMs} ms`);
  }

  const sections = [
    terminalOutputAttachmentSection("Output", node.output),
    terminalOutputAttachmentSection("Stdout", node.stdout),
    terminalOutputAttachmentSection("Stderr", node.stderr)
  ].filter(Boolean);
  const text = `${lines.join("\n")}\n\n${sections.join("\n\n")}`;
  return truncateAttachmentText(text, MAX_TERMINAL_ATTACHMENT_CHARS);
}

function terminalOutputAttachmentSection(label: string, value: string | undefined): string {
  if (!value) {
    return "";
  }
  return `${label}:\n${redactString(value)}`;
}

function truncateAttachmentText(text: string, limit: number): string {
  if (text.length <= limit) {
    return text;
  }
  return `${text.slice(0, limit)}\n\n[CrabDB VS Code truncated this attachment to ${limit} characters.]`;
}

function stableAttachmentId(...parts: string[]): string {
  let hash = 0;
  const input = parts.join("\0");
  for (let index = 0; index < input.length; index += 1) {
    hash = (hash * 31 + input.charCodeAt(index)) >>> 0;
  }
  return `att-${hash.toString(16)}`;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (!value || seen.has(value)) {
      continue;
    }
    seen.add(value);
    result.push(value);
  }
  return result;
}

function safePreviewLanguage(value: string | undefined): string {
  const cleaned = (value || "plaintext").replace(/[^a-zA-Z0-9_+.-]/g, "").slice(0, 40);
  return cleaned || "plaintext";
}

function insideWorkspace(workspaceRoot: string, candidate: string): boolean {
  const relative = path.relative(workspaceRoot, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function acpExitStatusMessage(code: number | null, signal: NodeJS.Signals | null): string {
  if (typeof code === "number") {
    return `ACP process exited with code ${code}`;
  }
  if (signal) {
    return `ACP process exited with signal ${signal}`;
  }
  return "ACP process exited without an exit code or signal";
}

async function pickTask(tasks: AgentTask[], title: string): Promise<AgentTask | undefined> {
  const picked = await vscode.window.showQuickPick(
    tasks.map((task) => ({
      label: task.title,
      description: task.lane,
      detail: `${task.status}${task.changedPaths.length ? ` - ${task.changedPaths.length} changed paths` : ""}`,
      task
    })),
    { title }
  );
  return picked?.task;
}

import * as assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";
import { TaskRepository } from "../../../crabdb/TaskRepository";
import { ChatPanel } from "../../../views/ChatPanel";
import { DiffContentProvider } from "../../../views/DiffContentProvider";

export async function run(): Promise<void> {
  const extension = vscode.extensions.getExtension("crabdb.crabdb-vscode");
  assert.ok(extension, "CrabDB extension should be discoverable by VS Code");

  await extension.activate();
  assert.equal(extension.isActive, true);

  const commands = await vscode.commands.getCommands(true);
  for (const command of [
    "crabdb.initWorkspace",
    "crabdb.newAgentTask",
    "crabdb.openAgentChat",
    "crabdb.openLatestReview",
    "crabdb.applyLatestDryRun",
    "crabdb.queueMerge",
    "crabdb.explainQueueEntry",
    "crabdb.runMergeQueue",
    "crabdb.removeQueueEntry",
    "crabdb.rewindTask",
    "crabdb.preserveFailedAttempt",
    "crabdb.removeAgentTask",
    "crabdb.runLaneTest",
    "crabdb.runLaneEval",
    "crabdb.openLaneWorkdir",
    "crabdb.compareTasks",
    "crabdb.refreshTasks",
    "crabdb.startDaemon",
    "crabdb.doctor",
    "crabdb.addAcpProvider",
    "crabdb.askSelection",
    "crabdb.attachSelection",
    "crabdb.showLineHistory",
    "crabdb.showFileChanges"
  ]) {
    assert.ok(commands.includes(command), `${command} should be contributed`);
  }

  const config = vscode.workspace.getConfiguration("crabdb");
  assert.equal(typeof config.get("path"), "string");
  assert.equal(typeof config.get("defaultProvider"), "string");

  await runFakeAcpChatSmoke(extension);
  await runPermissionAcpChatSmoke(extension);
}

async function runFakeAcpChatSmoke(extension: vscode.Extension<unknown>): Promise<void> {
  const workspaceRoot = process.env.CRABDB_VSCODE_TEST_WORKSPACE;
  assert.ok(workspaceRoot, "CRABDB_VSCODE_TEST_WORKSPACE should point at the disposable workspace");

  const output = vscode.window.createOutputChannel("CrabDB Agents Test");
  const repository = new TaskRepository(workspaceRoot, output);
  const diffProvider = new DiffContentProvider();
  const agent = writeStubAcpAgent(workspaceRoot);
  const chat = await ChatPanel.open(
    extension.extensionUri,
    repository,
    output,
    diffProvider,
    {
      id: "vscode-smoke",
      label: "VS Code Smoke via CrabDB",
      command: "crabdb",
      args: [
        "--workspace",
        workspaceRoot,
        "agent",
        "acp",
        "--provider",
        "claude-code",
        "--no-mcp",
        "--",
        process.execPath,
        agent
      ],
      crabdbBacked: true,
      supportsTaskName: false,
      supportsFromRef: false
    }
  );

  try {
    await (chat as unknown as { handleMessage(message: unknown): Promise<void> }).handleMessage({
      type: "sendPrompt",
      text: "Change README from the VS Code ACP chat smoke test."
    });
    await (chat as unknown as { refresh(): Promise<void> }).refresh();

    const task = await waitForValue(async () => {
      const latest = await repository.latestTask();
      return latest?.changedPaths.includes("README.md") ? latest : undefined;
    }, "CrabDB should record a task with README.md changed");
    const view = await repository.viewTask(task.lane);
    const diff = await repository.diffTask(task.lane);
    const workdir = view.task.workdir ?? (await repository.laneWorkdir(view.task.lane));
    const state = (chat as unknown as { stateMessage(): Record<string, unknown> }).stateMessage();
    const stateNodes = Array.isArray(state.nodes) ? state.nodes : [];

    assert.ok(workdir, "CrabDB should expose the materialized lane workdir");
    assert.equal(state.sending, false);
    assert.equal(state.permissionPending, false);
    assert.ok(typeof state.acpSessionId === "string" && state.acpSessionId.length > 0);
    assert.ok(stateNodes.some((node) => isNodeKind(node, "message")), "chat state should include transcript messages");
    assert.ok(stateNodes.some((node) => isNodeKind(node, "completion")), "chat state should include completion footer");
    assert.match(JSON.stringify(view.raw), /VS Code ACP chat smoke test/);
    assert.match(JSON.stringify(diff), /README\.md/);
    assert.equal(fs.readFileSync(path.join(workdir, "README.md"), "utf8"), "changed by VS Code ACP chat smoke test\n");
  } finally {
    (chat as unknown as { panel?: vscode.WebviewPanel }).panel?.dispose();
    output.dispose();
  }
}

async function runPermissionAcpChatSmoke(extension: vscode.Extension<unknown>): Promise<void> {
  const workspaceRoot = process.env.CRABDB_VSCODE_TEST_WORKSPACE;
  assert.ok(workspaceRoot, "CRABDB_VSCODE_TEST_WORKSPACE should point at the disposable workspace");

  const output = vscode.window.createOutputChannel("CrabDB Agents Permission Test");
  const repository = new TaskRepository(workspaceRoot, output);
  const diffProvider = new DiffContentProvider();
  const agent = writePermissionStubAcpAgent(workspaceRoot);
  const chat = await ChatPanel.open(
    extension.extensionUri,
    repository,
    output,
    diffProvider,
    {
      id: "vscode-permission-smoke",
      label: "VS Code Permission Smoke via CrabDB",
      command: "crabdb",
      args: [
        "--workspace",
        workspaceRoot,
        "agent",
        "acp",
        "--provider",
        "claude-code",
        "--name",
        "permission-smoke",
        "--no-mcp",
        "--",
        process.execPath,
        agent
      ],
      crabdbBacked: true,
      supportsTaskName: false,
      supportsFromRef: false
    }
  );

  try {
    const prompt = (chat as unknown as { handleMessage(message: unknown): Promise<void> }).handleMessage({
      type: "sendPrompt",
      text: "Request permission before changing README from the VS Code chat."
    });
    const approval = await waitForValue(async () => {
      const state = (chat as unknown as { stateMessage(): Record<string, unknown> }).stateMessage();
      const nodes = Array.isArray(state.nodes) ? state.nodes : [];
      const approval = nodes.find((node) => isNodeKind(node, "approval")) as
        | { requestId?: unknown; status?: unknown; options?: Array<{ optionId?: string }> }
        | undefined;
      return approval && state.permissionPending === true ? approval : undefined;
    }, "ChatPanel should expose a pending permission request");

    assert.equal(approval.status, "pending");
    assert.ok(approval.requestId, "approval node should include the ACP request id");
    await (chat as unknown as { handleMessage(message: unknown): Promise<void> }).handleMessage({
      type: "approve",
      requestId: String(approval.requestId),
      optionId: approval.options?.[0]?.optionId || "allow"
    });
    const approvedState = (chat as unknown as { stateMessage(): Record<string, unknown> }).stateMessage();
    const approvedNodes = Array.isArray(approvedState.nodes) ? approvedState.nodes : [];
    const approved = approvedNodes.find((node) => isNodeKind(node, "approval")) as { status?: unknown } | undefined;
    assert.equal(approved?.status, "completed");
    await prompt;
    await (chat as unknown as { refresh(): Promise<void> }).refresh();

    const state = (chat as unknown as { stateMessage(): Record<string, unknown> }).stateMessage();
    const view = await waitForValue(async () => {
      const tasks = await repository.listTasks();
      for (const candidate of tasks) {
        if (!candidate.changedPaths.includes("README.md")) {
          continue;
        }
        const candidateView = await repository.viewTask(candidate.lane);
        if (/approval granted in VS Code chat/.test(JSON.stringify(candidateView.raw))) {
          return candidateView;
        }
      }
      return undefined;
    }, "CrabDB should record the approved permission write");
    const workdir = view.task.workdir ?? (await repository.laneWorkdir(view.task.lane));

    assert.ok(workdir, "permission smoke should expose the lane workdir");
    assert.equal(state.permissionPending, false);
    assert.match(JSON.stringify(view.raw), /approval granted in VS Code chat/);
    assert.equal(fs.readFileSync(path.join(workdir, "README.md"), "utf8"), "approval granted in VS Code chat\n");
  } finally {
    (chat as unknown as { panel?: vscode.WebviewPanel }).panel?.dispose();
    output.dispose();
  }
}

function writeStubAcpAgent(workspaceRoot: string): string {
  const agent = path.join(workspaceRoot, "vscode-smoke-acp-agent.js");
  fs.writeFileSync(
    agent,
    `const fs = require("node:fs");
const path = require("node:path");
const readline = require("node:readline");

let sessionId = "sess_vscode_smoke";
let cwd = process.cwd();
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\\n");
}

function notify(update) {
  send({ method: "session/update", params: { sessionId, update } });
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({ id: message.id, result: { protocolVersion: 1, agentCapabilities: {} } });
    return;
  }
  if (message.method === "session/new" || message.method === "session/load" || message.method === "session/resume") {
    cwd = message.params && typeof message.params.cwd === "string" ? message.params.cwd : cwd;
    send({ id: message.id, result: { sessionId } });
    return;
  }
  if (message.method === "session/prompt") {
    notify({
      sessionUpdate: "tool_call",
      toolCallId: "tool_vscode_smoke",
      title: "write README",
      kind: "edit",
      status: "pending"
    });
    fs.mkdirSync(cwd, { recursive: true });
    fs.writeFileSync(path.join(cwd, "README.md"), "changed by VS Code ACP chat smoke test\\n");
    notify({
      sessionUpdate: "tool_call_update",
      toolCallId: "tool_vscode_smoke",
      status: "completed"
    });
    notify({
      sessionUpdate: "agent_message_chunk",
      messageId: "msg_vscode_smoke",
      content: { type: "text", text: "Changed README through the VS Code ACP chat smoke test." }
    });
    send({ id: message.id, result: { stopReason: "end_turn" } });
    setTimeout(() => process.exit(0), 20);
  }
});
`
  );
  return agent;
}

function writePermissionStubAcpAgent(workspaceRoot: string): string {
  const agent = path.join(workspaceRoot, "vscode-permission-acp-agent.js");
  fs.writeFileSync(
    agent,
    `const fs = require("node:fs");
const path = require("node:path");
const readline = require("node:readline");

let sessionId = "sess_vscode_permission";
let cwd = process.cwd();
let pendingPromptId = null;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\\n");
}

function notify(update) {
  send({ method: "session/update", params: { sessionId, update } });
}

function finishPrompt() {
  fs.mkdirSync(cwd, { recursive: true });
  fs.writeFileSync(path.join(cwd, "README.md"), "approval granted in VS Code chat\\n");
  notify({
    sessionUpdate: "agent_message_chunk",
    messageId: "msg_vscode_permission",
    content: { type: "text", text: "approval granted in VS Code chat" }
  });
  send({ id: pendingPromptId, result: { stopReason: "end_turn" } });
  setTimeout(() => process.exit(0), 20);
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({ id: message.id, result: { protocolVersion: 1, agentCapabilities: {} } });
    return;
  }
  if (message.method === "session/new" || message.method === "session/load" || message.method === "session/resume") {
    cwd = message.params && typeof message.params.cwd === "string" ? message.params.cwd : cwd;
    send({ id: message.id, result: { sessionId } });
    return;
  }
  if (message.method === "session/prompt") {
    pendingPromptId = message.id;
    send({
      id: "permission-vscode-smoke",
      method: "session/request_permission",
      params: {
        sessionId,
        toolCall: {
          sessionUpdate: "tool_call",
          toolCallId: "tool_permission_smoke",
          title: "write README after approval",
          kind: "edit",
          status: "pending",
          locations: [{ path: "README.md", line: 1 }]
        },
        options: [{ optionId: "allow", kind: "allow_once", name: "Allow once", description: "Allow README update" }]
      }
    });
    return;
  }
  if (message.id === "permission-vscode-smoke") {
    finishPrompt();
  }
});
`
  );
  return agent;
}

async function waitForValue<T>(load: () => Promise<T | undefined>, message: string, timeoutMs = 10000): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown;
  while (Date.now() < deadline) {
    try {
      const value = await load();
      if (value !== undefined) {
        return value;
      }
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`${message}${lastError instanceof Error ? `: ${lastError.message}` : ""}`);
}

function isNodeKind(node: unknown, kind: string): boolean {
  return Boolean(node && typeof node === "object" && (node as { kind?: unknown }).kind === kind);
}

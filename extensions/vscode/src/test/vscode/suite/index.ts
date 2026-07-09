import * as assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";
import { TaskRepository } from "../../../trail/TaskRepository";
import { ChatPanel } from "../../../views/ChatPanel";
import { DiffContentProvider } from "../../../views/DiffContentProvider";

export async function run(): Promise<void> {
  const extension = vscode.extensions.getExtension("trail.trail-vscode");
  assert.ok(extension, "Trail extension should be discoverable by VS Code");

  await extension.activate();
  assert.equal(extension.isActive, true);

  const commands = await vscode.commands.getCommands(true);
  for (const command of [
    "trail.initWorkspace",
    "trail.newAgentTask",
    "trail.openAgentChat",
    "trail.openLatestReview",
    "trail.applyLatestDryRun",
    "trail.queueMerge",
    "trail.explainQueueEntry",
    "trail.runMergeQueue",
    "trail.removeQueueEntry",
    "trail.rewindTask",
    "trail.preserveFailedAttempt",
    "trail.removeAgentTask",
    "trail.runLaneTest",
    "trail.runLaneEval",
    "trail.openLaneWorkdir",
    "trail.compareTasks",
    "trail.refreshTasks",
    "trail.startDaemon",
    "trail.doctor",
    "trail.openSettings",
    "trail.addAcpProvider",
    "trail.askSelection",
    "trail.attachSelection",
    "trail.showLineHistory",
    "trail.showFileChanges"
  ]) {
    assert.ok(commands.includes(command), `${command} should be contributed`);
  }

  const config = vscode.workspace.getConfiguration("trail");
  assert.equal(typeof config.get("path"), "string");
  assert.equal(typeof config.get("defaultProvider"), "string");
  assertSplitWebviewAssets(extension);

  await runFakeAcpChatSmoke(extension);
  await runPermissionAcpChatSmoke(extension);
}

async function runFakeAcpChatSmoke(extension: vscode.Extension<unknown>): Promise<void> {
  const workspaceRoot = process.env.TRAIL_VSCODE_TEST_WORKSPACE;
  assert.ok(workspaceRoot, "TRAIL_VSCODE_TEST_WORKSPACE should point at the disposable workspace");

  const output = vscode.window.createOutputChannel("Trail Agents Test");
  const repository = new TaskRepository(workspaceRoot, output);
  const diffProvider = new DiffContentProvider();
  const agent = writeStubAcpAgent(workspaceRoot);
  const provider = {
    id: "vscode-smoke",
    label: "VS Code Smoke via Trail",
    command: "trail",
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
    trailBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };
  const chat = await ChatPanel.open(
    extension.extensionUri,
    repository,
    output,
    diffProvider,
    provider
  );

  try {
    assertChatWebviewShell(chat);
    await (chat as unknown as { handleMessage(message: unknown): Promise<void> }).handleMessage({
      type: "sendPrompt",
      text: "Change README from the VS Code ACP chat smoke test."
    });
    await (chat as unknown as { refresh(): Promise<void> }).refresh();

    const task = await waitForValue(async () => {
      const latest = await repository.latestTask();
      return latest?.changedPaths.includes("README.md") ? latest : undefined;
    }, "Trail should record a task with README.md changed");
    const view = await repository.viewTask(task.lane);
    const diff = await repository.diffTask(task.lane);
    const workdir = view.task.workdir ?? (await repository.laneWorkdir(view.task.lane));
    const state = (chat as unknown as { stateMessage(): Record<string, unknown> }).stateMessage();
    const stateNodes = Array.isArray(state.nodes) ? state.nodes : [];

    assert.ok(workdir, "Trail should expose the materialized lane workdir");
    assert.equal(state.sending, false);
    assert.equal(state.permissionPending, false);
    assert.ok(typeof state.acpSessionId === "string" && state.acpSessionId.length > 0);
    assert.ok(stateNodes.some((node) => isNodeKind(node, "message")), "chat state should include transcript messages");
    assert.ok(stateNodes.some((node) => isNodeSource(node, "trail")), "chat state should include Trail-hydrated transcript nodes");
    assert.equal(
      stateNodes.some((node) => isNodeKind(node, "completion") && Boolean((node as { checkpointPending?: unknown }).checkpointPending)),
      false,
      "chat state should drop pending live completion placeholders after Trail hydration"
    );
    assert.match(JSON.stringify(view.raw), /VS Code ACP chat smoke test/);
    assert.match(JSON.stringify(diff), /README\.md/);
    assert.equal(fs.readFileSync(path.join(workdir, "README.md"), "utf8"), "changed by VS Code ACP chat smoke test\n");

    const freshChat = await ChatPanel.open(extension.extensionUri, repository, output, diffProvider, provider);
    try {
      assert.notEqual(freshChat, chat, "New Agent Task should open a fresh draft after the previous draft became a lane");
      const freshState = (freshChat as unknown as { stateMessage(): Record<string, unknown> }).stateMessage();
      assert.equal(freshState.task, undefined, "fresh New Agent Task draft should not hydrate the latest task");
      assert.equal(freshState.taskView, undefined, "fresh New Agent Task draft should not carry the latest task view");
    } finally {
      (freshChat as unknown as { panel?: vscode.WebviewPanel }).panel?.dispose();
    }
  } finally {
    (chat as unknown as { panel?: vscode.WebviewPanel }).panel?.dispose();
    output.dispose();
  }
}

async function runPermissionAcpChatSmoke(extension: vscode.Extension<unknown>): Promise<void> {
  const workspaceRoot = process.env.TRAIL_VSCODE_TEST_WORKSPACE;
  assert.ok(workspaceRoot, "TRAIL_VSCODE_TEST_WORKSPACE should point at the disposable workspace");

  const output = vscode.window.createOutputChannel("Trail Agents Permission Test");
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
      label: "VS Code Permission Smoke via Trail",
      command: "trail",
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
      trailBacked: true,
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
        | {
            requestId?: unknown;
            status?: unknown;
            options?: Array<{ optionId?: string; description?: string }>;
            tool?: { locations?: Array<{ path?: string; line?: number | null }> };
          }
        | undefined;
      return approval && state.permissionPending === true ? approval : undefined;
    }, "ChatPanel should expose a pending permission request");

    assert.equal(approval.status, "pending");
    assert.ok(approval.requestId, "approval node should include the ACP request id");
    assert.equal(approval.options?.[0]?.description, "Allow README update");
    assert.equal(approval.tool?.locations?.[0]?.path, "README.md");
    assert.equal(approval.tool?.locations?.[0]?.line, 1);
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
    }, "Trail should record the approved permission write");
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

function assertSplitWebviewAssets(extension: vscode.Extension<unknown>): void {
  const webviewDist = path.join(extension.extensionUri.fsPath, "dist", "webview");
  const mainScript = path.join(webviewDist, "main.js");
  const mainStyle = path.join(webviewDist, "main.css");
  const chunksDir = path.join(webviewDist, "chunks");

  assert.ok(fs.existsSync(mainScript), "split webview module entry should exist in extension dist");
  assert.ok(fs.existsSync(mainStyle), "split webview stylesheet should exist in extension dist");
  assert.ok(fs.existsSync(chunksDir), "split webview chunks should exist in extension dist");
  assert.equal(fs.existsSync(path.join(extension.extensionUri.fsPath, "dist", "webview.js")), false);

  const mainBytes = fs.statSync(mainScript).size;
  assert.ok(mainBytes < 237_000, `webview module entry should stay below 237kb, got ${mainBytes} bytes`);

  const chunks = fs.readdirSync(chunksDir);
  assert.ok(chunks.some((file) => /^highlight-[A-Z0-9]+\.js$/.test(file)), "highlight chunk should be packaged with the extension");
  assert.match(fs.readFileSync(mainScript, "utf8"), /import\("\.\/chunks\/highlight-[A-Z0-9]+\.js"\)/);
}

function assertChatWebviewShell(chat: ChatPanel): void {
  const html = (chat as unknown as { panel: vscode.WebviewPanel }).panel.webview.html;
  const normalized = html.replace(/&#39;/g, "'");
  assert.match(html, /<script nonce="[^"]+" type="module" src="[^"]+\/dist\/webview\/main\.js[^"]*"><\/script>/);
  assert.match(html, /<link rel="stylesheet" href="[^"]+\/dist\/webview\/main\.css[^"]*">/);
  assert.match(normalized, /script-src 'nonce-[^']+' [^;]+/);
  assert.match(normalized, /style-src [^;]+ 'unsafe-inline'/);
  assert.doesNotMatch(html, /dist\/webview\.js/);
  assert.doesNotMatch(html, /dist\/webview\.css/);
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

function isNodeSource(node: unknown, source: string): boolean {
  return Boolean(node && typeof node === "object" && (node as { source?: unknown }).source === source);
}

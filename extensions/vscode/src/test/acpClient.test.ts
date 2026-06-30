import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import type * as vscode from "vscode";
import { AcpClient } from "../acp/AcpClient";
import type { AcpProviderProfile } from "../acp/ProviderRegistry";
import type { RequestPermissionParams, SessionUpdate } from "../shared/acpTypes";

test("AcpClient negotiates capabilities and handles provider updates", async () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-client-"));
  const scriptPath = path.join(tempDir, "stub-acp-agent.mjs");
  fs.writeFileSync(scriptPath, stubAgentSource(), "utf8");

  const output: vscode.OutputChannel = {
    name: "test",
    append(_value: string): void {},
    appendLine(_value: string): void {},
    replace(_value: string): void {},
    clear(): void {},
    show(): void {},
    hide(): void {},
    dispose(): void {}
  };
  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [scriptPath],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };

  const updates: SessionUpdate[] = [];
  const updateSessionIds: Array<string | undefined> = [];
  const permissions: RequestPermissionParams[] = [];
  const completions: unknown[] = [];
  const errors: Error[] = [];
  const client = new AcpClient(process.cwd(), provider, output);

  try {
    const session = await client.start({
      update: (update, sessionId) => {
        updates.push(update);
        updateSessionIds.push(sessionId);
      },
      permission: (requestId, params) => {
        permissions.push(params);
        client.approve(requestId, "allow");
      },
      completed: (response) => completions.push(response),
      error: (error) => errors.push(error),
      exit: () => {}
    });

    assert.equal(session.sessionId, "stub-session");
    assert.deepEqual(session.capabilities.promptCapabilities, {
      image: true,
      audio: false,
      embeddedContext: true
    });
    assert.equal(session.capabilities.sessionCapabilities.close, true);
    assert.equal(session.session.modes?.currentModeId, "ask");
    assert.equal(session.session.configOptions?.[0]?.id, "model");

    assert.deepEqual(await client.setMode("code"), { ok: true, modeId: "code" });
    assert.deepEqual(await client.setConfigOption("model", "large"), {
      configOptions: [
        {
          id: "model",
          name: "Model",
          type: "select",
          currentValue: "large",
          options: [
            {
              value: "large",
              name: "Large"
            }
          ]
        }
      ]
    });

    const response = await client.prompt([{ type: "text", text: "hello" }]);
    assert.deepEqual(response, { stopReason: "end_turn" });
    assert.equal(completions.length, 1);
    assert.equal(errors.length, 0);
    assert.equal(updates.some((update) => update.sessionUpdate === "agent_message_chunk"), true);
    assert.deepEqual(updateSessionIds, ["stub-session"]);
    assert.equal(permissions.length, 1);
    assert.equal(permissions[0]?.toolCall.kind, "execute");
  } finally {
    client.dispose();
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});

test("AcpClient normalizes snake-case session ids for updates and permissions", async () => {
  const { client, cleanup } = clientForLifecycleStub(snakeCaseSessionAliasStubSource());
  const updates: SessionUpdate[] = [];
  const updateSessionIds: Array<string | undefined> = [];
  const permissions: RequestPermissionParams[] = [];

  try {
    const session = await client.start({
      update: (update, sessionId) => {
        updates.push(update);
        updateSessionIds.push(sessionId);
      },
      permission: (requestId, params) => {
        permissions.push(params);
        client.approve(requestId, "allow");
      },
      completed(): void {},
      error(error): void {
        throw error;
      },
      exit(): void {}
    });

    assert.equal(session.sessionId, "snake-session");
    const response = await client.prompt([{ type: "text", text: "hello" }]);
    assert.deepEqual(response, { stopReason: "end_turn" });
    assert.equal(updates[0]?.sessionUpdate, "agent_message_chunk");
    assert.deepEqual(updateSessionIds, ["snake-session"]);
    assert.equal(permissions.length, 1);
    assert.equal(permissions[0]?.sessionId, "snake-session");
    assert.equal(permissions[0]?.toolCall.toolCallId, "snake-tool");
    assert.equal(permissions[0]?.options[0]?.optionId, "allow");
    assert.equal(permissions[0]?.options[0]?.name, "Allow");
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient cancels pending permission requests when cancelling a turn", async () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-client-"));
  const scriptPath = path.join(tempDir, "stub-acp-agent.mjs");
  fs.writeFileSync(scriptPath, cancelPermissionStubSource(), "utf8");
  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [scriptPath],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };
  const client = new AcpClient(process.cwd(), provider, testOutput());
  let cancelledRequests: string[] = [];

  try {
    await client.start({
      ...emptyListeners(),
      permission: (requestId) => {
        cancelledRequests = client.cancel();
        assert.equal(requestId, "permission-cancel");
      }
    });
    const response = await client.prompt([{ type: "text", text: "please cancel" }]);
    assert.deepEqual(cancelledRequests, ["permission-cancel"]);
    assert.deepEqual(response, {
      stopReason: "cancelled",
      cancelSeen: true,
      permissionCancelled: true
    });
  } finally {
    client.dispose();
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});

test("AcpClient preserves numeric permission request ids when approving", async () => {
  const { client, cleanup } = clientForLifecycleStub(numericPermissionStubSource());
  let seenRequestId: string | undefined;

  try {
    await client.start({
      ...emptyListeners(),
      permission: (requestId) => {
        seenRequestId = requestId;
        client.approve(requestId, "allow");
      }
    });
    const response = await client.prompt([{ type: "text", text: "numeric permission" }]);
    assert.equal(seenRequestId, "0");
    assert.deepEqual(response, {
      stopReason: "end_turn",
      permissionResponseIdType: "number",
      selected: true
    });
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient serves safe read-only workspace file requests", async () => {
  const workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-workspace-"));
  const providerRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-client-"));
  const outsideRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-outside-"));
  const workspaceFile = path.join(workspaceRoot, "notes.txt");
  const outsideFile = path.join(outsideRoot, "secret.txt");
  const scriptPath = path.join(providerRoot, "stub-acp-agent.mjs");
  fs.writeFileSync(workspaceFile, "zero\none\ntwo\nthree\n", "utf8");
  fs.writeFileSync(outsideFile, "secret\n", "utf8");
  fs.writeFileSync(scriptPath, fsReadStubSource({ readFile: workspaceFile, outsideFile }), "utf8");

  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [scriptPath],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };
  const client = new AcpClient(workspaceRoot, provider, testOutput());
  const completions: unknown[] = [];

  try {
    await client.start({
      ...emptyListeners(),
      completed: (response) => completions.push(response)
    });
    const response = await client.prompt([{ type: "text", text: "read notes" }]);
    assert.deepEqual(response, {
      stopReason: "end_turn",
      allowedContent: "one\ntwo",
      outsideDenied: true,
      writeDenied: true
    });
    assert.equal(completions.length, 1);
  } finally {
    client.dispose();
    fs.rmSync(workspaceRoot, { recursive: true, force: true });
    fs.rmSync(providerRoot, { recursive: true, force: true });
    fs.rmSync(outsideRoot, { recursive: true, force: true });
  }
});

test("AcpClient reads open editor buffers before disk content", async () => {
  const workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-workspace-"));
  const providerRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-client-"));
  const outsideRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-outside-"));
  const workspaceFile = path.join(workspaceRoot, "notes.txt");
  const outsideFile = path.join(outsideRoot, "secret.txt");
  const scriptPath = path.join(providerRoot, "stub-acp-agent.mjs");
  fs.writeFileSync(workspaceFile, "disk-zero\ndisk-one\ndisk-two\n", "utf8");
  fs.writeFileSync(outsideFile, "secret\n", "utf8");
  fs.writeFileSync(scriptPath, fsReadStubSource({ readFile: workspaceFile, outsideFile }), "utf8");

  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [scriptPath],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };
  const client = new AcpClient(workspaceRoot, provider, testOutput(), {
    readOpenTextDocument: (filePath) =>
      path.resolve(filePath) === path.resolve(workspaceFile)
        ? "buffer-zero\nbuffer-one\nbuffer-two\nbuffer-three\n"
        : undefined
  });

  try {
    await client.start(emptyListeners());
    const response = await client.prompt([{ type: "text", text: "read notes" }]);
    assert.deepEqual(response, {
      stopReason: "end_turn",
      allowedContent: "buffer-one\nbuffer-two",
      outsideDenied: true,
      writeDenied: true
    });
  } finally {
    client.dispose();
    fs.rmSync(workspaceRoot, { recursive: true, force: true });
    fs.rmSync(providerRoot, { recursive: true, force: true });
    fs.rmSync(outsideRoot, { recursive: true, force: true });
  }
});

test("AcpClient allows reads from advertised additional workspace roots", async () => {
  const workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-workspace-"));
  const additionalRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-additional-"));
  const providerRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-client-"));
  const outsideRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-outside-"));
  const additionalFile = path.join(additionalRoot, "notes.txt");
  const outsideFile = path.join(outsideRoot, "secret.txt");
  const scriptPath = path.join(providerRoot, "stub-acp-agent.mjs");
  fs.writeFileSync(additionalFile, "extra-zero\nextra-one\nextra-two\n", "utf8");
  fs.writeFileSync(outsideFile, "secret\n", "utf8");
  fs.writeFileSync(scriptPath, fsReadStubSource({ readFile: additionalFile, outsideFile, additionalDirectories: true }), "utf8");

  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [scriptPath],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };
  const client = new AcpClient(workspaceRoot, provider, testOutput(), {
    additionalWorkspaceRoots: [additionalRoot]
  });

  try {
    await client.start(emptyListeners());
    const response = await client.prompt([{ type: "text", text: "read notes" }]);
    assert.deepEqual(response, {
      stopReason: "end_turn",
      allowedContent: "extra-one\nextra-two",
      outsideDenied: true,
      writeDenied: true
    });
  } finally {
    client.dispose();
    fs.rmSync(workspaceRoot, { recursive: true, force: true });
    fs.rmSync(additionalRoot, { recursive: true, force: true });
    fs.rmSync(providerRoot, { recursive: true, force: true });
    fs.rmSync(outsideRoot, { recursive: true, force: true });
  }
});

test("AcpClient denies additional workspace reads when not advertised", async () => {
  const workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-workspace-"));
  const additionalRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-additional-"));
  const providerRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-client-"));
  const additionalFile = path.join(additionalRoot, "notes.txt");
  const scriptPath = path.join(providerRoot, "stub-acp-agent.mjs");
  fs.writeFileSync(additionalFile, "extra-zero\nextra-one\n", "utf8");
  fs.writeFileSync(scriptPath, fsReadDeniedStubSource(additionalFile), "utf8");

  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [scriptPath],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };
  const client = new AcpClient(workspaceRoot, provider, testOutput(), {
    additionalWorkspaceRoots: [additionalRoot]
  });

  try {
    await client.start(emptyListeners());
    const response = await client.prompt([{ type: "text", text: "read notes" }]);
    assert.deepEqual(response, {
      stopReason: "end_turn",
      denied: true
    });
  } finally {
    client.dispose();
    fs.rmSync(workspaceRoot, { recursive: true, force: true });
    fs.rmSync(additionalRoot, { recursive: true, force: true });
    fs.rmSync(providerRoot, { recursive: true, force: true });
  }
});

test("AcpClient loads an existing session only when loadSession is advertised", async () => {
  const { client, cleanup } = clientForLifecycleStub(
    lifecycleStubSource({
      capabilities: {
        loadSession: true
      },
      expectedMethod: "session/load",
      sessionId: "persisted-session"
    })
  );

  try {
    const session = await client.start(emptyListeners(), {
      existingSessionId: "persisted-session"
    });
    assert.equal(session.startMode, "load");
    assert.equal(session.sessionId, "persisted-session");
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient prefers session/resume when resume capability is advertised", async () => {
  const { client, cleanup } = clientForLifecycleStub(
    lifecycleStubSource({
      capabilities: {
        loadSession: true,
        sessionCapabilities: {
          resume: {}
        }
      },
      expectedMethod: "session/resume",
      sessionId: "persisted-session"
    })
  );

  try {
    const session = await client.start(emptyListeners(), {
      existingSessionId: "persisted-session"
    });
    assert.equal(session.startMode, "resume");
    assert.equal(session.sessionId, "persisted-session");
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient starts a new session when existing session loading is unsupported", async () => {
  const { client, cleanup } = clientForLifecycleStub(
    lifecycleStubSource({
      capabilities: {},
      expectedMethod: "session/new",
      sessionId: "new-session"
    })
  );

  try {
    const session = await client.start(emptyListeners(), {
      existingSessionId: "persisted-session"
    });
    assert.equal(session.startMode, "new");
    assert.equal(session.requestedSessionId, "persisted-session");
    assert.equal(session.sessionId, "new-session");
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient sends additional workspace roots when advertised", async () => {
  const workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-workspace-"));
  const additionalRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-additional-"));
  const providerRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-client-"));
  const scriptPath = path.join(providerRoot, "stub-acp-agent.mjs");
  fs.writeFileSync(
    scriptPath,
    lifecycleStubSource({
      capabilities: {
        sessionCapabilities: {
          additionalDirectories: {}
        }
      },
      expectedMethod: "session/new",
      sessionId: "multi-root-session",
      expectedAdditionalDirectories: [path.resolve(additionalRoot)]
    }),
    "utf8"
  );

  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [scriptPath],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };
  const client = new AcpClient(workspaceRoot, provider, testOutput(), {
    additionalWorkspaceRoots: [additionalRoot, workspaceRoot, additionalRoot]
  });

  try {
    const session = await client.start(emptyListeners());
    assert.equal(session.sessionId, "multi-root-session");
  } finally {
    client.dispose();
    fs.rmSync(workspaceRoot, { recursive: true, force: true });
    fs.rmSync(additionalRoot, { recursive: true, force: true });
    fs.rmSync(providerRoot, { recursive: true, force: true });
  }
});

test("AcpClient authenticates and retries session setup when required", async () => {
  const { client, cleanup } = clientForLifecycleStub(authRequiredStubSource());
  const seenMethods: string[][] = [];
  const authenticatedMethods: string[] = [];

  try {
    const session = await client.start({
      ...emptyListeners(),
      authenticate: (methods) => {
        seenMethods.push(methods.map((method) => method.id));
        return "agent-login";
      },
      authenticated: (method) => {
        authenticatedMethods.push(method.id);
      }
    });
    assert.equal(session.sessionId, "authenticated-session");
    assert.deepEqual(seenMethods, [["agent-login"]]);
    assert.deepEqual(authenticatedMethods, ["agent-login"]);
    assert.equal(session.authMethods[0]?.name, "Agent login");
    assert.equal(session.authenticatedMethod?.id, "agent-login");
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient authenticates and retries active session requests when required", async () => {
  const { client, cleanup } = clientForLifecycleStub(activeSessionAuthRequiredStubSource());
  const seenMethods: string[][] = [];
  const authenticatedMethods: string[] = [];
  const completions: unknown[] = [];

  try {
    const session = await client.start({
      ...emptyListeners(),
      authenticate: (methods) => {
        seenMethods.push(methods.map((method) => method.id));
        return "agent-login";
      },
      authenticated: (method) => {
        authenticatedMethods.push(method.id);
      },
      completed: (response) => completions.push(response)
    });
    assert.equal(session.sessionId, "active-auth-session");

    const response = await client.prompt([{ type: "text", text: "continue" }]);
    assert.deepEqual(response, { stopReason: "end_turn", authenticated: true });
    assert.deepEqual(completions, [response]);
    assert.deepEqual(seenMethods, [["agent-login"]]);
    assert.deepEqual(authenticatedMethods, ["agent-login"]);
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient leaves prompt requests unbounded so approvals can wait indefinitely", async () => {
  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false
  };
  const client = new AcpClient(process.cwd(), provider, testOutput());
  const calls: Array<{ method: string; timeoutMs: number | null | undefined }> = [];
  const internals = client as unknown as {
    sessionId?: string;
    rpc: {
      request<T>(method: string, params?: unknown, timeoutMs?: number | null): Promise<T>;
    };
  };
  internals.sessionId = "unbounded-session";
  internals.rpc.request = async <T>(
    method: string,
    _params?: unknown,
    timeoutMs?: number | null
  ): Promise<T> => {
    calls.push({ method, timeoutMs });
    if (method === "session/prompt") {
      return { stopReason: "end_turn" } as T;
    }
    throw new Error(`unexpected request ${method}`);
  };

  const response = await client.prompt("wait for approval");

  assert.deepEqual(response, { stopReason: "end_turn" });
  assert.deepEqual(calls, [{ method: "session/prompt", timeoutMs: null }]);
});

test("AcpClient forwards stderr and exit signal diagnostics", async () => {
  const { client, cleanup } = clientForLifecycleStub(exitSignalStubSource());
  const stderrLines: string[] = [];
  let exitStatus: { code: number | null; signal: NodeJS.Signals | null } | undefined;
  let resolveExit: (() => void) | undefined;
  const exited = new Promise<void>((resolve) => {
    resolveExit = resolve;
  });

  try {
    const session = await client.start({
      ...emptyListeners(),
      stderr: (line) => stderrLines.push(line),
      exit: (code, signal) => {
        exitStatus = { code, signal };
        resolveExit?.();
      }
    });
    assert.equal(session.sessionId, "exit-signal-session");

    await exited;
    assert.deepEqual(stderrLines, ["upstream exploded"]);
    assert.deepEqual(exitStatus, { code: null, signal: "SIGTERM" });
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient passes task name and checkpoint ref to CrabDB-backed providers", async () => {
  const { client, cleanup } = clientForLifecycleStub(
    lifecycleStubSource({
      capabilities: {},
      expectedMethod: "session/new",
      sessionId: "named-session",
      expectedArgv: ["--name", "Docs task", "--from", "ch_checkpoint"]
    }),
    {
      supportsTaskName: true,
      supportsFromRef: true
    }
  );

  try {
    const session = await client.start(emptyListeners(), {
      taskName: "Docs task",
      fromRef: "ch_checkpoint"
    });
    assert.equal(session.sessionId, "named-session");
  } finally {
    client.dispose();
    cleanup();
  }
});

test("AcpClient rejects unsupported protocol versions", async () => {
  const { client, cleanup } = clientForLifecycleStub(
    lifecycleStubSource({
      capabilities: {},
      expectedMethod: "session/new",
      sessionId: "unsupported-version-session",
      protocolVersion: 2
    })
  );

  try {
    await assert.rejects(
      () => client.start(emptyListeners()),
      /Unsupported ACP protocol version 2/
    );
  } finally {
    client.dispose();
    cleanup();
  }
});

function stubAgentSource(): string {
  return `
import readline from "node:readline";

let promptRequestId = null;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    if (message.params.protocolVersion !== 1) {
      send({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32602,
          message: "Expected numeric protocolVersion 1"
        }
      });
      return;
    }
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: "1",
        agentCapabilities: {
          promptCapabilities: {
            image: true,
            audio: false,
            embeddedContext: true
          },
          sessionCapabilities: {
            close: true
          }
        }
      }
    });
    return;
  }

  if (message.method === "session/new") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: "stub-session",
        modes: {
          currentModeId: "ask",
          availableModes: [
            {
              id: "ask",
              name: "Ask"
            },
            {
              id: "code",
              name: "Code"
            }
          ]
        },
        configOptions: [
          {
            id: "model",
            name: "Model",
            type: "select",
            currentValue: "small",
            options: [
              {
                value: "small",
                name: "Small"
              },
              {
                value: "large",
                name: "Large"
              }
            ]
          }
        ]
      }
    });
    return;
  }

  if (message.method === "session/set_mode") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        ok: true,
        modeId: message.params.modeId
      }
    });
    return;
  }

  if (message.method === "session/set_config_option") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        configOptions: [
          {
            id: message.params.configId,
            name: "Model",
            type: "select",
            currentValue: message.params.value,
            options: [
              {
                value: message.params.value,
                name: "Large"
              }
            ]
          }
        ]
      }
    });
    return;
  }

  if (message.method === "session/prompt") {
    promptRequestId = message.id;
    send({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: "stub-session",
        update: {
          sessionUpdate: "agent_message_chunk",
          messageId: "msg-1",
          content: {
            type: "text",
            text: "received"
          }
        }
      }
    });
    send({
      jsonrpc: "2.0",
      id: "permission-1",
      method: "session/request_permission",
      params: {
        sessionId: "stub-session",
        toolCall: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-1",
          title: "Run test command",
          kind: "execute",
          status: "pending"
        },
        options: [
          {
            optionId: "allow",
            name: "Allow once"
          }
        ]
      }
    });
    return;
  }

  if (message.id === "permission-1") {
    send({
      jsonrpc: "2.0",
      id: promptRequestId,
      result: {
        stopReason: "end_turn"
      }
    });
    setTimeout(() => process.exit(0), 10);
  }
});
`;
}

function snakeCaseSessionAliasStubSource(): string {
  return `
import readline from "node:readline";

let promptRequestId = null;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: "1",
        agentCapabilities: {}
      }
    });
    return;
  }

  if (message.method === "session/new") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        session_id: "snake-session"
      }
    });
    return;
  }

  if (message.method === "session/prompt") {
    promptRequestId = message.id;
    send({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        session_id: "snake-session",
        session_update: {
          session_update: "agent_message_chunk",
          message_id: "snake-message",
          content: {
            type: "text",
            text: "received"
          }
        }
      }
    });
    send({
      jsonrpc: "2.0",
      id: "snake-permission",
      method: "session/request_permission",
      params: {
        session_id: "snake-session",
        tool_call: {
          session_update: "tool_call",
          tool_call_id: "snake-tool",
          title: "Run snake command",
          kind: "execute",
          status: "pending"
        },
        options: [
          {
            option_id: "allow",
            label: "Allow"
          }
        ]
      }
    });
    return;
  }

  if (message.id === "snake-permission") {
    send({
      jsonrpc: "2.0",
      id: promptRequestId,
      result: {
        stopReason: "end_turn"
      }
    });
    setTimeout(() => process.exit(0), 10);
  }
});
`;
}

function cancelPermissionStubSource(): string {
  return `
import readline from "node:readline";

let promptRequestId = null;
let cancelSeen = false;
let permissionCancelled = false;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

function finishIfReady() {
  if (!promptRequestId || !cancelSeen || !permissionCancelled) {
    return;
  }
  send({
    jsonrpc: "2.0",
    id: promptRequestId,
    result: {
      stopReason: "cancelled",
      cancelSeen,
      permissionCancelled
    }
  });
  setTimeout(() => process.exit(0), 10);
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: 1,
        agentCapabilities: {}
      }
    });
    return;
  }

  if (message.method === "session/new") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: "cancel-session"
      }
    });
    return;
  }

  if (message.method === "session/prompt") {
    promptRequestId = message.id;
    send({
      jsonrpc: "2.0",
      id: "permission-cancel",
      method: "session/request_permission",
      params: {
        sessionId: "cancel-session",
        toolCall: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-cancel",
          title: "Run cancellable command",
          kind: "execute",
          status: "pending"
        },
        options: [
          {
            optionId: "allow",
            name: "Allow once"
          }
        ]
      }
    });
    return;
  }

  if (message.method === "session/cancel") {
    cancelSeen = true;
    finishIfReady();
    return;
  }

  if (message.id === "permission-cancel") {
    permissionCancelled = message.result?.outcome?.outcome === "cancelled";
    finishIfReady();
  }
});
`;
}

function numericPermissionStubSource(): string {
  return `
import readline from "node:readline";

let promptRequestId = null;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: "1",
        agentCapabilities: {}
      }
    });
    return;
  }

  if (message.method === "session/new") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: "numeric-permission-session"
      }
    });
    return;
  }

  if (message.method === "session/prompt") {
    promptRequestId = message.id;
    send({
      jsonrpc: "2.0",
      id: 0,
      method: "session/request_permission",
      params: {
        sessionId: "numeric-permission-session",
        toolCall: {
          sessionUpdate: "tool_call",
          toolCallId: "tool-numeric",
          title: "Run numeric id command",
          kind: "execute",
          status: "pending"
        },
        options: [
          {
            optionId: "allow",
            name: "Allow once"
          }
        ]
      }
    });
    return;
  }

  if (message.id === 0 || message.id === "0") {
    send({
      jsonrpc: "2.0",
      id: promptRequestId,
      result: {
        stopReason: "end_turn",
        permissionResponseIdType: typeof message.id,
        selected: message.result?.outcome?.outcome === "selected" && message.result?.outcome?.optionId === "allow"
      }
    });
    setTimeout(() => process.exit(0), 10);
  }
});
`;
}

function fsReadStubSource(options: { readFile: string; outsideFile: string; additionalDirectories?: boolean | undefined }): string {
  return `
import readline from "node:readline";

let promptRequestId = null;
let allowedContent = "";
let outsideDenied = false;
let writeDenied = false;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    const capabilities = message.params.clientCapabilities;
    if (capabilities.fs.readTextFile !== true || capabilities.fs.writeTextFile !== false || capabilities.terminal !== false) {
      send({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32602,
          message: "Unexpected client capabilities"
        }
      });
      return;
    }
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: "1",
        agentCapabilities: ${
          options.additionalDirectories
            ? JSON.stringify({ sessionCapabilities: { additionalDirectories: {} } })
            : "{}"
        }
      }
    });
    return;
  }

  if (message.method === "session/new") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: "fs-session"
      }
    });
    return;
  }

  if (message.method === "session/prompt") {
    promptRequestId = message.id;
    send({
      jsonrpc: "2.0",
      id: "read-allowed",
      method: "fs/read_text_file",
      params: {
        sessionId: "fs-session",
        path: ${JSON.stringify(options.readFile)},
        line: 2,
        limit: 2
      }
    });
    return;
  }

  if (message.id === "read-allowed") {
    allowedContent = message.result.content;
    send({
      jsonrpc: "2.0",
      id: "read-outside",
      method: "fs/read_text_file",
      params: {
        sessionId: "fs-session",
        path: ${JSON.stringify(options.outsideFile)}
      }
    });
    return;
  }

  if (message.id === "read-outside") {
    outsideDenied = Boolean(message.error && message.error.message.includes("outside the workspace"));
    send({
      jsonrpc: "2.0",
      id: "write-denied",
      method: "fs/write_text_file",
      params: {
        sessionId: "fs-session",
        path: ${JSON.stringify(options.readFile)},
        content: "change"
      }
    });
    return;
  }

  if (message.id === "write-denied") {
    writeDenied = Boolean(message.error && message.error.message.includes("does not expose direct filesystem mutation"));
    send({
      jsonrpc: "2.0",
      id: promptRequestId,
      result: {
        stopReason: "end_turn",
        allowedContent,
        outsideDenied,
        writeDenied
      }
    });
    setTimeout(() => process.exit(0), 10);
  }
});
`;
}

function fsReadDeniedStubSource(readFile: string): string {
  return `
import readline from "node:readline";

let promptRequestId = null;
let denied = false;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: "1",
        agentCapabilities: {}
      }
    });
    return;
  }

  if (message.method === "session/new") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: "fs-denied-session"
      }
    });
    return;
  }

  if (message.method === "session/prompt") {
    promptRequestId = message.id;
    send({
      jsonrpc: "2.0",
      id: "read-denied",
      method: "fs/read_text_file",
      params: {
        sessionId: "fs-denied-session",
        path: ${JSON.stringify(readFile)}
      }
    });
    return;
  }

  if (message.id === "read-denied") {
    denied = Boolean(message.error && message.error.message.includes("outside the workspace"));
    send({
      jsonrpc: "2.0",
      id: promptRequestId,
      result: {
        stopReason: "end_turn",
        denied
      }
    });
    setTimeout(() => process.exit(0), 10);
  }
});
`;
}

function clientForLifecycleStub(
  source: string,
  profileOverrides: Partial<AcpProviderProfile> = {}
): { client: AcpClient; cleanup(): void } {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-acp-client-"));
  const scriptPath = path.join(tempDir, "stub-acp-agent.mjs");
  fs.writeFileSync(scriptPath, source, "utf8");
  const provider: AcpProviderProfile = {
    id: "stub",
    label: "Stub ACP",
    command: process.execPath,
    args: [scriptPath],
    crabdbBacked: true,
    supportsTaskName: false,
    supportsFromRef: false,
    ...profileOverrides
  };
  return {
    client: new AcpClient(process.cwd(), provider, testOutput()),
    cleanup: () => fs.rmSync(tempDir, { recursive: true, force: true })
  };
}

function emptyListeners() {
  return {
    update(): void {},
    permission(): void {},
    completed(): void {},
    error(error: Error): void {
      throw error;
    },
    exit(): void {}
  };
}

function testOutput(): vscode.OutputChannel {
  return {
    name: "test",
    append(_value: string): void {},
    appendLine(_value: string): void {},
    replace(_value: string): void {},
    clear(): void {},
    show(): void {},
    hide(): void {},
    dispose(): void {}
  };
}

function lifecycleStubSource(options: {
  capabilities: Record<string, unknown>;
  expectedMethod: string;
  sessionId: string;
  expectedArgv?: string[] | undefined;
  expectedAdditionalDirectories?: string[] | undefined;
  protocolVersion?: string | number | undefined;
}): string {
  return `
import readline from "node:readline";

const expectedArgv = ${JSON.stringify(options.expectedArgv ?? [])};
const expectedAdditionalDirectories = ${JSON.stringify(options.expectedAdditionalDirectories)};
for (let index = 0; index < expectedArgv.length; index += 1) {
  if (process.argv[index + 2] !== expectedArgv[index]) {
    throw new Error("Expected argv " + JSON.stringify(expectedArgv) + " but received " + JSON.stringify(process.argv.slice(2)));
  }
}

const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: ${JSON.stringify(options.protocolVersion ?? 1)},
        agentCapabilities: ${JSON.stringify(options.capabilities)}
      }
    });
    return;
  }

  if (message.method === ${JSON.stringify(options.expectedMethod)}) {
    if (expectedAdditionalDirectories !== undefined && JSON.stringify(message.params.additionalDirectories) !== JSON.stringify(expectedAdditionalDirectories)) {
      send({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32602,
          message: "Expected additionalDirectories " + JSON.stringify(expectedAdditionalDirectories) + " but received " + JSON.stringify(message.params.additionalDirectories)
        }
      });
      return;
    }
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: ${JSON.stringify(options.sessionId)}
      }
    });
    setTimeout(() => process.exit(0), 10);
    return;
  }

  send({
    jsonrpc: "2.0",
    id: message.id,
    error: {
      code: -32601,
      message: "Unexpected method " + message.method
    }
  });
});
`;
}

function authRequiredStubSource(): string {
  return `
import readline from "node:readline";

let authenticated = false;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: "1",
        agentCapabilities: {},
        authMethods: [
          {
            id: "agent-login",
            name: "Agent login",
            description: "Sign in using the agent flow"
          }
        ]
      }
    });
    return;
  }

  if (message.method === "authenticate") {
    authenticated = message.params.methodId === "agent-login";
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {}
    });
    return;
  }

  if (message.method === "session/new") {
    if (!authenticated) {
      send({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32000,
          message: "auth_required",
          data: {
            code: "auth_required"
          }
        }
      });
      return;
    }
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: "authenticated-session"
      }
    });
    setTimeout(() => process.exit(0), 10);
  }
});
`;
}

function activeSessionAuthRequiredStubSource(): string {
  return `
import readline from "node:readline";

let authenticated = false;
const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: "1",
        agentCapabilities: {},
        authMethods: [
          {
            id: "agent-login",
            name: "Agent login",
            description: "Sign in using the agent flow"
          }
        ]
      }
    });
    return;
  }

  if (message.method === "session/new") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: "active-auth-session"
      }
    });
    return;
  }

  if (message.method === "authenticate") {
    authenticated = message.params.methodId === "agent-login";
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {}
    });
    return;
  }

  if (message.method === "session/prompt") {
    if (!authenticated) {
      send({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32000,
          message: "auth_required",
          data: {
            code: "auth_required"
          }
        }
      });
      return;
    }
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        stopReason: "end_turn",
        authenticated: true
      }
    });
    setTimeout(() => process.exit(0), 10);
  }
});
`;
}

function exitSignalStubSource(): string {
  return `
import readline from "node:readline";

const rl = readline.createInterface({ input: process.stdin });

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\\n");
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: "1",
        agentCapabilities: {}
      }
    });
    return;
  }

  if (message.method === "session/new") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        sessionId: "exit-signal-session"
      }
    });
    setTimeout(() => {
      process.stderr.write("upstream exploded\\n");
      process.kill(process.pid, "SIGTERM");
    }, 10);
  }
});
`;
}

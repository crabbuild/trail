import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { EventEmitter } from "node:events";
import * as readline from "node:readline";
import type * as vscode from "vscode";
import { redactCommandArgs, redactString } from "../shared/securityRedaction";

export interface JsonRpcMessage {
  jsonrpc: "2.0";
  id?: string | number | null;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: { code?: number; message?: string; data?: unknown };
}

export class JsonRpcRequestError extends Error {
  constructor(
    readonly code: number | undefined,
    message: string,
    readonly data?: unknown
  ) {
    super(message);
    this.name = "JsonRpcRequestError";
  }
}

export interface JsonRpcProcessEvents {
  notification: [message: JsonRpcMessage];
  request: [message: JsonRpcMessage];
  stderr: [line: string];
  exit: [code: number | null, signal: NodeJS.Signals | null];
}

const DEFAULT_REQUEST_TIMEOUT_MS = 120000;

export declare interface JsonRpcProcess {
  on<K extends keyof JsonRpcProcessEvents>(event: K, listener: (...args: JsonRpcProcessEvents[K]) => void): this;
  emit<K extends keyof JsonRpcProcessEvents>(event: K, ...args: JsonRpcProcessEvents[K]): boolean;
}

export class JsonRpcProcess extends EventEmitter {
  private child: ChildProcessWithoutNullStreams | undefined;
  private nextId = 1000;
  private readonly pending = new Map<
    string,
    {
      resolve(value: unknown): void;
      reject(error: Error): void;
      timeout: NodeJS.Timeout | undefined;
    }
  >();

  constructor(private readonly output: vscode.OutputChannel) {
    super();
  }

  start(command: string, args: string[], cwd: string): void {
    if (this.child) {
      throw new Error("ACP process is already running.");
    }

    this.output.appendLine(`Starting ACP process: ${redactString(command)} ${redactCommandArgs(args).join(" ")}`);
    const child = spawn(command, args, {
      cwd,
      env: process.env
    });
    this.child = child;

    const stdout = readline.createInterface({ input: child.stdout });
    const stderr = readline.createInterface({ input: child.stderr });

    stdout.on("line", (line) => this.handleLine(line));
    stderr.on("line", (line) => {
      this.output.appendLine(`[acp] ${redactString(line)}`);
      this.emit("stderr", line);
    });
    child.on("error", (error) => this.rejectAll(error));
    child.on("exit", (code, signal) => {
      this.emit("exit", code, signal);
      this.rejectAll(new Error(acpExitMessage(code, signal)));
      this.child = undefined;
    });
  }

  request<T>(method: string, params?: unknown, timeoutMs: number | null = DEFAULT_REQUEST_TIMEOUT_MS): Promise<T> {
    const id = this.nextId++;
    this.write({ jsonrpc: "2.0", id, method, params });
    return new Promise<T>((resolve, reject) => {
      const timeout =
        timeoutMs === null
          ? undefined
          : setTimeout(() => {
              this.pending.delete(String(id));
              reject(new Error(`ACP request timed out: ${method}`));
            }, timeoutMs);
      this.pending.set(String(id), {
        resolve: (value) => resolve(value as T),
        reject,
        timeout
      });
    });
  }

  notify(method: string, params?: unknown): void {
    this.write({ jsonrpc: "2.0", method, params });
  }

  respond(id: string | number | null | undefined, result: unknown): void {
    if (id === undefined) {
      return;
    }
    this.write({ jsonrpc: "2.0", id, result });
  }

  respondError(id: string | number | null | undefined, message: string, code = -32000): void {
    if (id === undefined) {
      return;
    }
    this.write({ jsonrpc: "2.0", id, error: { code, message } });
  }

  dispose(): void {
    this.rejectAll(new Error("ACP process disposed."));
    this.child?.kill();
    this.child = undefined;
  }

  private handleLine(line: string): void {
    if (!line.trim()) {
      return;
    }
    let message: JsonRpcMessage;
    try {
      message = JSON.parse(line) as JsonRpcMessage;
    } catch {
      this.output.appendLine(`[acp] invalid JSON: ${redactString(line)}`);
      return;
    }

    if (message.method && message.id !== undefined) {
      this.emit("request", message);
      return;
    }

    if (message.method) {
      this.emit("notification", message);
      return;
    }

    if (message.id !== undefined) {
      const key = String(message.id);
      const pending = this.pending.get(key);
      if (!pending) {
        return;
      }
      if (pending.timeout) {
        clearTimeout(pending.timeout);
      }
      this.pending.delete(key);
      if (message.error) {
        pending.reject(
          new JsonRpcRequestError(message.error.code, message.error.message || `ACP request ${key} failed`, message.error.data)
        );
      } else {
        pending.resolve(message.result);
      }
    }
  }

  private write(message: JsonRpcMessage): void {
    if (!this.child) {
      throw new Error("ACP process is not running.");
    }
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  private rejectAll(error: Error): void {
    for (const pending of this.pending.values()) {
      if (pending.timeout) {
        clearTimeout(pending.timeout);
      }
      pending.reject(error);
    }
    this.pending.clear();
  }
}

function acpExitMessage(code: number | null, signal: NodeJS.Signals | null): string {
  if (typeof code === "number") {
    return `ACP process exited with code ${code}`;
  }
  if (signal) {
    return `ACP process exited with signal ${signal}`;
  }
  return "ACP process exited without an exit code or signal";
}

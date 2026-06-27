import { spawn } from "node:child_process";
import type * as vscode from "vscode";
import { getExtensionConfig } from "../config";
import { redactCommandArgs, redactString } from "../shared/securityRedaction";

export interface CrabDbCliResult {
  stdout: string;
  stderr: string;
  code: number | null;
}

export class CrabDbCli {
  constructor(
    private readonly workspaceRoot: string,
    private readonly output: vscode.OutputChannel
  ) {}

  async run(args: string[], options: { json?: boolean; timeoutMs?: number } = {}): Promise<CrabDbCliResult> {
    const config = getExtensionConfig();
    const fullArgs = ["--workspace", this.workspaceRoot];
    if (options.json) {
      fullArgs.push("--json");
    }
    fullArgs.push(...args);

    this.output.appendLine(`$ ${redactString(config.crabdbPath)} ${redactCommandArgs(fullArgs).map((arg) => shellDisplay(String(arg))).join(" ")}`);

    return new Promise((resolve, reject) => {
      const child = spawn(config.crabdbPath, fullArgs, {
        cwd: this.workspaceRoot,
        env: process.env
      });

      let stdout = "";
      let stderr = "";
      let settled = false;
      const timeout = setTimeout(() => {
        if (!settled) {
          child.kill();
        settled = true;
          reject(new Error(`CrabDB command timed out: ${redactCommandArgs(args).join(" ")}`));
        }
      }, options.timeoutMs ?? 30000);

      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", (chunk: string) => {
        stdout += chunk;
      });
      child.stderr.on("data", (chunk: string) => {
        stderr += chunk;
      });
      child.on("error", (error) => {
        if (!settled) {
          clearTimeout(timeout);
          settled = true;
          reject(error);
        }
      });
      child.on("close", (code) => {
        if (settled) {
          return;
        }
        clearTimeout(timeout);
        settled = true;
        if (stderr.trim()) {
          this.output.appendLine(redactString(stderr.trimEnd()));
        }
        resolve({ stdout, stderr, code });
      });
    });
  }

  async runJson<T>(args: string[], options: { timeoutMs?: number } = {}): Promise<T> {
    const result = await this.run(args, { ...options, json: true });
    if (result.code !== 0) {
      throw new Error(result.stderr.trim() || `CrabDB command failed with code ${result.code}`);
    }
    try {
      return JSON.parse(result.stdout) as T;
    } catch (error) {
      throw new Error(`CrabDB returned invalid JSON for ${redactCommandArgs(args).join(" ")}: ${String(error)}`);
    }
  }

  spawnDetached(args: string[]): void {
    const config = getExtensionConfig();
    const child = spawn(config.crabdbPath, ["--workspace", this.workspaceRoot, ...args], {
      cwd: this.workspaceRoot,
      detached: true,
      stdio: "ignore",
      env: process.env
    });
    child.unref();
  }
}

function shellDisplay(value: string): string {
  if (/^[A-Za-z0-9_./:@=-]+$/.test(value)) {
    return value;
  }
  return `'${value.replace(/'/g, "'\\''")}'`;
}

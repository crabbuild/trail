import * as vscode from "vscode";
import { shellCommandForPlatform } from "../trail/ShellCommand";
import type { LaneGateRequest } from "../trail/TaskRepository";

export type LaneGateKind = "test" | "eval";

export async function promptLaneGateRequest(kind: LaneGateKind): Promise<LaneGateRequest | undefined> {
  const label = laneGateLabel(kind);
  const commandLine = await vscode.window.showInputBox({
    prompt: `Command to run as a Trail lane ${label}`,
    placeHolder: kind === "test" ? "npm test" : "npm run eval",
    validateInput: (value) => (value.trim() ? undefined : "Enter a command to run.")
  });
  if (commandLine === undefined) {
    return undefined;
  }
  const suite = await vscode.window.showInputBox({
    prompt: `Suite name for this ${label} gate`,
    placeHolder: kind === "test" ? "unit" : "quality",
    value: kind === "test" ? "manual" : "manual-eval"
  });
  if (suite === undefined) {
    return undefined;
  }
  return {
    command: shellCommandForPlatform(commandLine),
    suite: suite.trim() || undefined
  };
}

export function laneGateLabel(kind: LaneGateKind): string {
  return kind === "test" ? "test" : "eval";
}

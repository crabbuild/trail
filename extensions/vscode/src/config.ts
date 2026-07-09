import * as vscode from "vscode";

export interface CustomProviderConfig {
  id: string;
  label: string;
  command: string;
  args?: string[];
}

export interface ExtensionConfig {
  trailPath: string;
  defaultProvider: string;
  autoStartDaemon: boolean;
  customProviders: CustomProviderConfig[];
}

export function getExtensionConfig(): ExtensionConfig {
  const config = vscode.workspace.getConfiguration("trail");
  return {
    trailPath: config.get<string>("path", "trail"),
    defaultProvider: config.get<string>("defaultProvider", "claude-code"),
    autoStartDaemon: config.get<boolean>("autoStartDaemon", true),
    customProviders: config.get<CustomProviderConfig[]>("customProviders", [])
  };
}

export function getWorkspaceRoot(): string | undefined {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

export function requireWorkspaceRoot(): string {
  const root = getWorkspaceRoot();
  if (!root) {
    throw new Error("Open a folder to use Trail agents.");
  }
  return root;
}

import * as vscode from "vscode";
import { classifyResourceTarget } from "../shared/resourceTargets";

export class ResourceOpener {
  constructor(private readonly workspaceRoot: string) {}

  async openPath(targetPath: string, line?: number): Promise<void> {
    await this.openResource(targetPath, line);
  }

  async openResource(target: string, line?: number): Promise<void> {
    const classified = classifyResourceTarget(target, this.workspaceRoot);
    switch (classified.kind) {
      case "workspace-file":
        await this.openFile(classified.path, line);
        return;
      case "external-file":
        if (await this.confirm(`Open file outside the workspace?\n${classified.path}`)) {
          await this.openFile(classified.path, line);
        }
        return;
      case "external-uri":
        if (await this.confirm(`Open external resource?\n${classified.uri}`)) {
          await vscode.env.openExternal(vscode.Uri.parse(classified.uri));
        }
        return;
      case "unsupported-uri":
        vscode.window.showWarningMessage(`Unsupported resource URI scheme: ${classified.scheme}`);
        return;
      case "invalid":
        vscode.window.showWarningMessage(`Cannot open resource: ${classified.reason}`);
        return;
      default:
        return;
    }
  }

  private async openFile(filePath: string, line?: number): Promise<void> {
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(filePath));
    const editor = await vscode.window.showTextDocument(document, {
      preview: true
    });
    if (typeof line === "number" && Number.isFinite(line) && line > 0) {
      const position = new vscode.Position(Math.max(0, line - 1), 0);
      editor.selection = new vscode.Selection(position, position);
      editor.revealRange(new vscode.Range(position, position), vscode.TextEditorRevealType.InCenterIfOutsideViewport);
    }
  }

  private async confirm(message: string): Promise<boolean> {
    const selected = await vscode.window.showWarningMessage(
      message,
      {
        modal: true
      },
      "Open"
    );
    return selected === "Open";
  }
}

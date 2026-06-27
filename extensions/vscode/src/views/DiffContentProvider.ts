import * as crypto from "node:crypto";
import * as vscode from "vscode";

interface DiffDocument {
  path: string;
  side: "old" | "new";
  content: string;
}

export class DiffContentProvider implements vscode.TextDocumentContentProvider {
  static readonly scheme = "crabdb-diff";

  private readonly changed = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this.changed.event;
  private readonly documents = new Map<string, DiffDocument>();

  provideTextDocumentContent(uri: vscode.Uri): string {
    const document = this.documents.get(uri.toString());
    return document?.content ?? "";
  }

  async openDiff(path: string, oldText: string, newText: string): Promise<void> {
    const id = crypto.randomBytes(8).toString("hex");
    const oldUri = this.storeDocument(id, path, "old", oldText);
    const newUri = this.storeDocument(id, path, "new", newText);
    await vscode.commands.executeCommand(
      "vscode.diff",
      oldUri,
      newUri,
      `${path} (CrabDB agent diff)`
    );
  }

  private storeDocument(id: string, path: string, side: "old" | "new", content: string): vscode.Uri {
    const uri = vscode.Uri.from({
      scheme: DiffContentProvider.scheme,
      authority: side,
      path: `/${id}/${encodePath(path)}`
    });
    this.documents.set(uri.toString(), { path, side, content });
    this.changed.fire(uri);
    return uri;
  }
}

function encodePath(value: string): string {
  return value
    .split("/")
    .map((part) => encodeURIComponent(part))
    .join("/");
}

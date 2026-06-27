import * as vscode from "vscode";
import type { PromptAttachment } from "./PromptAttachment";

export function attachmentFromSelectionOrFile(): PromptAttachment | undefined {
  return attachmentFromSelection() ?? attachmentFromActiveFile();
}

export function attachmentFromSelection(): PromptAttachment | undefined {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.selection.isEmpty) {
    return undefined;
  }
  const document = editor.document;
  const selected = document.getText(editor.selection);
  if (!selected) {
    return undefined;
  }
  return {
    id: stableAttachmentId("selection", document.uri.toString(), selected),
    kind: "selection",
    label: `${document.fileName}:${editor.selection.start.line + 1}-${editor.selection.end.line + 1}`,
    uri: document.uri.toString(),
    mimeType: mimeTypeForDocument(document),
    text: truncateAttachmentText(selected)
  };
}

export function attachmentFromActiveFile(): PromptAttachment | undefined {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.uri.scheme !== "file") {
    return undefined;
  }
  const document = editor.document;
  return {
    id: stableAttachmentId("file", document.uri.toString(), document.getText().slice(0, 256)),
    kind: "file",
    label: document.fileName,
    uri: document.uri.toString(),
    mimeType: mimeTypeForDocument(document),
    text: truncateAttachmentText(document.getText())
  };
}

export function attachmentFromDiagnostics(): PromptAttachment | undefined {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.uri.scheme !== "file") {
    return undefined;
  }
  const diagnostics = vscode.languages.getDiagnostics(editor.document.uri);
  if (!diagnostics.length) {
    return undefined;
  }
  const document = editor.document;
  const text = diagnostics
    .map((diagnostic) => {
      const start = diagnostic.range.start.line + 1;
      const end = diagnostic.range.end.line + 1;
      const severity = vscode.DiagnosticSeverity[diagnostic.severity] || "Diagnostic";
      return `${severity} ${start}-${end}: ${diagnostic.message}`;
    })
    .join("\n");

  return {
    id: stableAttachmentId("diagnostics", document.uri.toString(), text),
    kind: "diagnostics",
    label: `Diagnostics for ${document.fileName}`,
    uri: document.uri.toString(),
    mimeType: "text/plain",
    text: truncateAttachmentText(text)
  };
}

function truncateAttachmentText(text: string): string {
  const limit = 256 * 1024;
  if (text.length <= limit) {
    return text;
  }
  return `${text.slice(0, limit)}\n\n[CrabDB VS Code truncated this attachment to ${limit} characters.]`;
}

function mimeTypeForDocument(document: vscode.TextDocument): string {
  if (document.languageId && document.languageId !== "plaintext") {
    return `text/x-${document.languageId}`;
  }
  return "text/plain";
}

function stableAttachmentId(...parts: string[]): string {
  let hash = 0;
  const input = parts.join("\0");
  for (let index = 0; index < input.length; index += 1) {
    hash = (hash * 31 + input.charCodeAt(index)) >>> 0;
  }
  return `att-${hash.toString(16)}`;
}

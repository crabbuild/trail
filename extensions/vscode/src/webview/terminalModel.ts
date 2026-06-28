import { redactString } from "../shared/securityRedaction";

export type TerminalTone = "ok" | "warning" | "risk" | "muted";

export interface TerminalPresentationInput {
  terminalId?: string | undefined;
  title?: string | undefined;
  command?: string | unknown[] | undefined;
  commandLine?: string | null | undefined;
  command_line?: string | null | undefined;
  cwd?: string | null | undefined;
  workingDirectory?: string | null | undefined;
  working_directory?: string | null | undefined;
  status?: string | null | undefined;
  state?: string | null | undefined;
  terminalStatus?: string | null | undefined;
  exitCode?: number | null | undefined;
  exit_code?: number | null | undefined;
  elapsedMs?: number | null | undefined;
  elapsed_ms?: number | null | undefined;
  durationMs?: number | null | undefined;
  output?: string | null | undefined;
  stdout?: string | null | undefined;
  stdoutPreview?: string | null | undefined;
  stdout_preview?: string | null | undefined;
  stderr?: string | null | undefined;
  stderrPreview?: string | null | undefined;
  stderr_preview?: string | null | undefined;
}

export interface TerminalOutputSection {
  id: "output" | "stdout" | "stderr";
  label: string;
  text: string;
  fullText: string;
  lineCount: number;
  charCount: number;
  truncated: boolean;
  tone: TerminalTone;
  openByDefault: boolean;
}

export interface TerminalMetric {
  label: string;
  value: string;
  tone: TerminalTone;
}

export interface TerminalPresentation {
  title: string;
  command?: string | undefined;
  cwd?: string | undefined;
  status?: string | undefined;
  statusLabel: string;
  tone: TerminalTone;
  exitCode?: number | undefined;
  elapsedMs?: number | undefined;
  metrics: TerminalMetric[];
  sections: TerminalOutputSection[];
  emptyText: string;
  openByDefault: boolean;
}

export function buildTerminalPresentation(input: TerminalPresentationInput, limit = 24_000): TerminalPresentation {
  const command = terminalCommand(input);
  const cwd = stringChoice(input, ["cwd", "workingDirectory", "working_directory"]);
  const status = stringChoice(input, ["terminalStatus", "status", "state"]);
  const exitCode = numberChoice(input, ["exitCode", "exit_code"]);
  const elapsedMs = numberChoice(input, ["elapsedMs", "elapsed_ms", "durationMs"]);
  const sections = terminalSections(input, limit);
  const tone = terminalTone(status, exitCode, sections);
  const statusLabel = terminalStatusLabel(status, exitCode);
  return {
    title: input.title || command || input.terminalId || "Terminal command",
    command,
    cwd,
    status,
    statusLabel,
    tone,
    exitCode,
    elapsedMs,
    metrics: terminalMetrics({ statusLabel, tone, exitCode, elapsedMs, sections }),
    sections,
    emptyText: "No terminal output preview is available.",
    openByDefault: tone === "risk" || tone === "warning" || sections.some((section) => section.id === "stderr" && section.text.trim())
  };
}

export function terminalCommand(input: TerminalPresentationInput | Record<string, unknown>): string | undefined {
  const command = input.command;
  if (typeof command === "string" && command) {
    return command;
  }
  if (Array.isArray(command)) {
    return command.map((part) => String(part)).join(" ");
  }
  const commandLine = stringChoice(input, [
    "commandLine",
    "command_line",
    "cmd",
    "shellCommand",
    "shell_command",
    "bashCommand",
    "bash_command",
    "script"
  ]);
  if (commandLine) {
    return commandLine;
  }
  const executable = stringChoice(input, ["executable", "program", "binary"]);
  const args = (input as Record<string, unknown>).args;
  if (executable && Array.isArray(args)) {
    return [executable, ...args.map((part) => String(part))].join(" ");
  }
  return undefined;
}

export function terminalStatusLabel(status: string | undefined, exitCode: number | undefined): string {
  if (typeof exitCode === "number") {
    return exitCode === 0 ? "passed" : `exit ${exitCode}`;
  }
  switch (normalize(status)) {
    case "completed":
    case "succeeded":
    case "success":
    case "passed":
      return "passed";
    case "failed":
    case "error":
      return "failed";
    case "cancelled":
    case "canceled":
      return "cancelled";
    case "running":
    case "in-progress":
      return "running";
    case "pending":
      return "pending";
    default:
      return status || "recorded";
  }
}

function terminalSections(input: TerminalPresentationInput, limit: number): TerminalOutputSection[] {
  return [
    terminalSection("output", "Output", stringChoice(input, ["output"]), "muted", limit),
    terminalSection("stdout", "Stdout", stringChoice(input, ["stdout", "stdoutPreview", "stdout_preview"]), "ok", limit),
    terminalSection("stderr", "Stderr", stringChoice(input, ["stderr", "stderrPreview", "stderr_preview"]), "risk", limit)
  ].filter((section): section is TerminalOutputSection => Boolean(section));
}

function terminalSection(
  id: TerminalOutputSection["id"],
  label: string,
  value: string | undefined,
  tone: TerminalTone,
  limit: number
): TerminalOutputSection | undefined {
  if (!value) {
    return undefined;
  }
  const fullText = redactString(value);
  const truncated = truncateText(fullText, limit);
  return {
    id,
    label,
    text: truncated.text,
    fullText,
    lineCount: fullText ? fullText.split("\n").length : 0,
    charCount: Array.from(fullText).length,
    truncated: truncated.truncated,
    tone,
    openByDefault: true
  };
}

function terminalTone(
  status: string | undefined,
  exitCode: number | undefined,
  sections: TerminalOutputSection[]
): TerminalTone {
  if (typeof exitCode === "number") {
    return exitCode === 0 ? "ok" : "risk";
  }
  const normalized = normalize(status);
  if (["failed", "error", "cancelled", "canceled"].includes(normalized)) {
    return "risk";
  }
  if (["running", "in-progress", "pending"].includes(normalized)) {
    return "warning";
  }
  if (sections.some((section) => section.id === "stderr" && section.fullText.trim().length > 0)) {
    return "warning";
  }
  if (["completed", "succeeded", "success", "passed"].includes(normalized)) {
    return "ok";
  }
  return "muted";
}

function terminalMetrics({
  statusLabel,
  tone,
  exitCode,
  elapsedMs,
  sections
}: {
  statusLabel: string;
  tone: TerminalTone;
  exitCode?: number | undefined;
  elapsedMs?: number | undefined;
  sections: TerminalOutputSection[];
}): TerminalMetric[] {
  const metrics: TerminalMetric[] = [{ label: "status", value: statusLabel, tone }];
  if (typeof exitCode === "number") {
    metrics.push({ label: "exit", value: String(exitCode), tone: exitCode === 0 ? "ok" : "risk" });
  }
  if (typeof elapsedMs === "number") {
    metrics.push({ label: "elapsed", value: formatDuration(elapsedMs), tone: "muted" });
  }
  for (const section of sections) {
    metrics.push({
      label: section.id,
      value: `${formatCount(section.lineCount)} line${section.lineCount === 1 ? "" : "s"}`,
      tone: section.tone
    });
  }
  return metrics.slice(0, 5);
}

function stringChoice(record: TerminalPresentationInput | Record<string, unknown>, keys: string[]): string | undefined {
  const values = record as Record<string, unknown>;
  for (const key of keys) {
    const value = values[key];
    if (typeof value === "string" && value) {
      return value;
    }
  }
  return undefined;
}

function numberChoice(record: TerminalPresentationInput | Record<string, unknown>, keys: string[]): number | undefined {
  const values = record as Record<string, unknown>;
  for (const key of keys) {
    const value = values[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
  }
  return undefined;
}

function truncateText(value: string, limit: number): { text: string; truncated: boolean } {
  if (value.length <= limit) {
    return { text: value, truncated: false };
  }
  return {
    text: `${value.slice(0, limit)}\n\n[truncated]`,
    truncated: true
  };
}

function normalize(value: string | undefined): string {
  return String(value || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function formatDuration(ms: number): string {
  if (ms < 1000) {
    return `${Math.round(ms)} ms`;
  }
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s`;
}

function formatCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

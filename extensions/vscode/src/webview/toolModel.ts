import type { ToolCallContent, ToolCallLocation, ToolCallStatus, ToolKind } from "../shared/acpTypes";
import { redactString } from "../shared/securityRedaction";

export type ToolPresentationKind = ToolKind | "background_process" | "task";
export type ToolTone = "default" | "file" | "change" | "query" | "terminal" | "risk" | "agent";
export type ToolRiskTone = "ok" | "warning" | "risk";
export type ToolActionKind = "openLocation" | "focusDiff";
export type ToolActionTone = "primary" | "default" | "danger";

export interface ToolFact {
  label: string;
  value: string;
}

export interface ToolStat {
  label: string;
  value: string;
  tone: "default" | "ok" | "warning" | "risk";
}

export interface ToolAction {
  kind: ToolActionKind;
  label: string;
  description: string;
  tone: ToolActionTone;
  path?: string | undefined;
  line?: number | undefined;
}

export interface ToolPresentationInput {
  title?: string | undefined;
  toolKind: ToolKind;
  toolStatus: ToolCallStatus;
  locations: ToolCallLocation[];
  content: ToolCallContent[];
  rawInput?: Record<string, unknown> | undefined;
  rawOutput?: Record<string, unknown> | undefined;
  source?: string | undefined;
}

export interface ToolPresentation {
  kind: ToolPresentationKind;
  title: string;
  operationLabel: string;
  summary: string;
  icon: string;
  tone: ToolTone;
  riskTone: ToolRiskTone;
  riskLabel: string;
  statusLabel: string;
  openByDefault: boolean;
  stats: ToolStat[];
  facts: ToolFact[];
  actions: ToolAction[];
  emptyText: string;
}

const keyList = (value: string): string[] => value.split(" ");
const TOOL_ARGUMENT_KEYS = keyList("arguments args input params parameters payload inputJson input_json");
const TOOL_SIGNAL_KEYS = keyList("tool name toolName tool_name function functionName function_name operation action kind");
const PATH_KEYS = keyList("path file filePath file_path filename target targetPath target_path relativePath relative_path");
const MOVE_FROM_KEYS = keyList("from oldPath old_path source sourcePath source_path");
const MOVE_TO_KEYS = keyList("to newPath new_path destination destinationPath destination_path");
const QUERY_KEYS = keyList("query pattern regex search needle");
const URL_KEYS = keyList("url uri href");
const CWD_KEYS = keyList("cwd workingDirectory working_directory");
const LINE_KEYS = keyList("line lineNumber line_number startLine start_line");
const COMMAND_KEYS = keyList("command commandLine command_line");
const ACTION_KEYS = keyList("action operation mode");
const DESCRIPTION_KEYS = keyList("description prompt task taskDescription task_description");
const SUBAGENT_KEYS = keyList("subagent_type subagentType agent agentType agent_type type");
const PROCESS_ID_KEYS = keyList("id processId processID process_id pid");
const PROCESS_STATUS_KEYS = keyList("status state");
const MAX_ARGUMENT_JSON_CHARS = 20_000;
const GENERIC_TOOL_SIGNALS = new Set(["tool", "toolcall", "toolcallupdate", "calltool", "functioncall", "sessionupdate"]);

export function buildToolPresentation(input: ToolPresentationInput): ToolPresentation {
  const content = contentCounts(input.content);
  const kind = effectiveToolKind(input, content);
  const visual = toolVisual(kind);
  const risk = toolRisk(kind, input.toolStatus, content);
  const statusLabel = toolStatusLabel(input.toolStatus);
  const facts = toolInputFacts(input.rawInput);
  return {
    kind,
    title: toolTitle(input.title, visual.operationLabel),
    operationLabel: visual.operationLabel,
    summary: toolSummary(input, kind, content, facts),
    icon: visual.icon,
    tone: risk.riskTone === "risk" ? "risk" : visual.tone,
    riskTone: risk.riskTone,
    riskLabel: risk.riskLabel,
    statusLabel,
    openByDefault: openByDefault(input.toolStatus, kind),
    stats: toolStats(input, kind, content),
    facts,
    actions: toolActions(input, kind, content, risk.riskTone),
    emptyText: toolEmptyText(input.source, kind)
  };
}

function toolTitle(title: string | undefined, fallback: string): string {
  const cleaned = String(title || "").trim();
  if (!cleaned || /^(tool|tool_call|tool_call_update|call_tool|session_update)$/i.test(cleaned)) {
    return fallback;
  }
  return cleaned;
}

export function toolStatusLabel(status: string): string {
  switch (status) {
    case "in_progress":
      return "running";
    case "completed":
      return "done";
    case "failed":
      return "failed";
    case "cancelled":
      return "cancelled";
    case "pending":
      return "pending";
    default:
      return status;
  }
}

function toolVisual(kind: ToolPresentationKind): Pick<ToolPresentation, "operationLabel" | "icon" | "tone"> {
  switch (kind) {
    case "background_process":
      return { operationLabel: "Process", icon: "process", tone: "terminal" };
    case "task":
      return { operationLabel: "Agent", icon: "task", tone: "agent" };
    case "read":
      return { operationLabel: "Read", icon: "file", tone: "file" };
    case "edit":
      return { operationLabel: "Edit", icon: "changed", tone: "change" };
    case "delete":
      return { operationLabel: "Delete", icon: "close", tone: "risk" };
    case "move":
      return { operationLabel: "Move", icon: "changed", tone: "change" };
    case "search":
      return { operationLabel: "Search", icon: "search", tone: "query" };
    case "execute":
      return { operationLabel: "Run", icon: "terminal", tone: "terminal" };
    case "think":
      return { operationLabel: "Think", icon: "review", tone: "default" };
    case "fetch":
      return { operationLabel: "Fetch", icon: "open", tone: "query" };
    case "switch_mode":
      return { operationLabel: "Mode", icon: "settings", tone: "default" };
    default:
      return { operationLabel: "Tool", icon: "settings", tone: "default" };
  }
}

function toolSummary(
  input: ToolPresentationInput,
  kind: ToolPresentationKind,
  content: ReturnType<typeof contentCounts>,
  facts: ToolFact[]
): string {
  const location = firstLocation(input.locations);
  const path = location?.path || factValue(facts, "Path");
  const move = factValue(facts, "Move");
  const query = factValue(facts, "Query");
  const resource = factValue(facts, "Resource");
  const command = factValue(facts, "Command") || redactedCommand(input);
  const action = factValue(facts, "Action");
  const description = factValue(facts, "Description");
  const agent = factValue(facts, "Agent");
  const process = factValue(facts, "Process");
  const processStatus = factValue(facts, "Process status");
  const pathSuffix = input.locations.length > 1 ? ` +${formatCount(input.locations.length - 1)}` : "";

  if (kind === "background_process") {
    if (command) {
      return command;
    }
    const processParts = [action, processStatus, process].filter(Boolean);
    return processParts.length ? processParts.join(" · ") : "Background process";
  }
  if (kind === "task") {
    if (description && agent) {
      return `${agent}: ${description}`;
    }
    return description || agent || (input.toolStatus === "completed" ? "Agent task" : "Delegating to agent");
  }
  if (kind === "execute" && command) {
    return command;
  }
  if (kind === "think") {
    return input.toolStatus === "completed" ? "Thought" : "Thinking";
  }
  if (content.diffBlocks > 0 && path) {
    return `${countSummary(content.diffBlocks, "diff")} in ${shortLabel(path)}${pathSuffix}`;
  }
  if (content.diffBlocks > 0) {
    return countSummary(content.diffBlocks, "diff");
  }
  if (move) {
    return move;
  }
  if (path) {
    return `${shortLabel(path)}${pathSuffix}`;
  }
  if (query) {
    return query;
  }
  if (resource) {
    return resource;
  }
  if (command) {
    return command;
  }
  if (content.terminalBlocks > 0) {
    return countSummary(content.terminalBlocks, "terminal preview");
  }
  if (content.contentBlocks > 0) {
    return countSummary(content.contentBlocks, "output");
  }
  if (kind === "edit") {
    if (input.toolStatus === "failed") {
      return "Edit needs inspection";
    }
    if (input.toolStatus === "completed") {
      return "Workspace change";
    }
    return "Preparing workspace change";
  }
  return toolVisual(kind).operationLabel;
}

function toolEmptyText(source: string | undefined, kind: ToolPresentationKind): string {
  if (kind === "edit") {
    return "No diff preview available for this edit.";
  }
  return source === "crabdb"
    ? "CrabDB persisted this tool event without rendered output."
    : "No rendered output for tool call.";
}

function effectiveToolKind(input: ToolPresentationInput, content: ReturnType<typeof contentCounts>): ToolPresentationKind {
  if (input.toolKind !== "other") {
    return input.toolKind;
  }
  const signal = normalizedToolSignal(input);
  const raw = toolArgumentRecord(input.rawInput);
  if (toolSignalMatches(signal, BACKGROUND_PROCESS_TOOL_SIGNALS)) {
    return "background_process";
  }
  if (
    toolSignalMatches(signal, TASK_TOOL_SIGNALS) ||
    stringChoice(raw, SUBAGENT_KEYS) ||
    (toolSignalMatches(signal, AGENT_TOOL_SIGNALS) && stringChoice(raw, DESCRIPTION_KEYS))
  ) {
    return "task";
  }
  if (terminalCommand(raw) || content.terminalBlocks || toolSignalMatches(signal, EXECUTE_TOOL_SIGNALS)) {
    return "execute";
  }
  if (toolSignalMatches(signal, DELETE_TOOL_SIGNALS)) {
    return "delete";
  }
  if (toolSignalMatches(signal, MOVE_TOOL_SIGNALS)) {
    return "move";
  }
  if (content.diffBlocks || stringChoice(raw, ["diff", "patch", "oldText", "newText"]) || toolSignalMatches(signal, EDIT_TOOL_SIGNALS)) {
    return "edit";
  }
  if (stringChoice(raw, URL_KEYS) || toolSignalMatches(signal, FETCH_TOOL_SIGNALS)) {
    return "fetch";
  }
  if (stringChoice(raw, QUERY_KEYS) || toolSignalMatches(signal, SEARCH_TOOL_SIGNALS)) {
    return "search";
  }
  if (
    stringChoice(raw, PATH_KEYS) ||
    input.locations.length ||
    toolSignalMatches(signal, READ_TOOL_SIGNALS)
  ) {
    return "read";
  }
  return "other";
}

const EXECUTE_TOOL_SIGNALS = ["bash", "shell", "terminal", "command", "execute", "exec", "run"];
const BACKGROUND_PROCESS_TOOL_SIGNALS = ["backgroundprocess"];
const TASK_TOOL_SIGNALS = ["task", "subagent"];
const AGENT_TOOL_SIGNALS = ["agent", "delegate"];
const DELETE_TOOL_SIGNALS = ["delete", "remove", "unlink", "rm"];
const MOVE_TOOL_SIGNALS = ["move", "rename", "mv"];
const EDIT_TOOL_SIGNALS = ["edit", "write", "create", "patch", "applypatch", "replace", "insert", "updatefile", "strreplace", "multiedit"];
const FETCH_TOOL_SIGNALS = ["fetch", "webfetch", "download", "http", "url"];
const SEARCH_TOOL_SIGNALS = ["search", "grep", "ripgrep", "rg", "glob", "find", "match"];
const READ_TOOL_SIGNALS = ["read", "readfile", "view", "open", "cat", "list", "listdir", "ls", "tree"];

function normalizedToolSignal(input: ToolPresentationInput): string {
  const raw = input.rawInput || {};
  const nested = nestedToolArgumentRecord(raw, 0) || {};
  const rawSignal = stringChoice(raw, TOOL_SIGNAL_KEYS);
  const argsSignal = stringChoice(nested, TOOL_SIGNAL_KEYS);
  const signal = rawSignal && !GENERIC_TOOL_SIGNALS.has(normalizeToolSignal(rawSignal)) ? rawSignal : argsSignal || rawSignal || input.title || "";
  return normalizeToolSignal(signal);
}

function normalizeToolSignal(signal: string): string {
  return signal.toLowerCase().replace(/[^a-z0-9]+/g, "");
}

function toolSignalMatches(signal: string, choices: string[]): boolean {
  return Boolean(signal) && choices.some((choice) => signal.includes(choice));
}

function toolRisk(kind: ToolPresentationKind, status: ToolCallStatus, content: ReturnType<typeof contentCounts>): { riskTone: ToolRiskTone; riskLabel: string } {
  if (status === "failed" || status === "cancelled") {
    return { riskTone: "risk", riskLabel: "Needs inspection" };
  }
  if (kind === "delete") {
    return { riskTone: "risk", riskLabel: "Destructive" };
  }
  if (kind === "execute") {
    return { riskTone: content.terminalBlocks ? "warning" : "risk", riskLabel: "Command" };
  }
  if (kind === "background_process") {
    return { riskTone: "warning", riskLabel: "Background process" };
  }
  if (kind === "task") {
    return { riskTone: "warning", riskLabel: "Delegated agent" };
  }
  if (kind === "edit" || kind === "move") {
    return { riskTone: "warning", riskLabel: "Workspace change" };
  }
  return { riskTone: "ok", riskLabel: "Read-only" };
}

function openByDefault(status: ToolCallStatus, kind: ToolPresentationKind): boolean {
  return status === "in_progress" || status === "pending" || (kind === "background_process" && status === "completed");
}

function toolStats(input: ToolPresentationInput, kind: ToolPresentationKind, content: ReturnType<typeof contentCounts>): ToolStat[] {
  if (kind === "think") {
    return [];
  }
  const stats: ToolStat[] = [];
  const locationCount = input.locations.length || (inputLocation(input.rawInput) ? 1 : 0);
  if (locationCount) {
    stats.push({
      label: `location${locationCount === 1 ? "" : "s"}`,
      value: formatCount(locationCount),
      tone: "default"
    });
  }
  if (content.diffBlocks) {
    stats.push({
      label: `diff${content.diffBlocks === 1 ? "" : "s"}`,
      value: formatCount(content.diffBlocks),
      tone: "warning"
    });
  }
  if (content.terminalBlocks) {
    stats.push({
      label: `terminal${content.terminalBlocks === 1 ? "" : "s"}`,
      value: formatCount(content.terminalBlocks),
      tone: kind === "execute" ? "warning" : "default"
    });
  }
  if (content.contentBlocks && kind !== "read") {
    stats.push({
      label: `output${content.contentBlocks === 1 ? "" : "s"}`,
      value: formatCount(content.contentBlocks),
      tone: "default"
    });
  }
  if (content.otherBlocks) {
    stats.push({
      label: `other block${content.otherBlocks === 1 ? "" : "s"}`,
      value: formatCount(content.otherBlocks),
      tone: "default"
    });
  }
  return stats.slice(0, 4);
}

function toolInputFacts(rawInput: Record<string, unknown> | undefined): ToolFact[] {
  const input = toolArgumentRecord(rawInput);
  if (!Object.keys(input).length) {
    return [];
  }
  const command = terminalCommand(input);
  const facts: ToolFact[] = [];
  const path = stringChoice(input, PATH_KEYS);
  const from = stringChoice(input, MOVE_FROM_KEYS);
  const to = stringChoice(input, MOVE_TO_KEYS);
  const query = stringChoice(input, QUERY_KEYS);
  const url = stringChoice(input, URL_KEYS);
  const cwd = stringChoice(input, CWD_KEYS);
  const action = stringChoice(input, ACTION_KEYS);
  const description = stringChoice(input, DESCRIPTION_KEYS);
  const agent = stringChoice(input, SUBAGENT_KEYS);
  const process = stringChoice(input, PROCESS_ID_KEYS);
  const processStatus = stringChoice(input, PROCESS_STATUS_KEYS);
  const line = numberChoice(input, LINE_KEYS);
  if (action) {
    facts.push({ label: "Action", value: action });
  }
  if (agent) {
    facts.push({ label: "Agent", value: agent });
  }
  if (description) {
    facts.push({ label: "Description", value: description });
  }
  if (path) {
    facts.push({ label: "Path", value: path });
  }
  if (from || to) {
    facts.push({ label: "Move", value: [from, to].filter(Boolean).join(" -> ") });
  }
  if (query) {
    facts.push({ label: "Query", value: query });
  }
  if (url) {
    facts.push({ label: "Resource", value: url });
  }
  if (command) {
    facts.push({ label: "Command", value: command });
  }
  if (cwd) {
    facts.push({ label: "Cwd", value: cwd });
  }
  if (process) {
    facts.push({ label: "Process", value: process });
  }
  if (processStatus) {
    facts.push({ label: "Process status", value: processStatus });
  }
  if (typeof line === "number") {
    facts.push({ label: "Line", value: String(line) });
  }
  return facts
    .filter((fact) => fact.value)
    .slice(0, 6)
    .map((fact) => ({ label: fact.label, value: redactString(shortLabel(fact.value)) }));
}

function toolActions(
  input: ToolPresentationInput,
  kind: ToolPresentationKind,
  content: ReturnType<typeof contentCounts>,
  riskTone: ToolRiskTone
): ToolAction[] {
  if (kind === "think") {
    return [];
  }
  const actions: ToolAction[] = [];
  const location = firstLocation(input.locations) || inputLocation(input.rawInput);

  if (content.diffBlocks) {
    actions.push({
      kind: "focusDiff",
      label: "Review diff",
      description: "Show this diff preview.",
      tone: riskTone === "risk" ? "default" : "primary"
    });
  }

  if (location) {
    actions.push({
      kind: "openLocation",
      label: input.locations.length > 1 ? "Open first path" : "Open path",
      description: `${location.line ? `Open line ${location.line} in ` : "Open "}${location.path}.`,
      tone: "default",
      path: location.path,
      line: location.line
    });
  }

  return dedupeActions(actions).slice(0, 4);
}

function inputLocation(rawInput: Record<string, unknown> | undefined): { path: string; line?: number | undefined } | undefined {
  const input = toolArgumentRecord(rawInput);
  const path = stringChoice(input, PATH_KEYS);
  if (!path) {
    return undefined;
  }
  return {
    path,
    line: numberChoice(input, LINE_KEYS)
  };
}

function firstLocation(locations: ToolCallLocation[]): { path: string; line?: number | undefined } | undefined {
  for (const location of locations) {
    const path = typeof location.path === "string" ? location.path.trim() : "";
    if (!path) {
      continue;
    }
    return {
      path,
      line: typeof location.line === "number" && Number.isFinite(location.line) ? location.line : undefined
    };
  }
  return undefined;
}

function redactedCommand(input: ToolPresentationInput): string | undefined {
  const rawCommand = terminalCommand(toolArgumentRecord(input.rawInput));
  if (rawCommand) {
    return redactString(shortLabel(rawCommand));
  }
  for (const block of input.content) {
    const command = terminalCommand(asRecord(block));
    if (command) {
      return redactString(shortLabel(command));
    }
  }
  return undefined;
}

function dedupeActions(actions: ToolAction[]): ToolAction[] {
  const seen = new Set<ToolActionKind>();
  return actions.filter((action) => {
    if (seen.has(action.kind)) {
      return false;
    }
    seen.add(action.kind);
    return true;
  });
}

function contentCounts(content: ToolCallContent[]): {
  diffBlocks: number;
  terminalBlocks: number;
  contentBlocks: number;
  otherBlocks: number;
} {
  let diffBlocks = 0;
  let terminalBlocks = 0;
  let contentBlocks = 0;
  let otherBlocks = 0;
  for (const block of content) {
    switch (asRecord(block).type) {
      case "diff":
        diffBlocks += 1;
        break;
      case "terminal":
        terminalBlocks += 1;
        break;
      case "content":
        contentBlocks += 1;
        break;
      default:
        otherBlocks += 1;
    }
  }
  return { diffBlocks, terminalBlocks, contentBlocks, otherBlocks };
}

function terminalCommand(record: Record<string, unknown>): string | undefined {
  const command = record.command;
  if (Array.isArray(command)) {
    return command.map((part) => String(part)).join(" ");
  }
  return stringChoice(record, COMMAND_KEYS);
}

function stringChoice(record: Record<string, unknown>, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value) {
      return value;
    }
  }
  return undefined;
}

function numberChoice(record: Record<string, unknown>, keys: string[]): number | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
    if (typeof value === "string" && /^\d+$/.test(value.trim())) {
      return Number(value.trim());
    }
  }
  return undefined;
}

export function toolArgumentRecord(rawInput: Record<string, unknown> | undefined): Record<string, unknown> {
  const raw = rawInput || {};
  const nested = nestedToolArgumentRecord(raw, 0);
  return nested ? { ...nested, ...raw } : raw;
}

function nestedToolArgumentRecord(record: Record<string, unknown>, depth: number): Record<string, unknown> | undefined {
  if (depth >= 3) {
    return undefined;
  }
  for (const key of TOOL_ARGUMENT_KEYS) {
    const nested = argumentValueRecord(record[key]);
    if (nested && Object.keys(nested).length) {
      const deeper = nestedToolArgumentRecord(nested, depth + 1);
      return deeper ? { ...deeper, ...nested } : nested;
    }
  }
  return undefined;
}

function argumentValueRecord(value: unknown): Record<string, unknown> | undefined {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  if (typeof value !== "string") {
    return undefined;
  }
  const trimmed = value.trim();
  if (!trimmed.startsWith("{") || trimmed.length > MAX_ARGUMENT_JSON_CHARS) {
    return undefined;
  }
  try {
    const parsed = JSON.parse(trimmed);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as Record<string, unknown>) : undefined;
  } catch {
    return undefined;
  }
}

function factValue(facts: ToolFact[], label: string): string | undefined {
  return facts.find((fact) => fact.label === label)?.value;
}

function countSummary(count: number, label: string): string {
  if (count <= 0) {
    return label;
  }
  return `${formatCount(count)} ${label}${count === 1 ? "" : "s"}`;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function shortLabel(value: string): string {
  if (value.length <= 96) {
    return value;
  }
  return `${value.slice(0, 45)}...${value.slice(-45)}`;
}

function formatCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

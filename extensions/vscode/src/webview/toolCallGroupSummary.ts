import type { ToolCallCardProps } from "./ToolCallCard";

interface ActivityCounts {
  commands: number;
  deletedFiles: number;
  editedFiles: number;
  listedFiles: number;
  movedFiles: number;
  otherTools: number;
  readFiles: number;
  searchedCode: number;
  searchedWeb: number;
}

export interface ToolCallGroupSummary {
  detail: string;
  title: string;
}

export function summarizeToolCallGroup(items: ToolCallCardProps[]): ToolCallGroupSummary {
  const title = activitySummary(items) || countLabel(items.length, "tool call");
  return {
    title,
    detail: operationDetail(items)
  };
}

function activitySummary(items: ToolCallCardProps[]): string {
  const counts: ActivityCounts = {
    commands: 0,
    deletedFiles: 0,
    editedFiles: 0,
    listedFiles: 0,
    movedFiles: 0,
    otherTools: 0,
    readFiles: 0,
    searchedCode: 0,
    searchedWeb: 0
  };

  for (const item of items) {
    countActivity(item, counts);
  }

  const changePhrases = [
    fileAction("edited", counts.editedFiles),
    fileAction("deleted", counts.deletedFiles),
    fileAction("moved", counts.movedFiles)
  ].filter(Boolean);
  const inspectionPhrases = [
    fileAction("read", counts.readFiles),
    counts.listedFiles ? "listed files" : "",
    counts.searchedCode ? "searched code" : ""
  ].filter(Boolean);
  const remainingPhrases = [
    counts.searchedWeb ? "searched the web" : "",
    countAction("ran", counts.commands, "command"),
    countAction("used", counts.otherTools, "tool")
  ].filter(Boolean);
  const summary = [
    ...changePhrases,
    joinWithAnd(inspectionPhrases),
    ...remainingPhrases
  ].filter(Boolean).join(", ");

  return sentenceCase(summary);
}

function countActivity(item: ToolCallCardProps, counts: ActivityCounts): void {
  const text = toolText(item);
  if (item.model.kind === "execute") {
    countCommandActivity(item, counts);
    return;
  }
  if (item.model.kind === "edit") {
    counts.editedFiles += fileCount(item);
    return;
  }
  if (item.model.kind === "delete") {
    counts.deletedFiles += fileCount(item);
    return;
  }
  if (item.model.kind === "move") {
    counts.movedFiles += fileCount(item);
    return;
  }
  if (item.model.kind === "read") {
    if (looksLikeFileListing(text)) {
      counts.listedFiles += 1;
    } else {
      counts.readFiles += fileCount(item);
    }
    return;
  }
  if (item.model.kind === "search") {
    if (looksLikeWebLookup(text)) {
      counts.searchedWeb += 1;
    } else {
      counts.searchedCode += 1;
    }
    return;
  }
  if (item.model.kind === "fetch") {
    counts.searchedWeb += 1;
    return;
  }
  counts.otherTools += 1;
}

function countCommandActivity(item: ToolCallCardProps, counts: ActivityCounts): void {
  const text = toolText(item);
  if (looksLikeFileListing(text)) {
    counts.listedFiles += 1;
    return;
  }
  if (looksLikeWebLookup(text)) {
    counts.searchedWeb += 1;
    return;
  }
  if (looksLikeCodeSearch(text)) {
    counts.searchedCode += 1;
    return;
  }
  counts.commands += 1;
}

function operationDetail(items: ToolCallCardProps[]): string {
  const operationCounts = new Map<string, number>();
  for (const item of items) {
    operationCounts.set(item.model.operationLabel, (operationCounts.get(item.model.operationLabel) || 0) + 1);
  }
  const operations = Array.from(operationCounts.entries())
    .map(([label, count]) => `${label} ${count}`)
    .join(" / ");
  return operations ? `${countLabel(items.length, "tool call")}: ${operations}` : countLabel(items.length, "tool call");
}

function fileCount(item: ToolCallCardProps): number {
  return Math.max(item.locations.length, statCount(item, "location"), 1);
}

function statCount(item: ToolCallCardProps, label: string): number {
  const stat = item.stats.find((candidate) => candidate.label === label || candidate.label === `${label}s`);
  const value = Number(String(stat?.value || "").replace(/,/g, ""));
  return Number.isFinite(value) && value > 0 ? value : 0;
}

function toolText(item: ToolCallCardProps): string {
  return `${item.rawToolKind} ${item.title} ${item.subtitle}`.toLowerCase();
}

function looksLikeFileListing(text: string): boolean {
  return /\b(list|listdir|readdir|read_dir|tree)\b/.test(text) ||
    /(^|[\s;&|()])(?:ls|find|fd)\b/.test(text) ||
    /(^|[\s;&|()])rg\s+--files\b/.test(text) ||
    /\bgit\s+ls-files\b/.test(text);
}

function looksLikeCodeSearch(text: string): boolean {
  return /\b(search|ripgrep)\b/.test(text) ||
    /(^|[\s;&|()])(?:rg|grep|ag|ack)\b/.test(text) ||
    /\bgit\s+grep\b/.test(text);
}

function looksLikeWebLookup(text: string): boolean {
  return /\b(web|browser|url|fetch|download|curl|wget)\b/.test(text) ||
    /https?:\/\//.test(text);
}

function fileAction(verb: string, count: number): string {
  return countAction(verb, count, "file");
}

function countAction(verb: string, count: number, noun: string): string {
  if (!count) {
    return "";
  }
  return count === 1 ? `${verb} a ${noun}` : `${verb} ${count} ${noun}s`;
}

function joinWithAnd(phrases: string[]): string {
  if (phrases.length <= 1) {
    return phrases[0] || "";
  }
  return `${phrases.slice(0, -1).join(", ")} and ${phrases[phrases.length - 1]}`;
}

function sentenceCase(value: string): string {
  return value ? `${value[0]?.toUpperCase()}${value.slice(1)}` : "";
}

function countLabel(count: number, label: string): string {
  if (!count) {
    return "";
  }
  return `${count} ${label}${count === 1 ? "" : "s"}`;
}

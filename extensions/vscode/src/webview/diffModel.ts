import { createTwoFilesPatch, diffLines, diffWordsWithSpace } from "diff";

const MAX_DIFF_SOURCE_CHARS = 160_000;
const MAX_DIFF_COPY_CHARS = 260_000;
const MAX_WORD_DIFF_LINE_CHARS = 320;
const DIFF_CONTEXT_LINES = 3;

export type DiffRowKind = "context" | "added" | "removed" | "modified" | "gap";

export interface DiffSegment {
  text: string;
  tone?: "added" | "removed" | undefined;
}

export interface DiffRow {
  kind: DiffRowKind;
  oldLine?: number | undefined;
  newLine?: number | undefined;
  oldText?: string | undefined;
  newText?: string | undefined;
  oldSegments?: DiffSegment[] | undefined;
  newSegments?: DiffSegment[] | undefined;
  omitted?: number | undefined;
}

export interface DiffModel {
  rows: DiffRow[];
  additions: number;
  deletions: number;
  kind: string;
  oldLineCount: number;
  newLineCount: number;
  rawDiff: string;
  omittedRows: number;
  tooLarge: boolean;
}

export function buildDiffModel(path: string, oldText: string, newText: string): DiffModel {
  const oldLineCount = diffLineCount(oldText);
  const newLineCount = diffLineCount(newText);
  const kind = !oldText && newText ? "created" : oldText && !newText ? "deleted" : "modified";
  if (oldText.length + newText.length > MAX_DIFF_SOURCE_CHARS) {
    const rawDiff = truncateText(oversizedUnifiedDiffText(path, oldText, newText), MAX_DIFF_COPY_CHARS).text;
    return {
      rows: oversizedDiffRows(oldText, newText),
      additions: newLineCount,
      deletions: oldLineCount,
      kind,
      oldLineCount,
      newLineCount,
      rawDiff,
      omittedRows: Math.max(0, oldLineCount + newLineCount - 80),
      tooLarge: true
    };
  }

  const fullRows = buildDiffRows(oldText, newText);
  const rawDiff = truncateText(unifiedDiffText(path, oldText, newText), MAX_DIFF_COPY_CHARS).text;
  const additions = fullRows.filter((row) => row.kind === "added" || row.kind === "modified").length;
  const deletions = fullRows.filter((row) => row.kind === "removed" || row.kind === "modified").length;
  const rows = compactDiffRows(fullRows);
  return {
    rows,
    additions,
    deletions,
    kind,
    oldLineCount,
    newLineCount,
    rawDiff,
    omittedRows: fullRows.length - rows.filter((row) => row.kind !== "gap").length,
    tooLarge: false
  };
}

export function roughDiffStats(oldText: string, newText: string): { oldLineCount: number; newLineCount: number; kind: string } {
  return {
    oldLineCount: diffLineCount(oldText),
    newLineCount: diffLineCount(newText),
    kind: !oldText && newText ? "created" : oldText && !newText ? "deleted" : "modified"
  };
}

function buildDiffRows(oldText: string, newText: string): DiffRow[] {
  const rows: DiffRow[] = [];
  const parts = diffLines(oldText, newText);
  let oldLine = 1;
  let newLine = 1;
  for (let index = 0; index < parts.length; index += 1) {
    const part = parts[index];
    if (!part) {
      continue;
    }
    const next = parts[index + 1];
    if (part.removed && next?.added) {
      const removedLines = splitDiffLines(part.value);
      const addedLines = splitDiffLines(next.value);
      const count = Math.max(removedLines.length, addedLines.length);
      for (let offset = 0; offset < count; offset += 1) {
        const oldValue = removedLines[offset];
        const newValue = addedLines[offset];
        rows.push(buildChangedRow(oldValue, newValue, oldLine, newLine));
        if (oldValue !== undefined) {
          oldLine += 1;
        }
        if (newValue !== undefined) {
          newLine += 1;
        }
      }
      index += 1;
      continue;
    }
    const lines = splitDiffLines(part.value);
    for (const line of lines) {
      if (part.added) {
        rows.push({ kind: "added", newLine: newLine++, newText: line });
      } else if (part.removed) {
        rows.push({ kind: "removed", oldLine: oldLine++, oldText: line });
      } else {
        rows.push({ kind: "context", oldLine: oldLine++, newLine: newLine++, oldText: line, newText: line });
      }
    }
  }
  return rows;
}

function buildChangedRow(oldValue: string | undefined, newValue: string | undefined, oldLine: number, newLine: number): DiffRow {
  const row: DiffRow = {
    kind: oldValue !== undefined && newValue !== undefined ? "modified" : oldValue !== undefined ? "removed" : "added",
    oldLine: oldValue !== undefined ? oldLine : undefined,
    newLine: newValue !== undefined ? newLine : undefined,
    oldText: oldValue,
    newText: newValue
  };
  if (
    row.kind === "modified" &&
    oldValue !== undefined &&
    newValue !== undefined &&
    oldValue.length <= MAX_WORD_DIFF_LINE_CHARS &&
    newValue.length <= MAX_WORD_DIFF_LINE_CHARS
  ) {
    row.oldSegments = wordSegments(oldValue, newValue, "old");
    row.newSegments = wordSegments(oldValue, newValue, "new");
  }
  return row;
}

function compactDiffRows(rows: DiffRow[]): DiffRow[] {
  const changed = rows
    .map((row, index) => (row.kind === "context" ? -1 : index))
    .filter((index) => index >= 0);
  if (!changed.length) {
    return rows.slice(0, 80);
  }
  const keep = new Set<number>();
  for (const index of changed) {
    for (let cursor = Math.max(0, index - DIFF_CONTEXT_LINES); cursor <= Math.min(rows.length - 1, index + DIFF_CONTEXT_LINES); cursor += 1) {
      keep.add(cursor);
    }
  }
  const compact: DiffRow[] = [];
  let omitted = 0;
  for (let index = 0; index < rows.length; index += 1) {
    if (keep.has(index)) {
      if (omitted > 0) {
        compact.push({ kind: "gap", omitted });
        omitted = 0;
      }
      compact.push(rows[index] as DiffRow);
    } else {
      omitted += 1;
    }
  }
  if (omitted > 0) {
    compact.push({ kind: "gap", omitted });
  }
  return compact;
}

function oversizedDiffRows(oldText: string, newText: string): DiffRow[] {
  const oldLines = splitDiffLines(oldText).slice(0, 40);
  const newLines = splitDiffLines(newText).slice(0, 40);
  const count = Math.max(oldLines.length, newLines.length);
  const rows: DiffRow[] = [];
  for (let index = 0; index < count; index += 1) {
    rows.push({
      kind: oldLines[index] !== undefined && newLines[index] !== undefined ? "modified" : oldLines[index] !== undefined ? "removed" : "added",
      oldLine: oldLines[index] !== undefined ? index + 1 : undefined,
      newLine: newLines[index] !== undefined ? index + 1 : undefined,
      oldText: oldLines[index],
      newText: newLines[index]
    });
  }
  return rows;
}

function wordSegments(oldText: string, newText: string, side: "old" | "new"): DiffSegment[] {
  return diffWordsWithSpace(oldText, newText)
    .map((part): DiffSegment | undefined => {
      if (side === "old") {
        if (part.added) {
          return undefined;
        }
        return { text: part.value, tone: part.removed ? "removed" : undefined };
      }
      if (part.removed) {
        return undefined;
      }
      return { text: part.value, tone: part.added ? "added" : undefined };
    })
    .filter((part): part is DiffSegment => Boolean(part));
}

function unifiedDiffText(path: string, oldText: string, newText: string): string {
  return createTwoFilesPatch(`${path} (before)`, `${path} (after)`, oldText, newText, "", "", {
    context: DIFF_CONTEXT_LINES
  }).trim();
}

function oversizedUnifiedDiffText(path: string, oldText: string, newText: string): string {
  const oldLines = splitDiffLines(oldText).slice(0, 80).map((line) => `-${line}`);
  const newLines = splitDiffLines(newText).slice(0, 80).map((line) => `+${line}`);
  return [
    `--- ${path} (before)`,
    `+++ ${path} (after)`,
    "@@ large diff preview truncated @@",
    ...oldLines,
    ...newLines
  ].join("\n");
}

function diffLineCount(text: string): number {
  return splitDiffLines(text).length;
}

function splitDiffLines(text: string): string[] {
  if (!text) {
    return [];
  }
  const lines = text.split("\n");
  if (lines.at(-1) === "") {
    lines.pop();
  }
  return lines;
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

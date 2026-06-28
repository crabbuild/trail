type DiffReviewStatus = "added" | "deleted" | "modified" | "renamed" | "untracked";

interface DiffReviewFile {
  id: string;
  path: string;
  oldText: string;
  newText: string;
  patch?: string | undefined;
  status: DiffReviewStatus;
  additions: number;
  deletions: number;
}

interface DiffReviewSuggestion {
  command?: string | undefined;
  reason: string;
}

interface DiffReviewModel {
  files: DiffReviewFile[];
  suggestions: DiffReviewSuggestion[];
  summary: string;
  rawPatch?: string | undefined;
  raw: unknown;
  additions: number;
  deletions: number;
}

type DiffReviewInlineActionTone = "default" | "primary" | "provider" | "lane" | "review" | "danger";

interface DiffReviewInlineAction {
  action: string;
  label: string;
  ariaLabel?: string | undefined;
  className?: string | undefined;
  data?: Record<string, string> | undefined;
  disabled?: boolean | undefined;
  icon: string;
  iconOnly?: boolean | undefined;
  title?: string | undefined;
  tone?: DiffReviewInlineActionTone | undefined;
}

export interface DiffReviewDrawerHelpers {
  escapeHtml(value: string): string;
  escapeClass(value: string): string;
  shortLabel(value: string): string;
  inlineActions(input: {
    actions: DiffReviewInlineAction[];
    ariaLabel: string;
    className?: string | undefined;
  }): string;
  rawDetails(value: unknown): string;
  diffPreview(input: {
    path: string;
    oldText: string;
    newText: string;
    patch?: string | undefined;
    additions?: number | undefined;
    deletions?: number | undefined;
    title?: string | undefined;
  }): string;
}

export interface DiffReviewDrawerRender {
  html: string;
  firstPath: string;
}

let currentHost: DiffReviewDrawerHelpers;

export function renderDiffReviewDrawer(result: unknown, renderHelpers: DiffReviewDrawerHelpers): DiffReviewDrawerRender | undefined {
  currentHost = renderHelpers;
  const review = buildDiffReviewModel(result);
  if (!review.files.length) {
    return undefined;
  }
  return {
    html: diffReviewDrawerContent(review),
    firstPath: review.files[0]?.path || ""
  };
}

function diffReviewDrawerContent(review: DiffReviewModel, host?: DiffReviewDrawerHelpers): string {
  host = host || currentHost;
  const fileLabel = `${review.files.length} file${review.files.length === 1 ? "" : "s"}`;
  return `
    <div class="drawer-header diff-review-header">
      <div>
        <h2>Review changes</h2>
        <p>${host.escapeHtml(review.summary)}</p>
      </div>
      ${host.inlineActions({
        ariaLabel: "Diff review drawer actions",
        className: "diff-review-header-actions",
        actions: [
          {
            action: "closeDrawer",
            ariaLabel: "Close diff review",
            icon: "close",
            iconOnly: true,
            label: "Close diff review",
            tone: "provider"
          }
        ]
      })}
    </div>
    <div class="diff-review-stats" aria-label="Diff summary">
      <span><b>${host.escapeHtml(fileLabel)}</b><small>changed</small></span>
      <span><b>+${review.additions}</b><small>additions</small></span>
      <span><b>-${review.deletions}</b><small>deletions</small></span>
      ${review.rawPatch ? `<span><b>patch</b><small>available</small></span>` : ""}
    </div>
    <div class="diff-review-layout">
      <aside class="diff-review-tree-panel" aria-label="Changed files">
        <div class="diff-review-panel-heading">
          <span>Files</span>
          <small>${host.escapeHtml(fileLabel)}</small>
        </div>
        ${diffReviewStatusLegend(review.files, host)}
        <div class="diff-review-file-tree" data-diff-review-tree aria-label="Changed file tree"></div>
        <div class="diff-review-file-fallback" aria-label="Changed file fallback list">
          ${review.files.map((file) => diffReviewFileButton(file, host)).join("")}
        </div>
        ${jsonTemplate("diff-review-tree-data", diffReviewTreeData(review), host)}
      </aside>
      <section class="diff-review-main" aria-label="Selected file diff">
        ${review.files.map((file) => diffReviewFileSection(file, host)).join("")}
      </section>
      <aside class="diff-review-side" aria-label="Suggested next actions">
        <div class="diff-review-panel-heading">
          <span>Next actions</span>
          <small>${review.suggestions.length ? `${review.suggestions.length} suggested` : "optional"}</small>
        </div>
        ${diffReviewSuggestionList(review.suggestions, host)}
      </aside>
    </div>
    ${host.rawDetails(review.raw)}
  `;
}

function diffReviewFileButton(file: DiffReviewFile, host: DiffReviewDrawerHelpers): string {
  return `
    <button class="diff-review-file-button diff-review-file-${host.escapeClass(file.status)}" data-action="selectDiffReviewFile" data-path="${host.escapeHtml(file.path)}" data-diff-review-path="${host.escapeHtml(file.path)}" aria-pressed="false">
      <span class="diff-review-file-main">
        <span class="diff-review-file-label">${host.escapeHtml(host.shortLabel(file.path))}</span>
        ${diffReviewStatusChip(file.status, host)}
      </span>
      <small>+${file.additions} -${file.deletions}</small>
    </button>
  `;
}

function diffReviewStatusLegend(files: DiffReviewFile[], host: DiffReviewDrawerHelpers): string {
  const counts = new Map<DiffReviewStatus, number>();
  for (const file of files) {
    counts.set(file.status, (counts.get(file.status) || 0) + 1);
  }
  const chips = (["added", "modified", "renamed", "deleted", "untracked"] as DiffReviewStatus[])
    .filter((status) => counts.has(status))
    .map((status) => diffReviewStatusChip(status, host, counts.get(status)));
  return chips.length ? `<div class="diff-review-status-legend" aria-label="Change types">${chips.join("")}</div>` : "";
}

function diffReviewStatusChip(status: DiffReviewStatus, host: DiffReviewDrawerHelpers, count?: number | undefined): string {
  const label = diffReviewStatusLabel(status);
  const suffix = typeof count === "number" ? ` ${count}` : "";
  return `<span class="diff-review-status-chip diff-review-status-${host.escapeClass(status)}" title="${host.escapeHtml(label)}"><span>${host.escapeHtml(label)}</span>${suffix ? `<b>${host.escapeHtml(suffix.trim())}</b>` : ""}</span>`;
}

function diffReviewStatusLabel(status: DiffReviewStatus): string {
  switch (status) {
    case "added":
      return "Added";
    case "deleted":
      return "Deleted";
    case "renamed":
      return "Renamed";
    case "untracked":
      return "Untracked";
    default:
      return "Modified";
  }
}

function diffReviewFileSection(file: DiffReviewFile, host: DiffReviewDrawerHelpers): string {
  return `
    <section class="diff-review-file" data-diff-review-file="${host.escapeHtml(file.path)}" hidden>
      ${host.diffPreview({
        path: file.path,
        oldText: file.oldText,
        newText: file.newText,
        patch: file.patch,
        additions: file.additions,
        deletions: file.deletions,
        title: "Task file diff"
      })}
    </section>
  `;
}

function diffReviewSuggestionList(suggestions: DiffReviewSuggestion[], host: DiffReviewDrawerHelpers): string {
  if (!suggestions.length) {
    return `<p class="muted">No follow-up commands were returned with this diff.</p>`;
  }
  return `
    <div class="diff-review-suggestions">
      ${suggestions
        .slice(0, 6)
        .map(
          (suggestion) => `
            <article class="diff-review-suggestion">
              ${suggestion.command ? `<code>${host.escapeHtml(suggestion.command)}</code>` : ""}
              <p>${host.escapeHtml(suggestion.reason)}</p>
              ${
                suggestion.command
                  ? host.inlineActions({
                      ariaLabel: "Diff review suggestion actions",
                      className: "diff-review-suggestion-actions",
                      actions: [
                        {
                          action: "insertDiffSuggestion",
                          ariaLabel: "Insert command in composer",
                          data: { command: suggestion.command },
                          icon: "message",
                          iconOnly: true,
                          label: "Insert command in composer",
                          tone: "review"
                        }
                      ]
                    })
                  : ""
              }
            </article>
          `
        )
        .join("")}
    </div>
  `;
}

function buildDiffReviewModel(result: unknown): DiffReviewModel {
  const root = asRecord(result);
  const rawPatch = stringChoice(root, ["diff", "patch", "unified_diff", "unifiedDiff"]);
  const patchFiles = rawPatch ? splitPatchFiles(rawPatch) : [];
  const explicitFiles = explicitDiffFiles(result, patchFiles);
  const files = dedupeDiffReviewFiles(explicitFiles.length ? explicitFiles : patchFiles);
  const additions = files.reduce((total, file) => total + file.additions, 0);
  const deletions = files.reduce((total, file) => total + file.deletions, 0);
  return {
    files,
    suggestions: diffReviewSuggestionsModel(root),
    summary:
      stringChoice(root, ["summary", "message", "title"]) ||
      `${files.length} changed file${files.length === 1 ? "" : "s"} ready for review.`,
    rawPatch,
    raw: result,
    additions,
    deletions
  };
}

function explicitDiffFiles(result: unknown, patchFiles: DiffReviewFile[]): DiffReviewFile[] {
  const root = asRecord(result);
  const candidates = Array.isArray(result)
    ? result
    : ["files", "diffs", "changes", "changed_files", "changedFiles", "patches"].flatMap((key) => arrayField(root, key));
  return candidates
    .map((item, index) => explicitDiffFile(item, index, patchFiles))
    .filter((file): file is DiffReviewFile => Boolean(file));
}

function explicitDiffFile(value: unknown, index: number, patchFiles: DiffReviewFile[]): DiffReviewFile | undefined {
  const record = asRecord(value);
  const patch = stringChoice(record, ["diff", "patch", "unified_diff", "unifiedDiff"]);
  const fallbackPatchFile = patch ? splitPatchFiles(patch)[0] : undefined;
  const path =
    stringChoice(record, ["path", "name", "file", "filename", "newPath", "new_path"]) ||
    fallbackPatchFile?.path ||
    (typeof value === "string" ? value : "");
  const oldText = stringChoice(record, ["oldText", "old_text", "before", "old", "original"]) || "";
  const newText = stringChoice(record, ["newText", "new_text", "after", "new", "updated"]) || "";
  if (!path || (!patch && !oldText && !newText)) {
    return undefined;
  }
  const patchMatch = patchFiles.find((file) => file.path === path);
  const stats = patch ? patchLineStats(patch) : patchMatch ? { additions: patchMatch.additions, deletions: patchMatch.deletions } : textLineDelta(oldText, newText);
  const status = diffReviewStatus(String(record.status || record.kind || record.type || patchMatch?.status || ""));
  return {
    id: `diff-review-file-${index}`,
    path,
    oldText,
    newText,
    patch: patch || patchMatch?.patch,
    status,
    additions: stats.additions,
    deletions: stats.deletions
  };
}

function splitPatchFiles(patch: string): DiffReviewFile[] {
  const normalized = patch.replace(/\r\n/g, "\n");
  const matches = Array.from(normalized.matchAll(/^diff --git\s+a\/(.+?)\s+b\/(.+)$/gm));
  if (!matches.length) {
    const path = pathFromPatchSection(normalized, "Patch");
    if (!normalized.includes("@@") && !path) {
      return [];
    }
    const stats = patchLineStats(normalized);
    return [
      {
        id: "patch-file-0",
        path: path || "Patch",
        oldText: "",
        newText: "",
        patch: normalized,
        status: patchStatus(normalized),
        additions: stats.additions,
        deletions: stats.deletions
      }
    ];
  }

  return matches.map((match, index) => {
    const start = match.index ?? 0;
    const end = matches[index + 1]?.index ?? normalized.length;
    const section = normalized.slice(start, end).trimEnd();
    const path = pathFromPatchSection(section, normalizePatchPath(match[2] || match[1] || `file-${index + 1}`));
    const stats = patchLineStats(section);
    return {
      id: `patch-file-${index}`,
      path,
      oldText: "",
      newText: "",
      patch: section,
      status: patchStatus(section),
      additions: stats.additions,
      deletions: stats.deletions
    };
  });
}

function pathFromPatchSection(section: string, fallback: string): string {
  const newPath = section.match(/^\+\+\+\s+(.+)$/m)?.[1];
  const oldPath = section.match(/^---\s+(.+)$/m)?.[1];
  const path = normalizePatchPath(newPath && newPath !== "/dev/null" ? newPath : oldPath || fallback);
  return path || fallback;
}

function normalizePatchPath(value: string): string {
  return value.replace(/^"|"$/g, "").replace(/^[ab]\//, "").trim();
}

function patchLineStats(patch: string): { additions: number; deletions: number } {
  let additions = 0;
  let deletions = 0;
  for (const line of patch.split(/\r?\n/)) {
    if (line.startsWith("+++") || line.startsWith("---")) {
      continue;
    }
    if (line.startsWith("+")) {
      additions += 1;
    } else if (line.startsWith("-")) {
      deletions += 1;
    }
  }
  return { additions, deletions };
}

function textLineDelta(oldText: string, newText: string): { additions: number; deletions: number } {
  return { additions: lineCount(newText), deletions: lineCount(oldText) };
}

function patchStatus(patch: string): DiffReviewStatus {
  if (/^new file mode/m.test(patch)) {
    return "added";
  }
  if (/^deleted file mode/m.test(patch)) {
    return "deleted";
  }
  if (/^rename (from|to)/m.test(patch)) {
    return "renamed";
  }
  return "modified";
}

function diffReviewStatus(value: string): DiffReviewStatus {
  const normalized = value.toLowerCase();
  if (normalized.includes("add") || normalized === "a" || normalized === "new") {
    return "added";
  }
  if (normalized.includes("delete") || normalized === "d" || normalized === "removed") {
    return "deleted";
  }
  if (normalized.includes("rename") || normalized === "r") {
    return "renamed";
  }
  if (normalized.includes("untracked") || normalized === "?") {
    return "untracked";
  }
  return "modified";
}

function dedupeDiffReviewFiles(files: DiffReviewFile[]): DiffReviewFile[] {
  const seen = new Set<string>();
  const unique: DiffReviewFile[] = [];
  for (const file of files) {
    if (seen.has(file.path)) {
      continue;
    }
    seen.add(file.path);
    unique.push(file);
  }
  return unique;
}

function diffReviewSuggestionsModel(root: Record<string, unknown>): DiffReviewSuggestion[] {
  return arrayField(root, "suggestions")
    .map((item) => {
      const record = asRecord(item);
      if (typeof item === "string") {
        return { reason: item };
      }
      const command = stringChoice(record, ["command", "cmd", "cli"]);
      const reason = stringChoice(record, ["reason", "summary", "description", "label"]) || command || compactValueLabel(item, "suggestion");
      return { command, reason };
    })
    .filter((suggestion) => suggestion.reason);
}

function diffReviewTreeData(review: DiffReviewModel): Array<{ path: string; status: string; additions: number; deletions: number }> {
  return review.files.map((file) => ({
    path: file.path,
    status: file.status,
    additions: file.additions,
    deletions: file.deletions
  }));
}

function jsonTemplate(className: string, value: unknown, host: DiffReviewDrawerHelpers): string {
  return `<template class="${host.escapeClass(className)}">${host.escapeHtml(JSON.stringify(value))}</template>`;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function arrayField(record: Record<string, unknown>, key: string): unknown[] {
  const value = record[key];
  return Array.isArray(value) ? value : [];
}

function stringChoice(record: Record<string, unknown>, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) {
      return value;
    }
  }
  return undefined;
}

function compactValueLabel(value: unknown, fallback: string): string {
  if (typeof value === "string") {
    return value;
  }
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const label = record.label || record.title || record.name || record.id || record.kind || record.type;
    if (label) {
      return String(label);
    }
  }
  if (value === undefined || value === null || value === "") {
    return fallback;
  }
  return String(value);
}

function lineCount(value: string): number {
  return value ? value.split("\n").length : 0;
}

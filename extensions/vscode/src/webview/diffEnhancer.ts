import { FileDiff, parsePatchFiles, type FileContents, type FileDiffMetadata } from "@pierre/diffs";
import { FileTree, prepareFileTreeInput, type GitStatusEntry } from "@pierre/trees";

const MAX_DIFF_SOURCE_CHARS = 160_000;
const DIFF_THEME = {
  dark: "github-dark",
  light: "github-light"
} as const;

const diffInstances: Array<{ root: HTMLElement; cleanUp(recycle?: boolean): void }> = [];
const treeInstances: Array<{ root: HTMLElement; cleanUp(): void }> = [];
type PierreLanguage = NonNullable<FileContents["lang"]>;
type DiffReviewTreeEntry = {
  path: string;
  status?: string;
  additions?: number;
  deletions?: number;
};

export async function enhanceDiffPreviews(): Promise<void> {
  const mounts = Array.from(document.querySelectorAll<HTMLElement>(".diffs-mount:not([data-diffs-state])"));
  for (const mount of mounts) {
    renderEnhancedDiff(mount);
  }
  renderDiffReviewTrees();
}

export function cleanupDiffEnhancements(): void {
  while (diffInstances.length) {
    const instance = diffInstances.pop();
    try {
      instance?.cleanUp();
    } catch {
      // A rerender is already replacing this DOM; leaking an old diff instance would be worse than ignoring cleanup noise.
    }
  }
  while (treeInstances.length) {
    const instance = treeInstances.pop();
    try {
      instance?.cleanUp();
    } catch {
      // The drawer is going away; ignore tree disposal noise.
    }
  }
}

export function cleanupDetachedEnhancements(): void {
  cleanupDetachedDiffInstances();
  cleanupDetachedTreeInstances();
}

function renderEnhancedDiff(mount: HTMLElement): void {
  const preview = mount.closest<HTMLElement>(".diff-preview");
  const oldText = templateText(preview, "template.diff-old-source");
  const newText = templateText(preview, "template.diff-new-source");
  const patchText = templateText(preview, "template.diff-patch-source");
  const path = mount.dataset.diffsPath || "changes";
  const language = pierreLanguage(mount.dataset.diffsLanguage || "");

  if (!preview || oldText.length + newText.length + patchText.length > MAX_DIFF_SOURCE_CHARS) {
    mount.dataset.diffsState = "skipped";
    return;
  }

  mount.dataset.diffsState = "loading";
  try {
    const instance = new FileDiff({
      theme: DIFF_THEME,
      themeType: currentThemeType(),
      diffStyle: "split",
      overflow: "scroll",
      lineDiffType: "word",
      hunkSeparators: "line-info-basic",
      diffIndicators: "bars",
      disableFileHeader: true,
      disableVirtualizationBuffers: true,
      tokenizeMaxLength: MAX_DIFF_SOURCE_CHARS,
      tokenizeMaxLineLength: 500
    });
    const patchFile = patchText ? fileDiffFromPatch(path, patchText) : undefined;
    if (patchFile) {
      instance.render({ fileDiff: patchFile, containerWrapper: mount });
    } else {
      const oldFile = fileContents(path, oldText, language);
      const newFile = fileContents(path, newText, language);
      instance.render({ oldFile, newFile, containerWrapper: mount });
    }
    diffInstances.push({ root: mount, cleanUp: instance.cleanUp.bind(instance) });
    preview.classList.add("diff-preview-enhanced");
    mount.dataset.diffsState = "ready";
  } catch {
    preview.classList.add("diff-preview-fallback-only");
    mount.dataset.diffsState = "failed";
  }
}

function renderDiffReviewTrees(): void {
  const mounts = Array.from(document.querySelectorAll<HTMLElement>("[data-diff-review-tree]:not([data-tree-state])"));
  for (const mount of mounts) {
    renderDiffReviewTree(mount);
  }
}

function renderDiffReviewTree(mount: HTMLElement): void {
  const drawer = mount.closest<HTMLElement>(".diff-review-drawer");
  const entries = diffReviewTreeEntries(drawer);
  if (!drawer || !entries.length) {
    mount.dataset.treeState = "skipped";
    return;
  }
  const paths = entries.map((entry) => entry.path);
  const entryByPath = new Map(entries.map((entry) => [entry.path, entry]));
  const selectedPath = drawer.querySelector<HTMLElement>(".diff-review-file.active")?.dataset.diffReviewFile || paths[0];

  mount.dataset.treeState = "loading";
  try {
    const tree = new FileTree({
      flattenEmptyDirectories: true,
      gitStatus: entries.map(treeGitStatus),
      initialExpansion: "open",
      initialSelectedPaths: selectedPath ? [selectedPath] : [],
      paths,
      preparedInput: prepareFileTreeInput(paths, { flattenEmptyDirectories: true }),
      renderRowDecoration: ({ item }) => {
        const entry = entryByPath.get(item.path);
        if (!entry || item.kind !== "file") {
          return null;
        }
        return {
          text: `+${entry.additions || 0} -${entry.deletions || 0}`,
          title: `${entry.additions || 0} additions, ${entry.deletions || 0} deletions`
        };
      },
      search: true,
      searchBlurBehavior: "retain",
      unsafeCSS: diffReviewTreeCSS(),
      onSelectionChange: (selectedPaths) => {
        const path = selectedPaths[0];
        if (path) {
          selectDiffReviewFile(drawer, path);
        }
      }
    });
    tree.render({ containerWrapper: mount });
    treeInstances.push({ root: mount, cleanUp: tree.cleanUp.bind(tree) });
    mount.dataset.treeState = "ready";
    drawer.querySelector<HTMLElement>(".diff-review-file-fallback")?.setAttribute("hidden", "true");
  } catch {
    mount.dataset.treeState = "failed";
  }
}

function templateText(root: Element | null | undefined, selector: string): string {
  return root?.querySelector<HTMLTemplateElement>(selector)?.content.textContent || "";
}

function fileDiffFromPatch(path: string, patchText: string): FileDiffMetadata | undefined {
  const parsed = parsePatchFiles(patchText, `crabdb-${path}`, false);
  return parsed.flatMap((patch) => patch.files).find((file) => file.name === path || file.prevName === path) || parsed[0]?.files[0];
}

function fileContents(name: string, contents: string, lang: PierreLanguage | undefined): FileContents {
  return lang ? { name, contents, lang } : { name, contents };
}

function currentThemeType(): "dark" | "light" | "system" {
  if (document.body.classList.contains("vscode-light")) {
    return "light";
  }
  if (document.body.classList.contains("vscode-dark") || document.body.classList.contains("vscode-high-contrast")) {
    return "dark";
  }
  return "system";
}

function pierreLanguage(language: string): PierreLanguage | undefined {
  switch (language) {
    case "plaintext":
      return "text";
    case "shellscript":
      return "shellscript";
    default:
      return language as FileContents["lang"];
  }
}

function diffReviewTreeEntries(drawer: HTMLElement | null): DiffReviewTreeEntry[] {
  const text = drawer?.querySelector<HTMLTemplateElement>("template.diff-review-tree-data")?.content.textContent || "[]";
  try {
    const parsed = JSON.parse(text);
    return Array.isArray(parsed) ? parsed.filter(isDiffReviewTreeEntry) : [];
  } catch {
    return [];
  }
}

function isDiffReviewTreeEntry(value: unknown): value is DiffReviewTreeEntry {
  return Boolean(value && typeof value === "object" && typeof (value as DiffReviewTreeEntry).path === "string");
}

function treeGitStatus(entry: DiffReviewTreeEntry): GitStatusEntry {
  return {
    path: entry.path,
    status: treeGitStatusValue(entry.status)
  };
}

function treeGitStatusValue(status: string | undefined): GitStatusEntry["status"] {
  switch (status) {
    case "added":
      return "added";
    case "deleted":
      return "deleted";
    case "renamed":
      return "renamed";
    case "untracked":
      return "untracked";
    default:
      return "modified";
  }
}

function selectDiffReviewFile(drawer: HTMLElement, path: string): void {
  drawer.querySelectorAll<HTMLElement>("[data-diff-review-path]").forEach((element) => {
    const active = element.dataset.diffReviewPath === path;
    element.classList.toggle("active", active);
    if (element instanceof HTMLButtonElement) {
      element.setAttribute("aria-pressed", active ? "true" : "false");
    }
  });
  drawer.querySelectorAll<HTMLElement>("[data-diff-review-file]").forEach((element) => {
    const active = element.dataset.diffReviewFile === path;
    element.hidden = !active;
    element.classList.toggle("active", active);
  });
}

function cleanupDetachedDiffInstances(): void {
  for (let index = diffInstances.length - 1; index >= 0; index -= 1) {
    const instance = diffInstances[index];
    if (instance && !instance.root.isConnected) {
      try {
        instance.cleanUp();
      } catch {
        // Detached roots are already gone.
      }
      diffInstances.splice(index, 1);
    }
  }
}

function cleanupDetachedTreeInstances(): void {
  for (let index = treeInstances.length - 1; index >= 0; index -= 1) {
    const instance = treeInstances[index];
    if (instance && !instance.root.isConnected) {
      try {
        instance.cleanUp();
      } catch {
        // Detached roots are already gone.
      }
      treeInstances.splice(index, 1);
    }
  }
}

function diffReviewTreeCSS(): string {
  return `
    :host {
      --trees-bg-override: transparent;
      --trees-fg-override: var(--vscode-foreground, #d7dde8);
      --trees-border-color-override: color-mix(in srgb, var(--vscode-widget-border, #303849) 82%, transparent);
      --trees-selected-bg-override: color-mix(in srgb, var(--vscode-focusBorder, #6ea0ff) 16%, transparent);
      color: var(--vscode-foreground, #d7dde8);
      font-family: var(--vscode-font-family, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif);
      font-size: var(--vscode-font-size, 13px);
    }

    button[data-type="item"] {
      border-radius: 4px;
    }
  `;
}

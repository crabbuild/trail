export interface FilePreviewInput {
  path: string;
  language: string;
  text: string;
  maxChars: number;
}

export interface FilePreviewBadge {
  label: string;
  tone: "default" | "ok" | "warning";
  detail?: string | undefined;
}

export interface FilePreviewModel {
  title: string;
  language: string;
  highlightSupported: boolean;
  text: string;
  truncated: boolean;
  lineCount: number;
  charCount: number;
  metaLabel: string;
  badges: FilePreviewBadge[];
  accessibilityLabel: string;
}

export function buildFilePreviewModel(input: FilePreviewInput): FilePreviewModel {
  const title = cleanText(input.path) || "Read result";
  const language = cleanText(input.language) || "plaintext";
  const highlightSupported = isHighlightSupported(language);
  const maxChars = Number.isFinite(input.maxChars) && input.maxChars > 0 ? Math.floor(input.maxChars) : 60_000;
  const text = input.text || "";
  const truncated = truncateText(text, maxChars);
  const charCount = Array.from(text).length;
  const lineCount = countLines(text);
  const metaLabel = `${formatCount(lineCount)} line${lineCount === 1 ? "" : "s"} - ${formatCount(charCount)} char${charCount === 1 ? "" : "s"}`;
  const badges: FilePreviewBadge[] = [
    { label: `${formatCount(lineCount)} line${lineCount === 1 ? "" : "s"}`, tone: "default" },
    { label: `${formatCount(charCount)} char${charCount === 1 ? "" : "s"}`, tone: "default" }
  ];
  if (truncated.truncated) {
    badges.push({ label: `Truncated at ${formatCount(maxChars)}`, tone: "warning" });
  }
  return {
    title,
    language,
    highlightSupported,
    text: truncated.text,
    truncated: truncated.truncated,
    lineCount,
    charCount,
    metaLabel,
    badges,
    accessibilityLabel: `${title}, ${language}, ${formatCount(lineCount)} line${lineCount === 1 ? "" : "s"}`
  };
}

function isHighlightSupported(language: string): boolean {
  switch (language.toLowerCase()) {
    case "css":
    case "diff":
    case "go":
    case "html":
    case "javascript":
    case "json":
    case "jsx":
    case "markdown":
    case "python":
    case "rust":
    case "shellscript":
    case "tsx":
    case "typescript":
    case "xml":
    case "yaml":
      return true;
    default:
      return false;
  }
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

function countLines(value: string): number {
  return value ? value.split("\n").length : 0;
}

function cleanText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function formatCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(Math.max(0, Math.floor(value)));
}

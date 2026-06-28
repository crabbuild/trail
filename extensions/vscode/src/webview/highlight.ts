import css from "@shikijs/langs/css";
import diff from "@shikijs/langs/diff";
import go from "@shikijs/langs/go";
import html from "@shikijs/langs/html";
import javascript from "@shikijs/langs/javascript";
import json from "@shikijs/langs/json";
import jsx from "@shikijs/langs/jsx";
import markdown from "@shikijs/langs/markdown";
import python from "@shikijs/langs/python";
import rust from "@shikijs/langs/rust";
import shellscript from "@shikijs/langs/shellscript";
import tsx from "@shikijs/langs/tsx";
import typescript from "@shikijs/langs/typescript";
import xml from "@shikijs/langs/xml";
import yaml from "@shikijs/langs/yaml";
import githubDark from "@shikijs/themes/github-dark";
import githubLight from "@shikijs/themes/github-light";
import { createHighlighterCore, type HighlighterCore, type ThemedTokenWithVariants, type TokenStyles } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import { normalizeHighlightSource } from "./highlightSourceModel";

const MAX_HIGHLIGHT_CHARS = 80_000;
const MAX_TOKENIZED_LINE_LENGTH = 2_000;
const SHIKI_DARK_THEME = "github-dark";
const SHIKI_LIGHT_THEME = "github-light";
const SHIKI_LANGUAGES = [
  "css",
  "diff",
  "go",
  "html",
  "javascript",
  "json",
  "jsx",
  "markdown",
  "python",
  "rust",
  "shellscript",
  "tsx",
  "typescript",
  "xml",
  "yaml"
] as const;

type ShikiLanguage = (typeof SHIKI_LANGUAGES)[number];

const SHIKI_LANGUAGE_SET = new Set<string>(SHIKI_LANGUAGES);
let shikiHighlighterPromise: Promise<HighlighterCore> | undefined;

export async function highlightCodeBlocks(): Promise<void> {
  const blocks = Array.from(document.querySelectorAll<HTMLPreElement>("pre.code[data-highlight-language]"));
  for (const block of blocks) {
    await highlightCodeBlock(block);
  }
}

function getShikiHighlighter(): Promise<HighlighterCore> {
  if (!shikiHighlighterPromise) {
    shikiHighlighterPromise = createHighlighterCore({
      themes: [githubDark, githubLight],
      langs: [css, diff, go, html, javascript, json, jsx, markdown, python, rust, shellscript, tsx, typescript, xml, yaml],
      engine: createJavaScriptRegexEngine(),
      warnings: false
    }).catch((error) => {
      shikiHighlighterPromise = undefined;
      throw error;
    });
  }
  return shikiHighlighterPromise;
}

async function highlightCodeBlock(block: HTMLPreElement): Promise<void> {
  if (block.dataset.highlightState) {
    return;
  }

  const language = normalizeHighlightLanguage(block.dataset.highlightLanguage || "");
  if (!language) {
    block.dataset.highlightState = "skipped";
    return;
  }

  const source = normalizeHighlightSource(block.textContent || "", lineStartFromBlock(block));
  applyLineStart(block, source.lineStart);
  if (!source.text || source.text.length > MAX_HIGHLIGHT_CHARS) {
    block.dataset.highlightState = source.text.length > MAX_HIGHLIGHT_CHARS ? "too-large" : "empty";
    return;
  }

  block.dataset.highlightState = "pending";
  try {
    const highlighter = await getShikiHighlighter();
    const tokens = highlighter.codeToTokensWithThemes(source.text, {
      lang: language,
      themes: { light: SHIKI_LIGHT_THEME, dark: SHIKI_DARK_THEME },
      tokenizeMaxLineLength: MAX_TOKENIZED_LINE_LENGTH
    });
    if (!block.isConnected || block.dataset.highlightState !== "pending") {
      return;
    }
    block.innerHTML = renderHighlightedLines(tokens, language);
    block.classList.add("highlighted");
    block.dataset.highlightState = "highlighted";
  } catch {
    if (block.isConnected) {
      block.dataset.highlightState = "failed";
    }
  }
}

function lineStartFromBlock(block: HTMLPreElement): number | undefined {
  const lineStart = Number(block.dataset.lineStart);
  return Number.isFinite(lineStart) && lineStart > 1 ? lineStart : undefined;
}

function applyLineStart(block: HTMLPreElement, lineStart: number): void {
  if (lineStart <= 1) {
    return;
  }
  block.dataset.lineStart = String(lineStart);
  block.style.setProperty("--code-line-start", String(lineStart - 1));
}

function normalizeHighlightLanguage(value: string): ShikiLanguage | undefined {
  const normalized = cleanLanguage(value).toLowerCase();
  switch (normalized) {
    case "bash":
    case "shell":
    case "sh":
    case "zsh":
      return "shellscript";
    case "golang":
      return "go";
    case "js":
    case "mjs":
    case "cjs":
      return "javascript";
    case "md":
      return "markdown";
    case "py":
      return "python";
    case "rs":
      return "rust";
    case "ts":
      return "typescript";
    case "yml":
      return "yaml";
    default:
      return SHIKI_LANGUAGE_SET.has(normalized) ? (normalized as ShikiLanguage) : undefined;
  }
}

function cleanLanguage(value: string): string {
  const cleaned = value.trim().replace(/[^a-zA-Z0-9_+.-]/g, "").slice(0, 40);
  return cleaned || "plaintext";
}

function renderHighlightedLines(lines: ThemedTokenWithVariants[][], language: ShikiLanguage): string {
  return lines
    .map((line) => {
      const text = line.map((token) => token.content).join("");
      const classes = ["code-line", ...codeLineToneClasses(text, language)];
      return `<span class="${classes.join(" ")}">${line.map(renderHighlightedToken).join("")}</span>`;
    })
    .join("");
}

function renderHighlightedToken(token: ThemedTokenWithVariants): string {
  const style = tokenStyle(token.variants.light, token.variants.dark);
  return `<span class="shiki-token"${style ? ` style="${style}"` : ""}>${escapeHtml(token.content)}</span>`;
}

function codeLineToneClasses(text: string, language: ShikiLanguage): string[] {
  if (language !== "diff") {
    return [];
  }
  if (text.startsWith("+")) {
    return ["code-line-added"];
  }
  if (text.startsWith("-")) {
    return ["code-line-removed"];
  }
  if (text.startsWith("@")) {
    return ["code-line-meta"];
  }
  return [];
}

function tokenStyle(light: TokenStyles | undefined, dark: TokenStyles | undefined): string {
  const declarations: string[] = [];
  const lightColor = cssColor(light?.color);
  const darkColor = cssColor(dark?.color);
  const lightBg = cssColor(light?.bgColor);
  const darkBg = cssColor(dark?.bgColor);
  if (lightColor) {
    declarations.push(`--shiki-light:${lightColor}`, "color:var(--shiki-light)");
  }
  if (darkColor) {
    declarations.push(`--shiki-dark:${darkColor}`);
  }
  if (lightBg) {
    declarations.push(`--shiki-light-bg:${lightBg}`, "background-color:var(--shiki-light-bg)");
  }
  if (darkBg) {
    declarations.push(`--shiki-dark-bg:${darkBg}`);
  }
  declarations.push(...fontStyleDeclarations("light", light?.fontStyle));
  declarations.push(...fontStyleDeclarations("dark", dark?.fontStyle));
  return escapeHtml(declarations.join(";"));
}

function cssColor(color: string | undefined): string | undefined {
  const match = color?.trim().match(/^#(?:[0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i);
  return match ? match[0] : undefined;
}

function fontStyleDeclarations(theme: "light" | "dark", fontStyle: number | undefined): string[] {
  if (typeof fontStyle !== "number" || fontStyle < 0) {
    return [];
  }
  const declarations = [
    `--shiki-${theme}-font-style:${fontStyle & 1 ? "italic" : "normal"}`,
    `--shiki-${theme}-font-weight:${fontStyle & 2 ? "650" : "inherit"}`,
    `--shiki-${theme}-text-decoration:${fontStyle & 4 ? "underline" : "none"}`
  ];
  if (theme === "light") {
    declarations.push(
      "font-style:var(--shiki-light-font-style)",
      "font-weight:var(--shiki-light-font-weight)",
      "text-decoration:var(--shiki-light-text-decoration)",
      "text-underline-offset:2px"
    );
  }
  return declarations;
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      default:
        return "&#39;";
    }
  });
}

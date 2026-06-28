export interface MarkdownRenderOptions {
  maxChars?: number | undefined;
  renderCodeBlock?: ((text: string, language: string) => string) | undefined;
}

interface MarkdownTokenStore {
  values: string[];
}

const TOKEN_PREFIX = "\ue000";
const TOKEN_SUFFIX = "\ue001";

export function renderMarkdown(text: string, options: MarkdownRenderOptions = {}): string {
  const truncated = truncateText(text, options.maxChars ?? Number.MAX_SAFE_INTEGER);
  const body = renderMarkdownBlocks(truncated.text, options);
  const notice = truncated.truncated
    ? `<p class="muted">Message preview truncated after ${options.maxChars} characters.</p>`
    : "";
  return body + notice;
}

function renderMarkdownBlocks(text: string, options: MarkdownRenderOptions): string {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const html: string[] = [];
  let index = 0;
  while (index < lines.length) {
    const line = lines[index] || "";
    if (!line.trim()) {
      index += 1;
      continue;
    }

    const fence = line.match(/^\s*```([^\n`]*)\s*$/);
    if (fence) {
      const language = cleanLanguage(fence[1] || "plaintext");
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !/^\s*```\s*$/.test(lines[index] || "")) {
        code.push(lines[index] || "");
        index += 1;
      }
      if (index < lines.length) {
        index += 1;
      }
      html.push(renderCodeBlock(code.join("\n"), language, options));
      continue;
    }

    if (isTableStart(lines, index)) {
      const table = collectTable(lines, index);
      html.push(renderTable(table.header, table.alignments, table.rows));
      index = table.nextIndex;
      continue;
    }

    const heading = line.match(/^\s{0,3}(#{1,6})\s+(.+?)\s*#*\s*$/);
    if (heading) {
      const level = heading[1]?.length || 1;
      html.push(`<h${level}>${renderInline(heading[2] || "")}</h${level}>`);
      index += 1;
      continue;
    }

    if (/^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
      html.push("<hr>");
      index += 1;
      continue;
    }

    if (/^\s{0,3}>\s?/.test(line)) {
      const quoteLines: string[] = [];
      while (index < lines.length && /^\s{0,3}>\s?/.test(lines[index] || "")) {
        quoteLines.push((lines[index] || "").replace(/^\s{0,3}>\s?/, ""));
        index += 1;
      }
      html.push(`<blockquote>${renderMarkdownBlocks(quoteLines.join("\n"), options)}</blockquote>`);
      continue;
    }

    const list = listMarker(line);
    if (list) {
      const ordered = list.ordered;
      const items: string[] = [];
      while (index < lines.length) {
        const item = listMarker(lines[index] || "");
        if (!item || item.ordered !== ordered) {
          break;
        }
        items.push(renderListItem(item.content));
        index += 1;
      }
      html.push(`<${ordered ? "ol" : "ul"}>${items.join("")}</${ordered ? "ol" : "ul"}>`);
      continue;
    }

    const paragraph: string[] = [];
    while (index < lines.length && lines[index]?.trim() && !isBlockStart(lines, index)) {
      paragraph.push(lines[index] || "");
      index += 1;
    }
    html.push(`<p>${paragraph.map(renderInline).join("<br>")}</p>`);
  }
  return html.join("");
}

function isBlockStart(lines: string[], index: number): boolean {
  const line = lines[index] || "";
  return (
    /^\s*```/.test(line) ||
    isTableStart(lines, index) ||
    /^\s{0,3}(#{1,6})\s+/.test(line) ||
    /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line) ||
    /^\s{0,3}>\s?/.test(line) ||
    Boolean(listMarker(line))
  );
}

function renderCodeBlock(text: string, language: string, options: MarkdownRenderOptions): string {
  if (options.renderCodeBlock) {
    return options.renderCodeBlock(text, language);
  }
  return `<pre><code class="language-${escapeHtml(language)}">${escapeHtml(text)}</code></pre>`;
}

function renderListItem(content: string): string {
  const task = content.match(/^\[( |x|X)\]\s+(.*)$/);
  if (task) {
    const checked = task[1]?.toLowerCase() === "x";
    return `<li class="task-list-item"><input class="task-list-checkbox" type="checkbox" disabled${checked ? " checked" : ""}>${renderInline(task[2] || "")}</li>`;
  }
  return `<li>${renderInline(content)}</li>`;
}

function listMarker(line: string): { ordered: boolean; content: string } | undefined {
  const unordered = line.match(/^\s{0,3}[-+*]\s+(.+)$/);
  if (unordered) {
    return { ordered: false, content: unordered[1] || "" };
  }
  const ordered = line.match(/^\s{0,3}\d+[.)]\s+(.+)$/);
  if (ordered) {
    return { ordered: true, content: ordered[1] || "" };
  }
  return undefined;
}

function isTableStart(lines: string[], index: number): boolean {
  const header = lines[index] || "";
  const separator = lines[index + 1] || "";
  if (!header.includes("|") || !separator.includes("|")) {
    return false;
  }
  const headerCells = splitTableRow(header);
  const separatorCells = splitTableRow(separator);
  return headerCells.length > 1 && separatorCells.length === headerCells.length && separatorCells.every(isSeparatorCell);
}

function collectTable(
  lines: string[],
  start: number
): { header: string[]; alignments: string[]; rows: string[][]; nextIndex: number } {
  const header = splitTableRow(lines[start] || "");
  const separator = splitTableRow(lines[start + 1] || "");
  const alignments = separator.map(tableAlignment);
  const rows: string[][] = [];
  let index = start + 2;
  while (index < lines.length && lines[index]?.includes("|") && lines[index]?.trim()) {
    rows.push(normalizeTableRow(splitTableRow(lines[index] || ""), header.length));
    index += 1;
  }
  return { header, alignments, rows, nextIndex: index };
}

function renderTable(header: string[], alignments: string[], rows: string[][]): string {
  return `
    <div class="markdown-table-wrap">
      <table>
        <thead><tr>${header.map((cell, index) => `<th class="${tableAlignClass(alignments[index])}">${renderInline(cell)}</th>`).join("")}</tr></thead>
        <tbody>${rows
          .map((row) => `<tr>${row.map((cell, index) => `<td class="${tableAlignClass(alignments[index])}">${renderInline(cell)}</td>`).join("")}</tr>`)
          .join("")}</tbody>
      </table>
    </div>
  `;
}

function splitTableRow(line: string): string[] {
  let value = line.trim();
  if (value.startsWith("|")) {
    value = value.slice(1);
  }
  if (value.endsWith("|")) {
    value = value.slice(0, -1);
  }
  return value.split("|").map((cell) => cell.trim());
}

function normalizeTableRow(row: string[], length: number): string[] {
  return Array.from({ length }, (_, index) => row[index] || "");
}

function isSeparatorCell(cell: string): boolean {
  return /^:?-{3,}:?$/.test(cell.trim());
}

function tableAlignment(cell: string): string {
  const trimmed = cell.trim();
  if (trimmed.startsWith(":") && trimmed.endsWith(":")) {
    return "center";
  }
  if (trimmed.endsWith(":")) {
    return "right";
  }
  return "left";
}

function tableAlignClass(value: string | undefined): string {
  return `align-${value || "left"}`;
}

function renderInline(text: string): string {
  if (!text) {
    return "";
  }
  const tokens: MarkdownTokenStore = { values: [] };
  let value = text
    .replace(/`([^`\n]+)`/g, (_match, code: string) => token(tokens, `<code>${escapeHtml(code)}</code>`))
    .replace(/!\[([^\]]*)\]\(([^)\s]+)(?:\s+"([^"]+)")?\)/g, (_match, alt: string, href: string, title: string) =>
      imageToken(tokens, alt, href, title)
    )
    .replace(/\[([^\]]+)\]\(([^)\s]+)(?:\s+"([^"]+)")?\)/g, (_match, label: string, href: string, title: string) =>
      linkToken(tokens, label, href, title)
    )
    .replace(/<((?:https?:\/\/|mailto:)[^>\s]+)>/g, (_match, href: string) => linkToken(tokens, href, href))
    .replace(/\bhttps?:\/\/[^\s<]+/g, (href: string) => linkToken(tokens, href, href));

  value = escapeHtml(value)
    .replace(/~~([^~]+)~~/g, "<del>$1</del>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/__([^_]+)__/g, "<strong>$1</strong>")
    .replace(/\*([^*\n]+)\*/g, "<em>$1</em>");

  return restoreTokens(value, tokens);
}

function linkToken(tokens: MarkdownTokenStore, label: string, href: string, title?: string): string {
  const safe = safeHref(href);
  if (!safe) {
    return label;
  }
  const titleAttr = title ? ` title="${escapeHtml(title)}"` : "";
  return token(tokens, `<a href="${escapeHtml(safe)}"${titleAttr} rel="noreferrer">${escapeHtml(label)}</a>`);
}

function imageToken(tokens: MarkdownTokenStore, alt: string, href: string, title?: string): string {
  const safe = safeImageHref(href);
  if (!safe) {
    return alt;
  }
  const titleAttr = title ? ` title="${escapeHtml(title)}"` : "";
  return token(tokens, `<img class="markdown-image" src="${escapeHtml(safe)}" alt="${escapeHtml(alt)}"${titleAttr}>`);
}

function token(tokens: MarkdownTokenStore, html: string): string {
  const index = tokens.values.push(html) - 1;
  return `${TOKEN_PREFIX}${index}${TOKEN_SUFFIX}`;
}

function restoreTokens(value: string, tokens: MarkdownTokenStore): string {
  return value.replace(new RegExp(`${TOKEN_PREFIX}(\\d+)${TOKEN_SUFFIX}`, "g"), (_match, index: string) => {
    return tokens.values[Number(index)] || "";
  });
}

function safeHref(value: string): string {
  const trimmed = value.trim();
  if (trimmed.startsWith("#")) {
    return trimmed;
  }
  try {
    const url = new URL(trimmed);
    return ["http:", "https:", "mailto:"].includes(url.protocol) ? trimmed : "";
  } catch {
    return "";
  }
}

function safeImageHref(value: string): string {
  const trimmed = value.trim();
  try {
    const url = new URL(trimmed);
    return ["http:", "https:"].includes(url.protocol) ? trimmed : "";
  } catch {
    return "";
  }
}

function cleanLanguage(value: string): string {
  return value.trim().split(/\s+/)[0]?.replace(/[^\w#+.-]/g, "") || "plaintext";
}

function truncateText(value: string, limit: number): { text: string; truncated: boolean } {
  if (value.length <= limit) {
    return { text: value, truncated: false };
  }
  return { text: value.slice(0, limit), truncated: true };
}

function escapeHtml(value: unknown): string {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

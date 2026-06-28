export interface HighlightSource {
  text: string;
  lineStart: number;
}

export function normalizeHighlightSource(value: string, lineStart?: number | undefined): HighlightSource {
  const explicitLineStart = validLineStart(lineStart);
  const normalized = value.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const stripped = stripLineNumberPrefixes(normalized);
  return {
    text: stripped?.text || normalized,
    lineStart: explicitLineStart || stripped?.lineStart || 1
  };
}

function stripLineNumberPrefixes(value: string): HighlightSource | undefined {
  const lines = value.split("\n");
  if (lines.length < 3) {
    return undefined;
  }

  const prefixes = lines.map(numberedLinePrefix);
  const offsets = new Map<number, number>();
  prefixes.forEach((prefix, index) => {
    if (prefix) {
      const offset = prefix.lineNumber - index;
      offsets.set(offset, (offsets.get(offset) || 0) + 1);
    }
  });

  const winner = [...offsets.entries()].sort((a, b) => b[1] - a[1])[0];
  if (!winner || winner[1] < 3) {
    return undefined;
  }

  let numberedCount = 0;
  const lineStart = winner[0];
  const strippedLines = lines.map((line, index) => {
    const expected = lineStart + index;
    const prefix = prefixes[index];
    if (prefix?.lineNumber === expected) {
      numberedCount += 1;
      return prefix.text;
    }
    if (bareLineNumber(line) === expected) {
      numberedCount += 1;
      return "";
    }
    return line;
  });

  const meaningfulLineCount = lines.filter((line) => line.trim()).length;
  if (numberedCount < 3 || numberedCount < Math.ceil(meaningfulLineCount * 0.72)) {
    return undefined;
  }

  return { text: strippedLines.join("\n"), lineStart };
}

function numberedLinePrefix(line: string): { lineNumber: number; text: string } | undefined {
  const match = line.match(/^\s{0,4}(\d{1,7})(?:[ \t]*(?:\||\u2502)[ \t]?|[ \t]+)(.*)$/);
  const lineNumber = match ? Number(match[1]) : 0;
  return Number.isSafeInteger(lineNumber) && lineNumber > 0 ? { lineNumber, text: match?.[2] || "" } : undefined;
}

function bareLineNumber(line: string): number | undefined {
  const match = line.match(/^\s{0,4}(\d{1,7})\s*$/);
  const lineNumber = match ? Number(match[1]) : 0;
  return Number.isSafeInteger(lineNumber) && lineNumber > 0 ? lineNumber : undefined;
}

function validLineStart(value: number | undefined): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return undefined;
  }
  const lineStart = Math.floor(value);
  return lineStart > 1 ? lineStart : undefined;
}

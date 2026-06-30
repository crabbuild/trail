export function textContentValue(content: unknown): string | undefined {
  const record = asRecord(content);
  if (record.type !== "text") {
    return undefined;
  }
  const fallbackValues: string[] = [];
  for (const key of ["text", "content", "value"]) {
    const value = stringField(record, key);
    if (value === undefined) {
      continue;
    }
    if (value.length > 0) {
      return value;
    }
    fallbackValues.push(value);
  }
  return fallbackValues[0];
}

export function textOnlyContent(blocks: readonly unknown[]): string | undefined {
  if (!blocks.length) {
    return undefined;
  }
  const values: string[] = [];
  for (const block of blocks) {
    const record = asRecord(block);
    if (record.type !== "text") {
      return undefined;
    }
    const text = textContentValue(record);
    if (text === undefined) {
      return undefined;
    }
    values.push(text);
  }
  return values.join("");
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

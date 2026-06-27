export interface MergeQueueEntry {
  id: string;
  sourceRef: string;
  targetRef: string;
  status: string;
  priority: number;
  createdAt?: number | string | undefined;
  updatedAt?: number | string | undefined;
  raw: unknown;
}

export function normalizeMergeQueueList(report: unknown): MergeQueueEntry[] {
  if (Array.isArray(report)) {
    return report.map((item, index) => normalizeMergeQueueEntry(item, `queue-${index}`));
  }
  const root = asRecord(report);
  const entries = Array.isArray(root.entries)
    ? root.entries
    : Array.isArray(root.items)
      ? root.items
      : Array.isArray(root.queue)
        ? root.queue
        : [];
  return entries.map((item, index) => normalizeMergeQueueEntry(item, `queue-${index}`));
}

function normalizeMergeQueueEntry(value: unknown, fallback: string): MergeQueueEntry {
  const entry = asRecord(value);
  const id = stringField(entry, "queue_id") || stringField(entry, "queueId") || stringField(entry, "id") || fallback;
  return {
    id,
    sourceRef:
      stringField(entry, "source_ref") ||
      stringField(entry, "sourceRef") ||
      stringField(entry, "source") ||
      stringField(entry, "lane") ||
      id,
    targetRef:
      stringField(entry, "target_ref") ||
      stringField(entry, "targetRef") ||
      stringField(entry, "target") ||
      stringField(entry, "into") ||
      "main",
    status: stringField(entry, "status") || "queued",
    priority: numberField(entry, "priority") ?? 0,
    createdAt: scalarField(entry, "created_at") ?? scalarField(entry, "createdAt"),
    updatedAt: scalarField(entry, "updated_at") ?? scalarField(entry, "updatedAt"),
    raw: value
  };
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function numberField(record: Record<string, unknown>, key: string): number | undefined {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function scalarField(record: Record<string, unknown>, key: string): number | string | undefined {
  const value = record[key];
  return typeof value === "string" || (typeof value === "number" && Number.isFinite(value)) ? value : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

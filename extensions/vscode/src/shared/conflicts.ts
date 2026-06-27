const MAX_CONFLICT_IDS = 20;
const MAX_DETAIL_DEPTH = 6;

const CONFLICT_ID_KEYS = new Set(["conflict_set_id", "conflictSetId"]);
const CONFLICT_ID_ARRAY_KEYS = new Set(["conflict_set_ids", "conflictSetIds"]);
const CONFLICT_CONTAINER_KEYS = new Set(["conflicts", "open_conflicts", "conflict_sets", "conflictSets"]);
const ISSUE_KEYS = ["blockers", "blocking", "failed_gates", "warnings"];

export function conflictSetIdsFromSources(...sources: unknown[]): string[] {
  const ids: string[] = [];
  const seen = new Set<string>();
  const push = (value: unknown) => {
    if (ids.length >= MAX_CONFLICT_IDS || typeof value !== "string") {
      return;
    }
    const id = value.trim();
    if (!id || seen.has(id) || !looksLikeIdentifier(id)) {
      return;
    }
    seen.add(id);
    ids.push(id);
  };

  for (const record of uniqueRecords(sources.flatMap(candidateRecords))) {
    collectRecordConflictIds(record, push);
    if (ids.length >= MAX_CONFLICT_IDS) {
      break;
    }
  }

  return ids;
}

function collectRecordConflictIds(record: Record<string, unknown>, push: (value: unknown) => void): void {
  for (const [key, value] of Object.entries(record)) {
    if (CONFLICT_ID_KEYS.has(key)) {
      push(value);
    } else if (CONFLICT_ID_ARRAY_KEYS.has(key)) {
      for (const item of arrayValue(value)) {
        push(item);
      }
    } else if (CONFLICT_CONTAINER_KEYS.has(key)) {
      for (const item of arrayValue(value)) {
        collectConflictContainerValue(item, push);
      }
    }
  }

  for (const key of ISSUE_KEYS) {
    for (const issue of arrayField(record, key)) {
      collectConflictContainerValue(issue, push);
      collectDetails(asRecord(issue).details, push);
    }
  }

  collectDetails(record.details, push);
}

function collectConflictContainerValue(value: unknown, push: (value: unknown) => void): void {
  if (typeof value === "string") {
    push(value);
    return;
  }
  const record = asRecord(value);
  if (!Object.keys(record).length) {
    return;
  }
  for (const [key, child] of Object.entries(record)) {
    if (CONFLICT_ID_KEYS.has(key)) {
      push(child);
    } else if (CONFLICT_ID_ARRAY_KEYS.has(key)) {
      for (const item of arrayValue(child)) {
        push(item);
      }
    } else if (CONFLICT_CONTAINER_KEYS.has(key)) {
      for (const item of arrayValue(child)) {
        collectConflictContainerValue(item, push);
      }
    }
  }
  collectDetails(record.details, push);
}

function collectDetails(value: unknown, push: (value: unknown) => void, depth = 0, seen = new Set<unknown>()): void {
  if (depth > MAX_DETAIL_DEPTH) {
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      collectDetails(item, push, depth + 1, seen);
    }
    return;
  }
  const record = asRecord(value);
  if (!Object.keys(record).length || seen.has(value)) {
    return;
  }
  seen.add(value);
  for (const [key, child] of Object.entries(record)) {
    if (CONFLICT_ID_KEYS.has(key)) {
      push(child);
    } else if (CONFLICT_ID_ARRAY_KEYS.has(key)) {
      for (const item of arrayValue(child)) {
        push(item);
      }
    } else if (CONFLICT_CONTAINER_KEYS.has(key)) {
      for (const item of arrayValue(child)) {
        collectConflictContainerValue(item, push);
      }
    } else if (child && typeof child === "object") {
      collectDetails(child, push, depth + 1, seen);
    }
  }
}

function candidateRecords(value: unknown): Record<string, unknown>[] {
  const record = asRecord(value);
  if (!Object.keys(record).length) {
    return [];
  }
  return [
    record,
    asRecord(record.task),
    asRecord(record.raw),
    asRecord(record.status),
    asRecord(record.readiness),
    asRecord(record.review),
    asRecord(asRecord(record.review).readiness),
    asRecord(record.evidence_summary)
  ].filter((candidate) => Object.keys(candidate).length > 0);
}

function looksLikeIdentifier(value: string): boolean {
  return value.length <= 180 && !/\s/.test(value);
}

function uniqueRecords(records: Record<string, unknown>[]): Record<string, unknown>[] {
  const seen = new Set<Record<string, unknown>>();
  return records.filter((record) => {
    if (seen.has(record)) {
      return false;
    }
    seen.add(record);
    return true;
  });
}

function arrayField(record: Record<string, unknown>, key: string): unknown[] {
  return arrayValue(record[key]);
}

function arrayValue(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

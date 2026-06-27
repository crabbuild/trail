export type CoordinationSeverity = "ok" | "warning" | "blocked";

export interface CoordinationIssue {
  code: string;
  message: string;
  tone: Exclude<CoordinationSeverity, "ok">;
  details?: unknown | undefined;
}

export interface CoordinationSummary {
  severity: CoordinationSeverity;
  labels: string[];
  issues: CoordinationIssue[];
  blockers: number;
  warnings: number;
  conflicts: number;
  pendingApprovals: number;
  queuedMerges: number;
  changedPaths: number;
  workdirDirty: boolean;
  staleBaseOperations?: number | undefined;
  latestTestStatus?: string | undefined;
  latestEvalStatus?: string | undefined;
}

export function coordinationSummaryFromSources(...sources: unknown[]): CoordinationSummary {
  const candidates = uniqueRecords(sources.flatMap(candidateRecords));
  const blockerIssues = candidates.flatMap((record) => issueList(record, "blockers", "blocked"));
  const warningIssues = candidates.flatMap((record) => issueList(record, "warnings", "warning"));
  const issues = uniqueIssues(blockerIssues.concat(warningIssues));
  const conflicts = maxNumber(
    candidates.map((record) => arrayField(record, "conflicts").length),
    candidates.map((record) => numberField(asRecord(record.evidence_summary), "conflicts")),
    blockerIssues.map((issue) => countIssueDetail(issue, "conflict_set_ids"))
  );
  const pendingApprovals = maxNumber(
    candidates.map((record) => arrayField(record, "pending_approvals").length),
    candidates.map((record) => numberField(asRecord(record.evidence_summary), "pending_approvals")),
    blockerIssues.map((issue) => countIssueDetail(issue, "approval_ids"))
  );
  const queuedMerges = maxNumber(
    candidates.map((record) => numberField(record, "queued_merges")),
    candidates.map((record) => numberField(asRecord(record.evidence_summary), "queued_merges"))
  );
  const changedPaths = maxNumber(
    candidates.map((record) => arrayField(record, "changed_paths").length),
    candidates.map((record) => arrayField(record, "changedPaths").length),
    candidates.map((record) => arrayField(record, "changes").length)
  );
  const staleBaseOperations = maxNumberOrUndefined(
    issues.map((issue) => numberField(asRecord(issueDetail(issue)), "operations_behind")),
    candidates.map((record) => numberField(asRecord(record.base_status), "operations_behind"))
  );
  const workdirDirty =
    issues.some((issue) => issue.code === "dirty_workdir") ||
    candidates.some((record) => {
      const state = stringField(record, "workdir_state");
      return Boolean(state && state.toLowerCase() !== "clean");
    });
  const latestTestStatus = latestGateStatus(candidates, "latest_test");
  const latestEvalStatus = latestGateStatus(candidates, "latest_eval");
  const blockers = blockerIssues.length;
  const warnings = warningIssues.length;
  const labels = compactLabels([
    blockers ? `${blockers} blocked` : "",
    conflicts ? `${conflicts} conflict${conflicts === 1 ? "" : "s"}` : "",
    pendingApprovals ? `${pendingApprovals} approval${pendingApprovals === 1 ? "" : "s"}` : "",
    staleBaseOperations ? `stale +${staleBaseOperations}` : "",
    queuedMerges ? "queued" : "",
    workdirDirty ? "dirty workdir" : "",
    missingGateLabel(issues),
    warnings && !blockers ? `${warnings} warning${warnings === 1 ? "" : "s"}` : ""
  ]);

  return {
    severity: blockers || conflicts || pendingApprovals || workdirDirty ? "blocked" : warnings ? "warning" : "ok",
    labels,
    issues,
    blockers,
    warnings,
    conflicts,
    pendingApprovals,
    queuedMerges,
    changedPaths,
    workdirDirty,
    staleBaseOperations,
    latestTestStatus,
    latestEvalStatus
  };
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

function issueList(
  record: Record<string, unknown>,
  key: "blockers" | "warnings",
  tone: CoordinationIssue["tone"]
): CoordinationIssue[] {
  return arrayField(record, key)
    .map((value) => {
      if (typeof value === "string") {
        return { code: value, message: value, tone };
      }
      const issue = asRecord(value);
      const code = stringField(issue, "code") || stringField(issue, "name") || tone;
      const message = stringField(issue, "message") || stringField(issue, "title") || code.replace(/_/g, " ");
      return { code, message, tone, details: issue.details };
    })
    .filter((issue) => issue.message);
}

function issueDetail(issue: CoordinationIssue): unknown {
  return issue.details;
}

function countIssueDetail(issue: CoordinationIssue, key: string): number {
  return arrayField(asRecord(issueDetail(issue)), key).length;
}

function latestGateStatus(candidates: Record<string, unknown>[], key: "latest_test" | "latest_eval"): string | undefined {
  for (const candidate of candidates) {
    const gate = asRecord(candidate[key]);
    const status = stringField(gate, "status") || stringField(gate, "outcome");
    if (status) {
      return status;
    }
  }
  return undefined;
}

function missingGateLabel(issues: CoordinationIssue[]): string {
  const missingTest = issues.some((issue) => issueCodeTokens(issue).includes("missing") && issueCodeTokens(issue).includes("test"));
  const missingEval = issues.some((issue) => issueCodeTokens(issue).includes("missing") && issueCodeTokens(issue).includes("eval"));
  if (missingTest && missingEval) {
    return "missing gates";
  }
  if (missingTest) {
    return "missing test";
  }
  if (missingEval) {
    return "missing eval";
  }
  return "";
}

function issueCodeTokens(issue: CoordinationIssue): string[] {
  return issue.code.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
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

function uniqueIssues(issues: CoordinationIssue[]): CoordinationIssue[] {
  const seen = new Set<string>();
  const result: CoordinationIssue[] = [];
  for (const issue of issues) {
    const key = `${issue.tone}:${issue.code}:${issue.message}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    result.push(issue);
  }
  return result;
}

function compactLabels(labels: string[]): string[] {
  const seen = new Set<string>();
  return labels.filter((label) => {
    if (!label || seen.has(label)) {
      return false;
    }
    seen.add(label);
    return true;
  });
}

function maxNumber(...groups: Array<Array<number | undefined>>): number {
  return Math.max(0, ...groups.flat().filter((value): value is number => typeof value === "number" && Number.isFinite(value)));
}

function maxNumberOrUndefined(...groups: Array<Array<number | undefined>>): number | undefined {
  const value = maxNumber(...groups);
  return value > 0 ? value : undefined;
}

function arrayField(record: Record<string, unknown>, key: string): unknown[] {
  const value = record[key];
  return Array.isArray(value) ? value : [];
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function numberField(record: Record<string, unknown>, key: string): number | undefined {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

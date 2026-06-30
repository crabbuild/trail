import * as fs from "node:fs";
import * as path from "node:path";
import type * as vscode from "vscode";
import { CrabDbCli } from "./CrabDbCli";
import { CrabDbDaemonClient, type DaemonEndpoint } from "./CrabDbDaemonClient";
import { coordinationSummaryFromSources, type CoordinationSummary } from "../shared/coordinationSummary";
import { normalizeMergeQueueList, type MergeQueueEntry } from "../shared/mergeQueue";

export type { MergeQueueEntry } from "../shared/mergeQueue";

export type AgentTaskStatus =
  | "empty"
  | "active"
  | "dirty"
  | "blocked"
  | "conflicted"
  | "ready"
  | "applied"
  | string;

export interface AgentTask {
  id: string;
  lane: string;
  title: string;
  status: AgentTaskStatus;
  provider?: string | undefined;
  model?: string | undefined;
  sessionId?: string | undefined;
  acpSessionId?: string | undefined;
  workdir?: string | undefined;
  latestCheckpoint?: string | undefined;
  changedPaths: string[];
  coordination?: CoordinationSummary | undefined;
  updatedAt?: string | undefined;
  nextAction?: string | undefined;
  raw: unknown;
}

export interface TaskView {
  task: AgentTask;
  turns: unknown[];
  messages: unknown[];
  events: unknown[];
  changes: unknown[];
  review?: unknown | undefined;
  readiness?: unknown | undefined;
  queue?: unknown | undefined;
  raw: unknown;
}

export interface LaneGateRequest {
  command: string[];
  timeoutSecs?: number | undefined;
  suite?: string | undefined;
}

export class TaskRepository {
  private readonly cli: CrabDbCli;
  private readonly daemon: CrabDbDaemonClient;
  private daemonEndpoint?: DaemonEndpoint | null;

  constructor(
    readonly workspaceRoot: string,
    output: vscode.OutputChannel
  ) {
    this.cli = new CrabDbCli(workspaceRoot, output);
    this.daemon = new CrabDbDaemonClient(workspaceRoot);
  }

  async listTasks(): Promise<AgentTask[]> {
    const report = await this.getDaemonJson<unknown>("/v1/lanes", () =>
      this.cli.runJson<unknown>(["agent", "list"], { timeoutMs: 30000 })
    );
    return this.enrichTasksWithReadiness(normalizeTaskList(report));
  }

  async latestTask(): Promise<AgentTask | undefined> {
    const tasks = await this.listTasks();
    return tasks[0];
  }

  async initWorkspace(): Promise<void> {
    const result = await this.cli.run(["init", "--quiet"], { timeoutMs: 30000 });
    if (result.code !== 0) {
      throw new Error(result.stderr.trim() || `CrabDB init failed with code ${result.code}`);
    }
  }

  isWorkspaceInitialized(): boolean {
    return fs.existsSync(path.join(this.workspaceRoot, ".crabdb"));
  }

  async viewTask(selector: string): Promise<TaskView> {
    const report = await this.cli.runJson<unknown>(["agent", "view", selector], { timeoutMs: 30000 });
    const view = normalizeTaskView(report, selector);
    const [review, readiness] = await Promise.all([
      this.tryDaemonJson(`/v1/lanes/${pathSegment(view.task.lane)}/review?limit=50`),
      this.tryDaemonJson(`/v1/lanes/${pathSegment(view.task.lane)}/readiness`)
    ]);
    const task = {
      ...view.task,
      coordination: coordinationSummaryFromSources(view.task.raw, review, readiness)
    };
    return {
      ...view,
      task,
      review,
      readiness
    };
  }

  async reviewTask(selector: string): Promise<unknown> {
    return this.getDaemonJson(`/v1/lanes/${pathSegment(selector)}/review?limit=50`, () =>
      this.cli.runJson<unknown>(["agent", "review", selector], { timeoutMs: 30000 })
    );
  }

  async diffTask(selector: string): Promise<unknown> {
    return this.getDaemonJson(`/v1/lanes/${pathSegment(selector)}/diff?patch=true`, () =>
      this.cli.runJson<unknown>(["agent", "diff", selector, "--stat", "--patch"], { timeoutMs: 30000 }).catch((error) => {
        if (isUnsupportedAgentSubcommand(error, "diff")) {
          return this.cli.runJson<unknown>(["agent", "view", selector], { timeoutMs: 30000 });
        }
        throw error;
      })
    );
  }

  async compareTasks(left: string, right: string): Promise<unknown> {
    return this.cli.runJson<unknown>(["agent", "compare", left, right], { timeoutMs: 30000 });
  }

  async showConflict(conflictId: string, limit = 50): Promise<unknown> {
    const safeLimit = Math.max(1, Math.min(200, Math.floor(limit)));
    return this.getDaemonJson(`/v1/conflicts/${pathSegment(conflictId)}?limit=${safeLimit}`, () =>
      this.cli.runJson<unknown>(["conflicts", "show", conflictId, "--limit", String(safeLimit)], { timeoutMs: 30000 })
    );
  }

  async fileChanges(selector: string, path: string): Promise<unknown> {
    return this.cli.runJson<unknown>(["agent", "file", selector, path, "--patch"], { timeoutMs: 30000 });
  }

  async why(selector: string, path: string): Promise<unknown> {
    return this.cli.runJson<unknown>(["agent", "why", selector, path], { timeoutMs: 30000 });
  }

  async lineWhy(pathLine: string): Promise<unknown> {
    return this.cli.runJson<unknown>(["why", pathLine], { timeoutMs: 30000 });
  }

  async history(path: string): Promise<unknown> {
    return this.cli.runJson<unknown>(["history", path], { timeoutMs: 30000 });
  }

  async dryRunApply(selector: string): Promise<unknown> {
    return this.cli.runJson<unknown>(["agent", "apply", selector, "--dry-run"], { timeoutMs: 30000 });
  }

  async queueMerge(selector: string, target = "main"): Promise<unknown> {
    return this.postDaemonJson(
      "/v1/merge-queue",
      {
        source: selector,
        target
      },
      () => this.cli.runJson<unknown>(["merge-queue", "add", selector, "--into", target], { timeoutMs: 30000 })
    );
  }

  async listMergeQueue(): Promise<MergeQueueEntry[]> {
    const report = await this.getDaemonJson<unknown>("/v1/merge-queue", () =>
      this.cli.runJson<unknown>(["merge-queue", "list"], { timeoutMs: 30000 })
    );
    return normalizeMergeQueueList(report);
  }

  async explainMergeQueue(selector: string): Promise<unknown> {
    return this.getDaemonJson(`/v1/merge-queue/${pathSegment(selector)}/explain`, () =>
      this.cli.runJson<unknown>(["merge-queue", "explain", selector], { timeoutMs: 30000 })
    );
  }

  async runMergeQueue(limit?: number | undefined): Promise<unknown> {
    return this.postDaemonJson(
      "/v1/merge-queue/run",
      limit === undefined ? {} : { limit },
      () =>
        this.cli.runJson<unknown>(
          ["merge-queue", "run", ...(limit === undefined ? [] : ["--limit", String(limit)])],
          { timeoutMs: 30000 }
        ),
      { timeoutMs: 120000 }
    );
  }

  async removeMergeQueue(selector: string): Promise<unknown> {
    return this.deleteDaemonJson(`/v1/merge-queue/${pathSegment(selector)}`, () =>
      this.cli.runJson<unknown>(["merge-queue", "remove", selector], { timeoutMs: 30000 })
    );
  }

  async rewind(
    selector: string,
    target: string,
    options: { recordCurrent?: boolean; syncWorkdir?: boolean } = {}
  ): Promise<unknown> {
    const recordCurrent = options.recordCurrent ?? false;
    const syncWorkdir = options.syncWorkdir ?? true;
    return this.postDaemonJson(
      `/v1/lanes/${pathSegment(selector)}/rewind`,
      {
        to: target,
        record_current: recordCurrent,
        sync_workdir: syncWorkdir
      },
      () =>
        recordCurrent
          ? this.cli.runJson<unknown>(
              [
                "lane",
                "rewind",
                selector,
                "--to",
                target,
                "--record-current",
                ...(syncWorkdir ? ["--sync-workdir"] : [])
              ],
              { timeoutMs: 30000 }
            )
          : this.cli.runJson<unknown>(["agent", "rewind", selector, "--to", target], { timeoutMs: 30000 })
    );
  }

  async preserveAndRewind(selector: string, target = "before-last-turn"): Promise<unknown> {
    return this.cli.runJson<unknown>(["agent", "rewind", selector, "--to", target], { timeoutMs: 30000 });
  }

  async removeTask(selector: string, force = false): Promise<unknown> {
    const route = `/v1/lanes/${pathSegment(selector)}${force ? "?force=true" : ""}`;
    return this.deleteDaemonJson(route, () =>
      this.cli.runJson<unknown>(["lane", "rm", selector, ...(force ? ["--force"] : [])], { timeoutMs: 30000 })
    );
  }

  async runLaneTest(selector: string, request: LaneGateRequest): Promise<unknown> {
    return this.runLaneGate("tests", selector, request);
  }

  async runLaneEval(selector: string, request: LaneGateRequest): Promise<unknown> {
    return this.runLaneGate("evals", selector, request);
  }

  async laneWorkdir(selector: string): Promise<string | undefined> {
    const report = await this.getDaemonJson(`/v1/lanes/${pathSegment(selector)}/workdir`, () =>
      this.cli.runJson<unknown>(["lane", "workdir", selector], { timeoutMs: 30000 })
    );
    return workdirFromReport(report);
  }

  async doctor(provider: string): Promise<unknown> {
    return this.cli.runJson<unknown>(["agent", "doctor", "--provider", provider], { timeoutMs: 30000 });
  }

  async startDaemon(): Promise<void> {
    this.cli.spawnDetached(["daemon"]);
  }

  async health(): Promise<unknown> {
    const endpoint = await this.getDaemonEndpoint();
    if (!endpoint) {
      return { ok: false, reason: "daemon endpoint not discovered" };
    }
    return this.daemon.getJson(endpoint, "/v1/health");
  }

  crabdbCli(): CrabDbCli {
    return this.cli;
  }

  private async getDaemonJson<T>(route: string, fallback: () => Promise<T>): Promise<T> {
    const endpoint = await this.getDaemonEndpoint();
    if (!endpoint) {
      return fallback();
    }
    try {
      return await this.daemon.getJson<T>(endpoint, route);
    } catch {
      this.daemonEndpoint = null;
      return fallback();
    }
  }

  private async postDaemonJson<T>(
    route: string,
    body: unknown,
    fallback: () => Promise<T>,
    options: { timeoutMs?: number } = {}
  ): Promise<T> {
    const endpoint = await this.getDaemonEndpoint();
    if (!endpoint) {
      return fallback();
    }
    try {
      return await this.daemon.postJson<T>(endpoint, route, body, options);
    } catch {
      this.daemonEndpoint = null;
      return fallback();
    }
  }

  private async deleteDaemonJson<T>(route: string, fallback: () => Promise<T>): Promise<T> {
    const endpoint = await this.getDaemonEndpoint();
    if (!endpoint) {
      return fallback();
    }
    try {
      return await this.daemon.deleteJson<T>(endpoint, route);
    } catch {
      this.daemonEndpoint = null;
      return fallback();
    }
  }

  private async tryDaemonJson<T = unknown>(route: string): Promise<T | undefined> {
    const endpoint = await this.getDaemonEndpoint();
    if (!endpoint) {
      return undefined;
    }
    try {
      return await this.daemon.getJson<T>(endpoint, route);
    } catch {
      this.daemonEndpoint = null;
      return undefined;
    }
  }

  private async getDaemonEndpoint(): Promise<DaemonEndpoint | undefined> {
    if (this.daemonEndpoint !== undefined) {
      return this.daemonEndpoint ?? undefined;
    }
    this.daemonEndpoint = (await this.daemon.discover()) ?? null;
    return this.daemonEndpoint ?? undefined;
  }

  private async runLaneGate(kind: "tests" | "evals", selector: string, request: LaneGateRequest): Promise<unknown> {
    const timeoutSecs = request.timeoutSecs ?? 600;
    const timeoutMs = (timeoutSecs + 30) * 1000;
    const body = {
      command: request.command,
      timeout_secs: timeoutSecs,
      ...(request.suite ? { suite: request.suite } : {})
    };
    const cliKind = kind === "tests" ? "test" : "eval";
    const fallbackArgs = [
      "lane",
      cliKind,
      "--timeout-secs",
      String(timeoutSecs),
      ...(request.suite ? ["--suite", request.suite] : []),
      selector,
      "--",
      ...request.command
    ];
    return this.postDaemonJson(
      `/v1/lanes/${pathSegment(selector)}/${kind}`,
      body,
      () => this.cli.runJson<unknown>(fallbackArgs, { timeoutMs }),
      { timeoutMs }
    );
  }

  private async enrichTasksWithReadiness(tasks: AgentTask[]): Promise<AgentTask[]> {
    const endpoint = await this.getDaemonEndpoint();
    if (!endpoint || !tasks.length) {
      return tasks;
    }
    const enriched = await mapWithConcurrency(tasks, 8, async (task) => {
        try {
          const readiness = await this.daemon.getJson<unknown>(
            endpoint,
            `/v1/lanes/${pathSegment(task.lane)}/readiness`
          );
          return {
            ...task,
            coordination: coordinationSummaryFromSources(task.raw, readiness),
            raw: {
              task: task.raw,
              readiness
            }
          };
        } catch {
          return task;
        }
      });
    return enriched;
  }
}

function normalizeTaskList(report: unknown): AgentTask[] {
  if (Array.isArray(report)) {
    return report.map((item, index) => normalizeTask(item, `task-${index}`));
  }
  const root = asRecord(report);
  const tasks = Array.isArray(root.tasks) ? root.tasks : Array.isArray(root.items) ? root.items : [];
  return tasks.map((item, index) => normalizeTask(item, `task-${index}`));
}

function normalizeTaskView(report: unknown, selector: string): TaskView {
  const root = asRecord(report);
  const task = normalizeTask(root.task ?? root, selector);
  const transcript = asRecord(root.transcript);
  const acpSession = asRecord(transcript.acp_session);
  const turns = arrayField(transcript, "turns");
  const turnMessages = turns.flatMap((turn) => arrayField(asRecord(turn), "messages"));
  const turnEvents = turns.flatMap((turn) => arrayField(asRecord(turn), "events"));
  const transcriptMessages = arrayField(transcript, "messages");
  const transcriptEvents = arrayField(transcript, "events");
  const rootMessages = transcriptMessages.length ? transcriptMessages : arrayField(root, "messages");
  const rootEvents = transcriptEvents.length ? transcriptEvents : arrayField(root, "events");
  const review = asRecord(root.review);
  const readiness = asRecord(root.readiness);
  return {
    task: {
      ...task,
      sessionId: task.sessionId || stringField(acpSession, "crabdb_session_id"),
      acpSessionId: task.acpSessionId || stringField(acpSession, "acp_session_id"),
      workdir: task.workdir || stringField(acpSession, "cwd")
    },
    turns,
    messages: turnMessages.length ? turnMessages : rootMessages,
    events: turnEvents.length ? turnEvents : rootEvents,
    changes: arrayField(root, "changes").concat(arrayField(root, "changed_paths")).concat(arrayField(review, "changed_paths")),
    review: Object.keys(review).length ? review : undefined,
    readiness: Object.keys(readiness).length ? readiness : undefined,
    raw: report
  };
}

function normalizeTask(value: unknown, fallback: string): AgentTask {
  const task = asRecord(value);
  const record = asRecord(task.record);
  const branch = asRecord(task.branch);
  const status = asRecord(task.status);
  const lane =
    stringField(task, "lane") ||
    stringField(task, "lane_name") ||
    stringField(record, "name") ||
    stringField(branch, "lane") ||
    fallback;
  const changedPaths = arrayField(task, "changed_paths")
    .concat(arrayField(task, "changedPaths"))
    .concat(arrayField(status, "changed_paths"))
    .map((item) => {
      const record = asRecord(item);
      return stringField(record, "path") || String(item);
    });
  return {
    id: stringField(task, "task_id") || stringField(task, "id") || stringField(record, "lane_id") || lane,
    lane,
    title: stringField(task, "title") || stringField(task, "name") || stringField(record, "title") || lane,
    status: stringField(task, "status") || stringField(status, "status") || "active",
    provider: stringField(task, "provider") || stringField(record, "provider"),
    model: stringField(task, "model") || stringField(record, "model"),
    sessionId: stringField(task, "session_id") || stringField(task, "sessionId") || stringField(branch, "session_id"),
    acpSessionId: stringField(task, "acp_session_id") || stringField(task, "acpSessionId"),
    workdir: stringField(task, "workdir") || stringField(task, "work_dir") || stringField(task, "workdir_path"),
    latestCheckpoint:
      stringField(task, "latest_checkpoint") ||
      stringField(task, "latestCheckpoint") ||
      stringField(branch, "head_change") ||
      stringField(branch, "head"),
    changedPaths,
    updatedAt: stringField(task, "updated_at") || stringField(task, "updatedAt"),
    nextAction: stringField(task, "next_action") || stringField(task, "nextAction"),
    raw: value
  };
}

function arrayField(record: Record<string, unknown>, key: string): unknown[] {
  const value = record[key];
  return Array.isArray(value) ? value : [];
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function pathSegment(value: string): string {
  return encodeURIComponent(value);
}

function isUnsupportedAgentSubcommand(error: unknown, command: string): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes(`unrecognized subcommand '${command}'`) || message.includes(`unrecognized subcommand "${command}"`);
}

function workdirFromReport(value: unknown): string | undefined {
  const record = asRecord(value);
  const task = asRecord(record.task);
  return stringField(record, "workdir") || stringField(task, "workdir");
}

async function mapWithConcurrency<T, R>(
  values: T[],
  concurrency: number,
  mapper: (value: T) => Promise<R>
): Promise<R[]> {
  const results = new Array<R>(values.length);
  let next = 0;
  const workers = Array.from({ length: Math.min(concurrency, values.length) }, async () => {
    while (next < values.length) {
      const index = next;
      next += 1;
      results[index] = await mapper(values[index] as T);
    }
  });
  await Promise.all(workers);
  return results;
}

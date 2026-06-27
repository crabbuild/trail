import * as vscode from "vscode";
import type { AgentTask, AgentTaskStatus, MergeQueueEntry, TaskRepository } from "../crabdb/TaskRepository";

type TreeEntry = GroupEntry | QueueGroupEntry | TaskEntry | QueueItemEntry | EmptyEntry;

interface GroupEntry {
  type: "group";
  id: string;
  label: string;
  tasks: AgentTask[];
}

interface QueueGroupEntry {
  type: "queueGroup";
  id: string;
  label: string;
  entries: MergeQueueEntry[];
}

interface TaskEntry {
  type: "task";
  task: AgentTask;
}

interface QueueItemEntry {
  type: "queueItem";
  entry: MergeQueueEntry;
}

interface EmptyEntry {
  type: "empty";
  label: string;
}

export type TaskTreeMode = "tasks" | "reviews" | "queue";

export class TasksTreeProvider implements vscode.TreeDataProvider<TreeEntry> {
  private readonly changed = new vscode.EventEmitter<TreeEntry | undefined | null | void>();
  readonly onDidChangeTreeData = this.changed.event;
  private tasks: AgentTask[] = [];
  private queueEntries: MergeQueueEntry[] = [];
  private error: string | undefined;

  constructor(
    private readonly mode: TaskTreeMode,
    private readonly repository: TaskRepository
  ) {}

  async refresh(): Promise<void> {
    try {
      this.error = undefined;
      if (this.mode === "queue") {
        this.queueEntries = await this.repository.listMergeQueue();
        this.tasks = [];
      } else {
        this.tasks = await this.repository.listTasks();
        this.queueEntries = [];
      }
    } catch (error) {
      this.error = error instanceof Error ? error.message : String(error);
      this.tasks = [];
      this.queueEntries = [];
    }
    this.changed.fire();
  }

  getTreeItem(element: TreeEntry): vscode.TreeItem {
    if (element.type === "empty") {
      const item = new vscode.TreeItem(element.label, vscode.TreeItemCollapsibleState.None);
      item.iconPath = new vscode.ThemeIcon("info");
      return item;
    }

    if (element.type === "group") {
      const item = new vscode.TreeItem(
        `${element.label} (${element.tasks.length})`,
        vscode.TreeItemCollapsibleState.Expanded
      );
      item.iconPath = new vscode.ThemeIcon(groupIcon(element.id));
      return item;
    }

    if (element.type === "queueGroup") {
      const item = new vscode.TreeItem(
        `${element.label} (${element.entries.length})`,
        vscode.TreeItemCollapsibleState.Expanded
      );
      item.iconPath = new vscode.ThemeIcon(queueGroupIcon(element.id));
      return item;
    }

    if (element.type === "queueItem") {
      const entry = element.entry;
      const item = new vscode.TreeItem(entry.sourceRef, vscode.TreeItemCollapsibleState.None);
      item.id = entry.id;
      item.description = `-> ${entry.targetRef} priority ${entry.priority}`;
      item.tooltip = [
        `Queue: ${entry.id}`,
        `Source: ${entry.sourceRef}`,
        `Target: ${entry.targetRef}`,
        `Status: ${entry.status}`,
        `Priority: ${entry.priority}`,
        entry.createdAt !== undefined ? `Created: ${formatQueueTime(entry.createdAt)}` : undefined,
        entry.updatedAt !== undefined ? `Updated: ${formatQueueTime(entry.updatedAt)}` : undefined
      ]
        .filter(Boolean)
        .join("\n");
      item.contextValue = "crabdbQueueEntry";
      item.iconPath = new vscode.ThemeIcon(queueStatusIcon(entry.status));
      item.command = {
        command: "crabdb.explainQueueEntry",
        title: "Explain Merge Queue Entry",
        arguments: [entry]
      };
      return item;
    }

    const task = element.task;
    const item = new vscode.TreeItem(task.title, vscode.TreeItemCollapsibleState.None);
    const coordination = task.coordination;
    const coordinationLabels = coordination?.labels.slice(0, 3) ?? [];
    item.id = task.id;
    item.description = [task.provider, task.lane, ...coordinationLabels].filter(Boolean).join(" ");
    item.tooltip = [
      task.title,
      `Lane: ${task.lane}`,
      `Status: ${task.status}`,
      task.changedPaths.length ? `Changed paths: ${task.changedPaths.length}` : undefined,
      coordination?.labels.length ? `Coordination: ${coordination.labels.join(", ")}` : undefined,
      ...(coordination?.issues.slice(0, 5).map((issue) => `${issue.tone}: ${issue.message}`) ?? []),
      task.nextAction
    ]
      .filter(Boolean)
      .join("\n");
    item.contextValue = "crabdbTask";
    item.iconPath = new vscode.ThemeIcon(statusIcon(task.status, coordination?.severity));
    item.command = {
      command: "crabdb.openAgentChat",
      title: "Open Agent Chat",
      arguments: [task]
    };
    return item;
  }

  getChildren(element?: TreeEntry): TreeEntry[] {
    if (element?.type === "group") {
      return element.tasks.map((task) => ({ type: "task", task }));
    }

    if (element?.type === "queueGroup") {
      return element.entries.map((entry) => ({ type: "queueItem", entry }));
    }

    if (element) {
      return [];
    }

    if (this.error) {
      return [{ type: "empty", label: this.error }];
    }

    const groups = this.mode === "queue" ? this.queueGroupsForMode() : this.groupsForMode();
    if (groups.length === 0) {
      return [{ type: "empty", label: emptyLabel(this.mode) }];
    }
    return groups;
  }

  private groupsForMode(): GroupEntry[] {
    const filtered = this.tasks.filter((task) => {
      if (this.mode === "reviews") {
        return ["ready", "dirty", "blocked", "conflicted"].includes(normalizeStatus(task.status));
      }
      if (this.mode === "queue") {
        return ["ready", "conflicted", "blocked"].includes(normalizeStatus(task.status));
      }
      return normalizeStatus(task.status) !== "empty";
    });

    const order: Array<[string, string, (task: AgentTask) => boolean]> = [
      ["blocked", "Waiting for permission", (task) => normalizeStatus(task.status) === "blocked"],
      ["conflicted", "Conflicted", (task) => normalizeStatus(task.status) === "conflicted"],
      ["dirty", "Needs checkpoint", (task) => normalizeStatus(task.status) === "dirty"],
      ["ready", "Ready to review", (task) => normalizeStatus(task.status) === "ready"],
      ["active", "Running", (task) => normalizeStatus(task.status) === "active"],
      ["applied", "Applied", (task) => normalizeStatus(task.status) === "applied"]
    ];

    return order
      .map(([id, label, predicate]) => ({
        type: "group" as const,
        id,
        label,
        tasks: filtered.filter(predicate)
      }))
      .filter((group) => group.tasks.length > 0);
  }

  private queueGroupsForMode(): QueueGroupEntry[] {
    const order: Array<[string, string, (entry: MergeQueueEntry) => boolean]> = [
      ["running", "Running", (entry) => normalizeStatus(entry.status) === "running"],
      ["queued", "Queued", (entry) => normalizeStatus(entry.status) === "queued"],
      ["conflicted", "Conflicted", (entry) => normalizeStatus(entry.status) === "conflicted"],
      ["failed", "Failed", (entry) => normalizeStatus(entry.status) === "failed"],
      ["merged", "Merged", (entry) => normalizeStatus(entry.status) === "merged"],
      ["cancelled", "Cancelled", (entry) => normalizeStatus(entry.status) === "cancelled"]
    ];
    const known = new Set(order.map(([id]) => id));
    const groups = order
      .map(([id, label, predicate]) => ({
        type: "queueGroup" as const,
        id,
        label,
        entries: this.queueEntries.filter(predicate)
      }))
      .filter((group) => group.entries.length > 0);
    const other = this.queueEntries.filter((entry) => !known.has(normalizeStatus(entry.status)));
    if (other.length) {
      groups.push({
        type: "queueGroup",
        id: "other",
        label: "Other",
        entries: other
      });
    }
    return groups;
  }
}

function normalizeStatus(status: AgentTaskStatus): string {
  return String(status).toLowerCase();
}

function statusIcon(status: AgentTaskStatus, coordinationSeverity: string = "ok"): string {
  if (coordinationSeverity === "blocked") {
    return "warning";
  }
  if (coordinationSeverity === "warning") {
    return "issues";
  }
  switch (normalizeStatus(status)) {
    case "ready":
      return "pass";
    case "dirty":
      return "circle-filled";
    case "blocked":
      return "warning";
    case "conflicted":
      return "error";
    case "applied":
      return "check";
    case "active":
      return "sync~spin";
    default:
      return "circle-outline";
  }
}

function groupIcon(group: string): string {
  switch (group) {
    case "ready":
      return "checklist";
    case "blocked":
      return "lock";
    case "conflicted":
      return "warning";
    case "applied":
      return "verified";
    default:
      return "list-tree";
  }
}

function queueGroupIcon(group: string): string {
  switch (group) {
    case "running":
      return "sync~spin";
    case "queued":
      return "git-merge";
    case "conflicted":
      return "warning";
    case "failed":
      return "error";
    case "merged":
      return "pass";
    case "cancelled":
      return "circle-slash";
    default:
      return "list-tree";
  }
}

function queueStatusIcon(status: string): string {
  switch (normalizeStatus(status)) {
    case "running":
      return "sync~spin";
    case "queued":
      return "git-merge";
    case "conflicted":
      return "warning";
    case "failed":
      return "error";
    case "merged":
      return "pass";
    case "cancelled":
      return "circle-slash";
    default:
      return "circle-outline";
  }
}

function formatQueueTime(value: number | string): string {
  if (typeof value === "number") {
    return new Date(value * 1000).toLocaleString();
  }
  return value;
}

function emptyLabel(mode: TaskTreeMode): string {
  if (mode === "reviews") {
    return "No agent tasks need review.";
  }
  if (mode === "queue") {
    return "No merge queue entries.";
  }
  return "No agent tasks yet.";
}

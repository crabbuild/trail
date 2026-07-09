import * as vscode from "vscode";
import type { AgentTask, MergeQueueEntry, TaskRepository } from "../trail/TaskRepository";
import {
  buildEmptyTreePresentation,
  buildGroupTreePresentation,
  buildQueueItemTreePresentation,
  buildTaskTreePresentation,
  normalizeTreeStatus,
  taskTreeGroupStatus,
  type TaskTreeMode,
  type TreeIconPresentation,
  type TreeItemPresentation
} from "./taskTreeModel";

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
  mode: TaskTreeMode;
  error?: string | undefined;
}

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
      return treeItemFromPresentation(buildEmptyTreePresentation(element.mode, element.error));
    }

    if (element.type === "group") {
      const item = treeItemFromPresentation(
        buildGroupTreePresentation({ id: element.id, label: element.label, count: element.tasks.length, kind: "task" }),
        vscode.TreeItemCollapsibleState.Expanded
      );
      return item;
    }

    if (element.type === "queueGroup") {
      const item = treeItemFromPresentation(
        buildGroupTreePresentation({ id: element.id, label: element.label, count: element.entries.length, kind: "queue" }),
        vscode.TreeItemCollapsibleState.Expanded
      );
      return item;
    }

    if (element.type === "queueItem") {
      const entry = element.entry;
      const item = treeItemFromPresentation(buildQueueItemTreePresentation(entry));
      item.id = entry.id;
      item.contextValue = "trailQueueEntry";
      item.command = {
        command: "trail.explainQueueEntry",
        title: "Explain Merge Queue Entry",
        arguments: [entry]
      };
      return item;
    }

    const task = element.task;
    const item = treeItemFromPresentation(buildTaskTreePresentation(task, this.mode === "reviews" ? "reviews" : "tasks"));
    item.id = task.id;
    item.contextValue = "trailTask";
    item.command = {
      command: "trail.openAgentChat",
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
      return [{ type: "empty", mode: this.mode, error: this.error }];
    }

    const groups = this.mode === "queue" ? this.queueGroupsForMode() : this.groupsForMode();
    if (groups.length === 0) {
      return [{ type: "empty", mode: this.mode }];
    }
    return groups;
  }

  private groupsForMode(): GroupEntry[] {
    const filtered = this.tasks.filter((task) => {
      const groupStatus = taskTreeGroupStatus(task);
      if (this.mode === "reviews") {
        return ["ready", "dirty", "blocked", "conflicted", "attention"].includes(groupStatus);
      }
      if (this.mode === "queue") {
        return ["ready", "conflicted", "blocked"].includes(groupStatus);
      }
      return groupStatus !== "empty";
    });

    const order: Array<[string, string, (task: AgentTask) => boolean]> = [
      ["blocked", "Blocked", (task) => taskTreeGroupStatus(task) === "blocked"],
      ["conflicted", "Conflicted", (task) => taskTreeGroupStatus(task) === "conflicted"],
      ["attention", "Needs attention", (task) => taskTreeGroupStatus(task) === "attention"],
      ["dirty", "Needs checkpoint", (task) => taskTreeGroupStatus(task) === "dirty"],
      ["ready", "Ready to review", (task) => taskTreeGroupStatus(task) === "ready"],
      ["active", "Running", (task) => taskTreeGroupStatus(task) === "active"],
      ["applied", "Applied", (task) => taskTreeGroupStatus(task) === "applied"]
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
      ["running", "Running", (entry) => normalizeTreeStatus(entry.status) === "running"],
      ["queued", "Queued", (entry) => normalizeTreeStatus(entry.status) === "queued"],
      ["conflicted", "Conflicted", (entry) => normalizeTreeStatus(entry.status) === "conflicted"],
      ["failed", "Failed", (entry) => normalizeTreeStatus(entry.status) === "failed"],
      ["merged", "Merged", (entry) => normalizeTreeStatus(entry.status) === "merged"],
      ["cancelled", "Cancelled", (entry) => normalizeTreeStatus(entry.status) === "cancelled"]
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
    const other = this.queueEntries.filter((entry) => !known.has(normalizeTreeStatus(entry.status)));
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

function treeItemFromPresentation(
  presentation: TreeItemPresentation,
  collapsibleState = vscode.TreeItemCollapsibleState.None
): vscode.TreeItem {
  const item = new vscode.TreeItem(presentation.label, collapsibleState);
  const description = presentation.description;
  if (description !== undefined) {
    item.description = description;
  }
  item.tooltip = presentation.tooltip;
  item.iconPath = themeIcon(presentation.icon);
  item.accessibilityInformation = {
    label: presentation.accessibilityLabel
  };
  if (presentation.command) {
    item.command = {
      command: presentation.command.command,
      title: presentation.command.title
    };
  }
  return item;
}

function themeIcon(icon: TreeIconPresentation): vscode.ThemeIcon {
  return icon.color ? new vscode.ThemeIcon(icon.id, new vscode.ThemeColor(icon.color)) : new vscode.ThemeIcon(icon.id);
}

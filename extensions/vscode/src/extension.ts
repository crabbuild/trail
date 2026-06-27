import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";
import { getExtensionConfig, getWorkspaceRoot, requireWorkspaceRoot } from "./config";
import { ProviderRegistry, type AcpProviderProfile } from "./acp/ProviderRegistry";
import { TaskRepository, type AgentTask, type MergeQueueEntry } from "./crabdb/TaskRepository";
import type { PromptAttachment } from "./model/PromptAttachment";
import { attachmentFromSelectionOrFile } from "./model/VsCodePromptAttachments";
import { ChatPanel } from "./views/ChatPanel";
import { DiffContentProvider } from "./views/DiffContentProvider";
import { laneGateLabel, promptLaneGateRequest, type LaneGateKind } from "./views/LaneGatePrompts";
import { SettingsPanel } from "./views/SettingsPanel";
import { TasksTreeProvider } from "./views/TasksTreeProvider";
import { redactedJson } from "./shared/securityRedaction";

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel("CrabDB Agents");
  context.subscriptions.push(output);

  const root = getWorkspaceRoot();
  await updateWorkspaceContext(root);
  if (!root) {
    output.appendLine("CrabDB Agents activated without an open workspace.");
  }

  const repository = root ? new TaskRepository(root, output) : undefined;
  const providers = root ? new ProviderRegistry(root) : undefined;
  const diffProvider = new DiffContentProvider();
  const tasks = repository ? new TasksTreeProvider("tasks", repository) : undefined;
  const reviews = repository ? new TasksTreeProvider("reviews", repository) : undefined;
  const queue = repository ? new TasksTreeProvider("queue", repository) : undefined;

  if (tasks && reviews && queue) {
    context.subscriptions.push(
      vscode.workspace.registerTextDocumentContentProvider(DiffContentProvider.scheme, diffProvider),
      vscode.window.registerTreeDataProvider("crabdb.tasks", tasks),
      vscode.window.registerTreeDataProvider("crabdb.reviews", reviews),
      vscode.window.registerTreeDataProvider("crabdb.queue", queue)
    );
    void refreshAll(tasks, reviews, queue);
  }

  context.subscriptions.push(
    command("crabdb.initWorkspace", async () => {
      const workspaceRoot = requireWorkspaceRoot();
      const repo = repository ?? new TaskRepository(workspaceRoot, output);
      await repo.initWorkspace();
      await updateWorkspaceContext(workspaceRoot);
      vscode.window.showInformationMessage("Initialized CrabDB workspace.");
      if (tasks && reviews && queue) {
        await refreshAll(tasks, reviews, queue);
      }
    }),
    command("crabdb.refreshTasks", async () => {
      await updateWorkspaceContext(root);
      if (tasks && reviews && queue) {
        await refreshAll(tasks, reviews, queue);
      }
    }),
    command("crabdb.newAgentTask", async () => {
      const provider = await requireProviders(providers).pickProfile();
      if (!provider) {
        return;
      }
      await openChat(context.extensionUri, requireRepository(repository), output, diffProvider, provider);
    }),
    command("crabdb.openAgentChat", async (task?: AgentTask) => {
      await openChat(
        context.extensionUri,
        requireRepository(repository),
        output,
        diffProvider,
        requireProviders(providers).defaultProfile(),
        task
      );
    }),
    command("crabdb.openLatestReview", async (task?: AgentTask) => {
      const repo = requireRepository(repository);
      const selected = task ?? (await repo.latestTask());
      if (!selected) {
        vscode.window.showInformationMessage("No CrabDB agent task is available.");
        return;
      }
      await openChat(context.extensionUri, repo, output, diffProvider, requireProviders(providers).defaultProfile(), selected);
    }),
    command("crabdb.applyLatestDryRun", async () => {
      const repo = requireRepository(repository);
      const task = await repo.latestTask();
      if (!task) {
        vscode.window.showInformationMessage("No CrabDB agent task is available.");
        return;
      }
      const result = await repo.dryRunApply(task.lane);
      output.appendLine(redactedJson(result));
      vscode.window.showInformationMessage(`Dry-run apply finished for ${task.lane}.`);
    }),
    command("crabdb.queueMerge", async (task?: AgentTask) => {
      const repo = requireRepository(repository);
      const selected = task ?? (await repo.latestTask());
      if (!selected) {
        vscode.window.showInformationMessage("No CrabDB agent task is available.");
        return;
      }
      const target = await vscode.window.showInputBox({
        prompt: "Queue merge target branch",
        value: "main"
      });
      if (!target) {
        return;
      }
      const result = await repo.queueMerge(selected.lane, target);
      showJsonResult(output, `Queued ${selected.lane} into ${target}`, result);
      if (tasks && reviews && queue) {
        await refreshAll(tasks, reviews, queue);
      }
    }),
    command("crabdb.explainQueueEntry", async (entry?: MergeQueueEntry) => {
      const repo = requireRepository(repository);
      const selected = entry ?? (await pickQueueEntry(repo, "Choose merge queue entry to explain"));
      if (!selected) {
        return;
      }
      const result = await repo.explainMergeQueue(selected.id);
      showJsonResult(output, `Merge queue explanation for ${selected.sourceRef}`, result);
    }),
    command("crabdb.runMergeQueue", async () => {
      const repo = requireRepository(repository);
      const limitValue = await vscode.window.showInputBox({
        prompt: "Maximum queue entries to run",
        placeHolder: "Leave empty to run all queued entries",
        validateInput: (value) => validateOptionalPositiveInteger(value)
      });
      if (limitValue === undefined) {
        return;
      }
      const limit = limitValue.trim() ? Number(limitValue.trim()) : undefined;
      const result = await repo.runMergeQueue(limit);
      showJsonResult(output, "Merge queue run finished", result);
      if (tasks && reviews && queue) {
        await refreshAll(tasks, reviews, queue);
      }
    }),
    command("crabdb.removeQueueEntry", async (entry?: MergeQueueEntry) => {
      const repo = requireRepository(repository);
      const selected = entry ?? (await pickQueueEntry(repo, "Choose merge queue entry to remove"));
      if (!selected) {
        return;
      }
      const confirmed = await vscode.window.showWarningMessage(
        `Remove merge queue entry ${selected.id}?`,
        {
          modal: true,
          detail: `${selected.sourceRef} -> ${selected.targetRef}`
        },
        "Remove queue entry"
      );
      if (confirmed !== "Remove queue entry") {
        return;
      }
      const result = await repo.removeMergeQueue(selected.id);
      showJsonResult(output, `Removed merge queue entry ${selected.id}`, result);
      if (tasks && reviews && queue) {
        await refreshAll(tasks, reviews, queue);
      }
    }),
    command("crabdb.rewindTask", async (task?: AgentTask) => {
      const repo = requireRepository(repository);
      const selected = task ?? (await repo.latestTask());
      if (!selected) {
        vscode.window.showInformationMessage("No CrabDB agent task is available.");
        return;
      }
      const target = await vscode.window.showInputBox({
        prompt: "Rewind target",
        value: "before-last-turn",
        placeHolder: "before-last-turn, turn:2, or checkpoint id"
      });
      if (!target) {
        return;
      }
      await repo.rewind(selected.lane, target);
      vscode.window.showInformationMessage(`Rewound ${selected.lane} to ${target}.`);
      if (tasks && reviews && queue) {
        await refreshAll(tasks, reviews, queue);
      }
    }),
    command("crabdb.preserveFailedAttempt", async (task?: AgentTask) => {
      const repo = requireRepository(repository);
      const selected = task ?? (await repo.latestTask());
      if (!selected) {
        vscode.window.showInformationMessage("No CrabDB agent task is available.");
        return;
      }
      const target = await vscode.window.showInputBox({
        prompt: "Rewind target after preserving the current attempt",
        value: "before-last-turn",
        placeHolder: "before-last-turn, before-turn:2, or checkpoint id"
      });
      if (!target) {
        return;
      }
      const result = await repo.preserveAndRewind(selected.lane, target);
      showJsonResult(output, `Preserved and rewound ${selected.lane} to ${target}`, result);
      if (tasks && reviews && queue) {
        await refreshAll(tasks, reviews, queue);
      }
    }),
    command("crabdb.removeAgentTask", async (task?: AgentTask) => {
      const repo = requireRepository(repository);
      const selected = task ?? (await repo.latestTask());
      if (!selected) {
        vscode.window.showInformationMessage("No CrabDB agent task is available.");
        return;
      }
      const confirmed = await vscode.window.showWarningMessage(
        `Remove CrabDB agent task ${selected.title}?`,
        {
          modal: true,
          detail: `This removes lane ${selected.lane} and its materialized workdir. CrabDB keeps the historical lane record marked as removed.`
        },
        "Remove task"
      );
      if (confirmed !== "Remove task") {
        return;
      }
      const result = await repo.removeTask(selected.lane, true);
      showJsonResult(output, `Removed ${selected.lane}`, result);
      if (tasks && reviews && queue) {
        await refreshAll(tasks, reviews, queue);
      }
    }),
    command("crabdb.runLaneTest", async (task?: AgentTask) => {
      await runLaneGateCommand("test", task, requireRepository(repository), output, tasks, reviews, queue);
    }),
    command("crabdb.runLaneEval", async (task?: AgentTask) => {
      await runLaneGateCommand("eval", task, requireRepository(repository), output, tasks, reviews, queue);
    }),
    command("crabdb.openLaneWorkdir", async (task?: AgentTask) => {
      const repo = requireRepository(repository);
      const selected = task ?? (await repo.latestTask());
      if (!selected) {
        vscode.window.showInformationMessage("No CrabDB agent task is available.");
        return;
      }
      await openLaneWorkdir(repo, selected.lane);
    }),
    command("crabdb.compareTasks", async () => {
      const repo = requireRepository(repository);
      const allTasks = await repo.listTasks();
      if (allTasks.length < 2) {
        vscode.window.showInformationMessage("At least two CrabDB agent tasks are required to compare.");
        return;
      }
      const left = await pickTask(allTasks, "Choose left agent task");
      if (!left) {
        return;
      }
      const right = await pickTask(
        allTasks.filter((candidate) => candidate.lane !== left.lane),
        "Choose right agent task"
      );
      if (!right) {
        return;
      }
      const result = await repo.compareTasks(left.lane, right.lane);
      showJsonResult(output, `Compared ${left.lane} and ${right.lane}`, result);
    }),
    command("crabdb.startDaemon", async () => {
      await requireRepository(repository).startDaemon();
      vscode.window.showInformationMessage("Started crabdb daemon.");
    }),
    command("crabdb.doctor", async () => {
      const provider = getExtensionConfig().defaultProvider;
      const result = await requireRepository(repository).doctor(provider);
      output.appendLine(redactedJson(result));
      vscode.window.showInformationMessage(`CrabDB doctor finished for ${provider}.`);
    }),
    command("crabdb.openSettings", async () => {
      SettingsPanel.open(context.extensionUri, requireWorkspaceRoot());
    }),
    command("crabdb.addAcpProvider", async () => {
      await vscode.commands.executeCommand("workbench.action.openSettings", "crabdb.customProviders");
    }),
    command("crabdb.askSelection", async () => {
      const attachment = attachmentFromSelectionOrFile();
      await openChat(
        context.extensionUri,
        requireRepository(repository),
        output,
        diffProvider,
        requireProviders(providers).defaultProfile(),
        undefined,
        attachment ? [attachment] : []
      );
      vscode.window.showInformationMessage(
        attachment ? `Attached ${attachment.label} to the CrabDB agent prompt.` : "Open a file and select text to attach it."
      );
    }),
    command("crabdb.attachSelection", async () => {
      const attachment = attachmentFromSelectionOrFile();
      await openChat(
        context.extensionUri,
        requireRepository(repository),
        output,
        diffProvider,
        requireProviders(providers).defaultProfile(),
        undefined,
        attachment ? [attachment] : []
      );
    }),
    command("crabdb.showLineHistory", async () => {
      const repo = requireRepository(repository);
      const pathLine = activePathLine();
      if (!pathLine) {
        vscode.window.showInformationMessage("Open a file to inspect line history.");
        return;
      }
      const result = await repo.lineWhy(pathLine);
      showJsonResult(output, `CrabDB line history for ${pathLine}`, result);
    }),
    command("crabdb.showFileChanges", async () => {
      const repo = requireRepository(repository);
      const task = await repo.latestTask();
      const path = activeRelativeFilePath();
      if (!path) {
        vscode.window.showInformationMessage("Open a workspace file to inspect agent changes.");
        return;
      }
      if (!task) {
        const result = await repo.history(path);
        showJsonResult(output, `CrabDB history for ${path}`, result);
        return;
      }
      const result = await repo.fileChanges(task.lane, path);
      showJsonResult(output, `Agent changes for ${path}`, result);
    })
  );
}

export function deactivate(): void {}

async function updateWorkspaceContext(root: string | undefined): Promise<void> {
  await vscode.commands.executeCommand("setContext", "crabdb.workspaceOpen", Boolean(root));
  await vscode.commands.executeCommand("setContext", "crabdb.initialized", Boolean(root && fs.existsSync(path.join(root, ".crabdb"))));
}

async function openChat(
  extensionUri: vscode.Uri,
  repository: TaskRepository,
  output: vscode.OutputChannel,
  diffProvider: DiffContentProvider,
  provider: AcpProviderProfile,
  task?: AgentTask,
  attachments: PromptAttachment[] = []
): Promise<void> {
  await ChatPanel.open(extensionUri, repository, output, diffProvider, provider, task, attachments);
}

async function refreshAll(...providers: TasksTreeProvider[]): Promise<void> {
  await Promise.all(providers.map((provider) => provider.refresh()));
}

function command(commandId: string, callback: (...args: any[]) => unknown): vscode.Disposable {
  return vscode.commands.registerCommand(commandId, callback);
}

function requireRepository(repository: TaskRepository | undefined): TaskRepository {
  if (!repository) {
    requireWorkspaceRoot();
    throw new Error("CrabDB repository is unavailable.");
  }
  return repository;
}

function requireProviders(providers: ProviderRegistry | undefined): ProviderRegistry {
  if (!providers) {
    requireWorkspaceRoot();
    throw new Error("CrabDB provider registry is unavailable.");
  }
  return providers;
}

async function pickTask(tasks: AgentTask[], title: string): Promise<AgentTask | undefined> {
  const picked = await vscode.window.showQuickPick(
    tasks.map((task) => ({
      label: task.title,
      description: task.lane,
      detail: `${task.status}${task.changedPaths.length ? ` - ${task.changedPaths.length} changed paths` : ""}`,
      task
    })),
    { title }
  );
  return picked?.task;
}

async function pickQueueEntry(repo: TaskRepository, title: string): Promise<MergeQueueEntry | undefined> {
  const entries = await repo.listMergeQueue();
  if (!entries.length) {
    vscode.window.showInformationMessage("No CrabDB merge queue entries are available.");
    return undefined;
  }
  const picked = await vscode.window.showQuickPick(
    entries.map((entry) => ({
      label: entry.sourceRef,
      description: `-> ${entry.targetRef}`,
      detail: `${entry.status} - priority ${entry.priority} - ${entry.id}`,
      entry
    })),
    { title }
  );
  return picked?.entry;
}

function validateOptionalPositiveInteger(value: string): string | undefined {
  const trimmed = value.trim();
  if (!trimmed) {
    return undefined;
  }
  const number = Number(trimmed);
  return Number.isInteger(number) && number > 0 ? undefined : "Enter a positive whole number or leave empty.";
}

function activeRelativeFilePath(): string | undefined {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.uri.scheme !== "file") {
    return undefined;
  }
  return vscode.workspace.asRelativePath(editor.document.uri, false);
}

function activePathLine(): string | undefined {
  const path = activeRelativeFilePath();
  const editor = vscode.window.activeTextEditor;
  if (!path || !editor) {
    return undefined;
  }
  return `${path}:${editor.selection.active.line + 1}`;
}

function showJsonResult(output: vscode.OutputChannel, title: string, result: unknown): void {
  output.appendLine("");
  output.appendLine(`## ${title}`);
  output.appendLine(redactedJson(result));
  output.show(true);
  vscode.window.showInformationMessage(title);
}

async function runLaneGateCommand(
  kind: LaneGateKind,
  task: AgentTask | undefined,
  repo: TaskRepository,
  output: vscode.OutputChannel,
  ...providers: Array<TasksTreeProvider | undefined>
): Promise<void> {
  const selected = task ?? (await repo.latestTask());
  if (!selected) {
    vscode.window.showInformationMessage("No CrabDB agent task is available.");
    return;
  }
  const request = await promptLaneGateRequest(kind);
  if (!request) {
    return;
  }
  const result =
    kind === "test"
      ? await repo.runLaneTest(selected.lane, request)
      : await repo.runLaneEval(selected.lane, request);
  showJsonResult(output, `Lane ${laneGateLabel(kind)} finished for ${selected.lane}`, result);
  const refreshable = providers.filter((provider): provider is TasksTreeProvider => Boolean(provider));
  if (refreshable.length) {
    await refreshAll(...refreshable);
  }
}

async function openLaneWorkdir(repo: TaskRepository, lane: string): Promise<void> {
  const workdir = await repo.laneWorkdir(lane);
  if (!workdir) {
    vscode.window.showInformationMessage(`Lane ${lane} has no materialized workdir.`);
    return;
  }
  const uri = vscode.Uri.file(workdir);
  await vscode.workspace.fs.stat(uri);
  await vscode.commands.executeCommand("vscode.openFolder", uri, { forceNewWindow: true });
  vscode.window.showInformationMessage(`Opened lane workdir: ${workdir}`);
}

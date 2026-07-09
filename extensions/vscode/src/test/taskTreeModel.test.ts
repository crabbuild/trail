import assert from "node:assert/strict";
import test from "node:test";
import type { AgentTask, MergeQueueEntry } from "../trail/TaskRepository";
import {
  buildEmptyTreePresentation,
  buildGroupTreePresentation,
  buildQueueItemTreePresentation,
  buildTaskTreePresentation,
  normalizeTreeStatus,
  taskTreeGroupStatus
} from "../views/taskTreeModel";

function task(overrides: Partial<AgentTask> = {}): AgentTask {
  return {
    id: "task-1",
    lane: "agent-claude-code-54129f3b1bf8",
    title: "Improve VS Code extension polish",
    status: "ready",
    provider: "Claude Code via Trail",
    model: "claude-sonnet",
    sessionId: "session-1",
    acpSessionId: "acp-1",
    workdir: "/tmp/worktrees/agent-claude-code-54129f3b1bf8",
    latestCheckpoint: "ch_123",
    changedPaths: ["extensions/vscode/src/views/ChatPanel.ts"],
    updatedAt: "2026-06-27T12:00:00Z",
    raw: {},
    ...overrides
  };
}

function queueEntry(overrides: Partial<MergeQueueEntry> = {}): MergeQueueEntry {
  return {
    id: "mq-1",
    sourceRef: "refs/lanes/agent-ui-polish",
    targetRef: "refs/heads/main",
    status: "queued",
    priority: 7,
    createdAt: 1782570000,
    updatedAt: "2026-06-27T10:00:00Z",
    raw: {},
    ...overrides
  };
}

test("builds a readable task tree item for blocked coordination state", () => {
  const model = buildTaskTreePresentation(
    task({
      status: "ready",
      changedPaths: [
        "extensions/vscode/src/views/ChatPanel.ts",
        "extensions/vscode/src/webview/main.ts",
        "extensions/vscode/src/webview/styles.css",
        "extensions/vscode/src/views/TasksTreeProvider.ts",
        "extensions/vscode/src/views/SettingsPanel.ts",
        "extensions/vscode/src/test/taskTreeModel.test.ts"
      ],
      coordination: {
        severity: "blocked",
        labels: ["missing test", "1 approval"],
        issues: [
          { code: "missing_test", message: "Run the lane test before applying.", tone: "blocked" },
          { code: "approval_required", message: "Approval is required for write access.", tone: "blocked" }
        ],
        blockers: 2,
        warnings: 0,
        conflicts: 0,
        pendingApprovals: 1,
        queuedMerges: 0,
        changedPaths: 6,
        workdirDirty: false
      },
      nextAction: "Run test"
    })
  );

  assert.equal(model.label, "Improve VS Code extension polish");
  assert.match(model.description ?? "", /Blocked/);
  assert.match(model.description ?? "", /6 changes/);
  assert.equal(model.icon.id, "warning");
  assert.match(model.tooltip, /Coordination: missing test, 1 approval/);
  assert.match(model.tooltip, /Blocked: Run the lane test before applying\./);
  assert.match(model.tooltip, / - 1 more/);
  assert.equal(model.accessibilityLabel, "Improve VS Code extension polish, Blocked, 6 changed paths, next action Run test");
});

test("leads review tree rows with the next safe action", () => {
  const model = buildTaskTreePresentation(
    task({
      status: "ready",
      nextAction: "Dry-run apply before changing the main workspace",
      changedPaths: ["README.md", "extensions/vscode/src/webview/main.ts"]
    }),
    "reviews"
  );

  assert.match(model.description ?? "", /^Next: Dry-run apply/);
  assert.match(model.description ?? "", /2 changes/);
  assert.match(model.tooltip, /Next action: Dry-run apply before changing the main workspace/);
  assert.match(model.accessibilityLabel, /next action Dry-run apply before changing the main workspace/);
});

test("normalizes unusual statuses without leaking whitespace", () => {
  assert.equal(normalizeTreeStatus(" Needs_Review "), "needs_review");

  const model = buildTaskTreePresentation(
    task({
      status: "needs_review",
      title: "  RTL-ready labels  ",
      provider: "A very long provider label that should be shortened in the tree description"
    })
  );

  assert.equal(model.label, "RTL-ready labels");
  assert.match(model.description ?? "", /Needs Review/);
  assert.match(model.tooltip, /Provider: A very long provider label/);
});

test("groups coordinated warnings ahead of raw ready status", () => {
  const model = task({
    status: "ready",
    coordination: {
      severity: "warning",
      labels: ["missing eval"],
      issues: [{ code: "missing_eval", message: "Run eval before merge.", tone: "warning" }],
      blockers: 0,
      warnings: 1,
      conflicts: 0,
      pendingApprovals: 0,
      queuedMerges: 0,
      changedPaths: 1,
      workdirDirty: false
    }
  });

  assert.equal(taskTreeGroupStatus(model), "attention");
  assert.match(buildTaskTreePresentation(model).description ?? "", /Needs attention/);
});

test("builds queue items with short refs, stable time, and merge direction", () => {
  const model = buildQueueItemTreePresentation(
    queueEntry({
      raw: {
        reason: "Waiting for dry-run apply evidence before merge."
      }
    })
  );

  assert.equal(model.label, "agent-ui-polish");
  assert.equal(model.description, "Queued - to main - P7 - Waiting for dry-ru...fore merge.");
  assert.equal(model.icon.id, "git-merge");
  assert.match(model.tooltip, /Source: refs\/lanes\/agent-ui-polish/);
  assert.match(model.tooltip, /Reason: Waiting for dry-run apply evidence before merge\./);
  assert.match(model.tooltip, /Created: 2026-06-27T14:20:00.000Z/);
  assert.equal(model.accessibilityLabel, "agent-ui-polish, Queued, merge into main");
});

test("separates empty states from load failures", () => {
  const empty = buildEmptyTreePresentation("reviews");
  const error = buildEmptyTreePresentation("queue", "daemon unavailable");

  assert.equal(empty.label, "No tasks need review");
  assert.equal(empty.description, "Start a task to create review evidence");
  assert.equal(empty.icon.id, "info");
  assert.equal(empty.command?.command, "trail.newAgentTask");
  assert.equal(empty.accessibilityLabel, "No tasks need review, Start a task to create review evidence");
  assert.equal(error.label, "Trail data unavailable");
  assert.equal(error.description, "Refresh or open settings");
  assert.equal(error.command?.command, "trail.refreshTasks");
  assert.match(error.tooltip, /daemon unavailable/);
  assert.equal(error.icon.id, "warning");
});

test("makes queue empty state route users to review before queueing", () => {
  const empty = buildEmptyTreePresentation("queue");

  assert.equal(empty.label, "No queued merges");
  assert.equal(empty.description, "Open review to queue a lane");
  assert.equal(empty.command?.command, "trail.openLatestReview");
  assert.equal(empty.command?.title, "Open Latest Review");
});

test("labels group rows with count and item kind", () => {
  const taskGroup = buildGroupTreePresentation({ id: "ready", label: "Ready to review", count: 2, kind: "task" });
  const queueGroup = buildGroupTreePresentation({ id: "queued", label: "Queued", count: 1, kind: "queue" });

  assert.equal(taskGroup.label, "Ready to review (2)");
  assert.equal(taskGroup.description, "2 tasks");
  assert.equal(queueGroup.label, "Queued (1)");
  assert.equal(queueGroup.description, "1 entry");
});

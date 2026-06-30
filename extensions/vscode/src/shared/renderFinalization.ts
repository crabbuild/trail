import type { RenderNode, RenderPatch, RenderStatus, ToolPermissionRequest } from "./renderModel";
import type { PlanEntry, ToolCallContent, ToolCallStatus } from "./acpTypes";

export function finalizeAcpLiveTurnPatches(
  nodes: RenderNode[],
  turnId: string | undefined,
  status: RenderStatus,
  updatedAt?: string | undefined
): RenderPatch[] {
  if (!turnId) {
    return [];
  }
  const patches: RenderPatch[] = [];
  for (const node of nodes) {
    const finalized = finalizeAcpLiveTurnNode(node, turnId, status, updatedAt);
    if (finalized !== node) {
      patches.push({ type: "replace", node: finalized });
    }
  }
  return patches;
}

export function finalizeAcpLiveTurnNodes(
  nodes: RenderNode[],
  turnId: string | undefined,
  status: RenderStatus,
  updatedAt?: string | undefined
): RenderNode[] {
  if (!turnId) {
    return nodes;
  }
  let changed = false;
  const next = nodes.map((node) => {
    const finalized = finalizeAcpLiveTurnNode(node, turnId, status, updatedAt);
    changed ||= finalized !== node;
    return finalized;
  });
  return changed ? next : nodes;
}

function finalizeAcpLiveTurnNode(
  node: RenderNode,
  turnId: string,
  status: RenderStatus,
  updatedAt?: string | undefined
): RenderNode {
  if (node.turnId !== turnId || node.source !== "acp-live") {
    return node;
  }
  const finalStatus = status === "pending" ? "completed" : status;
  const timestamp = updatedAt || node.updatedAt;
  switch (node.kind) {
    case "message":
      if (node.status === finalStatus && !node.streaming) {
        return node;
      }
      return withUpdatedAt({ ...node, status: finalStatus, streaming: false }, timestamp);
    case "thought":
      if (node.status === finalStatus) {
        return node;
      }
      return withUpdatedAt({ ...node, status: finalStatus }, timestamp);
    case "plan": {
      const nextEntries = finalizePlanEntries(node.entries, finalStatus);
      if (!isActiveStatus(node.status) && nextEntries === node.entries) {
        return node;
      }
      return withUpdatedAt(
        {
          ...node,
          status: isActiveStatus(node.status) ? finalStatus : node.status,
          entries: nextEntries
        },
        timestamp
      );
    }
    case "diff":
    case "resource":
      if (!isActiveStatus(node.status)) {
        return node;
      }
      return withUpdatedAt({ ...node, status: finalStatus }, timestamp);
    case "terminal": {
      if (!isActiveStatus(node.status) && !isActiveStatus(node.terminalStatus)) {
        return node;
      }
      return withUpdatedAt(
        {
          ...node,
          status: isActiveStatus(node.status) ? finalStatus : node.status,
          terminalStatus: isActiveStatus(node.terminalStatus) ? finalStatus : node.terminalStatus
        },
        timestamp
      );
    }
    case "tool": {
      const nextStatus = isActiveStatus(node.status) ? finalStatus : node.status;
      const nextToolStatus = isActiveStatus(node.toolStatus) ? finalStatus : node.toolStatus;
      const nextPermission = finalizeToolPermission(node.permission, finalStatus, timestamp);
      const nextContent = finalizeToolTerminalContent(node.content, nextToolStatus);
      if (
        nextStatus === node.status &&
        nextToolStatus === node.toolStatus &&
        nextPermission === node.permission &&
        nextContent === node.content
      ) {
        return node;
      }
      return withUpdatedAt(
        {
          ...node,
          status: nextStatus,
          toolStatus: nextToolStatus,
          permission: nextPermission,
          content: nextContent
        },
        timestamp
      );
    }
    case "approval":
      if ((finalStatus !== "failed" && finalStatus !== "cancelled") || !isActiveStatus(node.status)) {
        return node;
      }
      return withUpdatedAt({ ...node, status: "cancelled" }, timestamp);
    default:
      return node;
  }
}

function finalizePlanEntries(entries: PlanEntry[], finalStatus: RenderStatus): PlanEntry[] {
  let changed = false;
  const next = entries.map((entry) => {
    if (!isActiveStatus(entry.status)) {
      return entry;
    }
    changed = true;
    return {
      ...entry,
      status: finalStatus
    };
  });
  return changed ? next : entries;
}

function finalizeToolPermission(
  permission: ToolPermissionRequest | undefined,
  finalStatus: RenderStatus,
  updatedAt: string | undefined
): ToolPermissionRequest | undefined {
  if (!permission || !isActiveStatus(permission.status) || (finalStatus !== "failed" && finalStatus !== "cancelled")) {
    return permission;
  }
  return {
    ...permission,
    status: "cancelled",
    updatedAt
  };
}

function finalizeToolTerminalContent(content: ToolCallContent[], toolStatus: ToolCallStatus): ToolCallContent[] {
  if (!content.length || !isFinalStatus(toolStatus)) {
    return content;
  }
  let changed = false;
  const next = content.map((item) => {
    const record = item as Record<string, unknown>;
    if (record.type !== "terminal") {
      return item;
    }
    const current = stringField(record, "terminalStatus") || stringField(record, "status") || stringField(record, "state");
    if (current !== undefined && !isActiveStatus(current)) {
      return item;
    }
    changed = true;
    const updated: Record<string, unknown> = {
      ...record,
      status: toolStatus
    };
    if (typeof record.terminalStatus === "string") {
      updated.terminalStatus = toolStatus;
    }
    return updated as ToolCallContent;
  });
  return changed ? next : content;
}

function withUpdatedAt<TNode extends RenderNode>(node: TNode, updatedAt: string | undefined): TNode {
  return updatedAt === undefined ? node : ({ ...node, updatedAt } as TNode);
}

function isActiveStatus(status: string | undefined): boolean {
  switch (normalizeStatus(status)) {
    case "pending":
    case "in_progress":
    case "in-progress":
    case "active":
    case "running":
      return true;
    default:
      return false;
  }
}

function isFinalStatus(status: string | undefined): boolean {
  switch (normalizeStatus(status)) {
    case "completed":
    case "succeeded":
    case "success":
    case "passed":
    case "failed":
    case "error":
    case "cancelled":
    case "canceled":
      return true;
    default:
      return false;
  }
}

function normalizeStatus(status: string | undefined): string | undefined {
  return status?.trim().toLowerCase().replace(/\s+/g, "_");
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

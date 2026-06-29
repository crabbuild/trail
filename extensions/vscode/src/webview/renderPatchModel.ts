import type { RenderNode, RenderPatch } from "../shared/renderModel";

export interface RenderPatchChanges {
  changedNodeIds: Set<string>;
  addedNodeIds: Set<string>;
  removedNodeIds: Set<string>;
}

export function parseRenderRevision(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0 ? value : undefined;
}

export function parseBaseRenderRevision(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}

export function shouldAcceptRenderRevision(revision: number | undefined, latestRevision: number): boolean {
  return revision === undefined ? latestRevision === 0 : revision > latestRevision;
}

export type RenderPatchBatchDecision = "apply" | "drop" | "resync";

export function renderPatchBatchDecision(
  baseRevision: number | undefined,
  revision: number | undefined,
  latestRevision: number
): RenderPatchBatchDecision {
  if (baseRevision === undefined || revision === undefined || revision <= latestRevision) {
    return "drop";
  }
  return baseRevision === latestRevision ? "apply" : "resync";
}

export function applyRenderPatchesLocally(nodes: RenderNode[], patches: RenderPatch[]): RenderNode[] {
  let next = [...nodes];
  for (const patch of patches) {
    if ((patch.type === "append" || patch.type === "replace" || patch.type === "upsert") && patch.node) {
      const index = next.findIndex((node) => node.id === patch.node?.id);
      if (index >= 0) {
        next[index] = patch.node;
      } else {
        next.push(patch.node);
      }
      continue;
    }
    if (patch.type === "remove" && patch.id) {
      next = next.filter((node) => node.id !== patch.id);
    }
  }
  return next;
}

export function changedRenderNodes(beforeById: Map<string, RenderNode>, nextNodes: RenderNode[]): RenderPatchChanges {
  const nextIds = new Set(nextNodes.map((node) => node.id));
  const changedNodeIds = new Set<string>();
  const addedNodeIds = new Set<string>();
  for (const node of nextNodes) {
    const previous = beforeById.get(node.id);
    if (!previous) {
      addedNodeIds.add(node.id);
      changedNodeIds.add(node.id);
    } else if (previous !== node) {
      changedNodeIds.add(node.id);
    }
  }
  const removedNodeIds = new Set([...beforeById.keys()].filter((id) => !nextIds.has(id)));
  return { changedNodeIds, addedNodeIds, removedNodeIds };
}

export function isStreamingTextPatchPayload(patch: RenderPatch): boolean {
  const node = patch.node;
  if (patch.type !== "upsert" || !node || node.source !== "acp-live") {
    return false;
  }
  if (node.status !== "pending" && node.status !== "in_progress") {
    return false;
  }
  if (node.kind !== "message" && node.kind !== "thought") {
    return false;
  }
  return node.content.length > 0 && node.content.every((block) => block.type === "text");
}

export function isLiveNodePatchPayload(patch: RenderPatch): boolean {
  const node = patch.node;
  if ((patch.type !== "upsert" && patch.type !== "replace") || !node || node.source !== "acp-live") {
    return false;
  }
  if (node.status !== "pending" && node.status !== "in_progress") {
    return false;
  }
  if (node.kind === "message" || node.kind === "thought") {
    return isStreamingTextPatchPayload(patch);
  }
  return node.kind === "plan" || node.kind === "tool" || node.kind === "diff" || node.kind === "terminal";
}

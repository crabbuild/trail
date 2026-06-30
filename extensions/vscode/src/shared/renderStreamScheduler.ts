import type { ContentBlock } from "./acpTypes";
import type { MessageNode, RenderNode, RenderPatch, ThoughtNode } from "./renderModel";

export interface RenderStreamSchedulerOptions {
  flushMs?: number;
  componentFlushMs?: number;
  shouldCoalescePatch?: (patch: RenderPatch) => boolean;
}

export interface RenderStreamSchedulerStats {
  received: number;
  emitted: number;
  batches: number;
  coalesced: number;
}

const DEFAULT_FLUSH_MS = 16;
const DEFAULT_COMPONENT_FLUSH_MS = 50;

type StreamNode = MessageNode | ThoughtNode;

export class RenderStreamScheduler {
  private timer: ReturnType<typeof setTimeout> | undefined;
  private componentTimer: ReturnType<typeof setTimeout> | undefined;
  private readonly queue = new Map<string, RenderPatch>();
  private readonly componentQueue = new Map<string, RenderPatch>();
  private readonly flushMs: number;
  private readonly componentFlushMs: number;
  private readonly shouldCoalescePatch: (patch: RenderPatch) => boolean;
  private readonly counters: RenderStreamSchedulerStats = {
    received: 0,
    emitted: 0,
    batches: 0,
    coalesced: 0
  };

  constructor(
    private readonly send: (patches: RenderPatch[]) => void,
    options: RenderStreamSchedulerOptions = {}
  ) {
    this.flushMs = options.flushMs ?? DEFAULT_FLUSH_MS;
    this.componentFlushMs = options.componentFlushMs ?? DEFAULT_COMPONENT_FLUSH_MS;
    this.shouldCoalescePatch = options.shouldCoalescePatch ?? (() => false);
  }

  push(patches: RenderPatch[]): void {
    let structural: RenderPatch[] = [];
    for (const patch of patches) {
      if (isStreamTextPatch(patch)) {
        if (structural.length) {
          this.emit(structural);
          structural = [];
        }
        this.pushStreamPatch(patch);
        continue;
      }
      if (this.shouldCoalescePatch(patch)) {
        if (structural.length) {
          this.emit(structural);
          structural = [];
        }
        this.pushComponentPatch(patch);
        continue;
      }
      this.counters.received += 1;
      this.flush();
      structural.push(patch);
    }
    if (structural.length) {
      this.emit(structural);
    }
  }

  flush(): void {
    this.clearTimers();
    const patches = [...this.queue.values(), ...this.componentQueue.values()];
    this.queue.clear();
    this.componentQueue.clear();
    this.emit(patches);
  }

  dispose(): void {
    this.clearTimers();
    this.queue.clear();
    this.componentQueue.clear();
  }

  stats(): Readonly<RenderStreamSchedulerStats> {
    return this.counters;
  }

  private pushStreamPatch(patch: RenderPatch & { node: StreamNode }): void {
    this.counters.received += 1;
    const key = patch.node.id;

    const previous = this.queue.get(key);
    if (previous) {
      this.counters.coalesced += 1;
    }
    this.queue.set(key, mergeStreamPatch(previous, patch));
    this.schedule();
  }

  private pushComponentPatch(patch: RenderPatch): void {
    this.counters.received += 1;
    const key = patch.node?.id;
    if (!key) {
      this.flush();
      this.emit([patch]);
      return;
    }

    const previous = this.componentQueue.get(key);
    if (previous) {
      this.counters.coalesced += 1;
    }
    this.componentQueue.set(key, patch);
    this.scheduleComponent();
  }

  private schedule(): void {
    if (this.timer) {
      return;
    }
    this.timer = setTimeout(() => {
      this.flushStream();
    }, this.flushMs);
  }

  private scheduleComponent(): void {
    if (this.componentTimer) {
      return;
    }
    this.componentTimer = setTimeout(() => {
      this.flushComponents();
    }, this.componentFlushMs);
  }

  private emit(patches: RenderPatch[]): void {
    if (!patches.length) {
      return;
    }
    this.counters.emitted += patches.length;
    this.counters.batches += 1;
    this.send(patches);
  }

  private flushStream(): void {
    this.clearStreamTimer();
    const patches = [...this.queue.values()];
    this.queue.clear();
    this.emit(patches);
  }

  private flushComponents(): void {
    this.clearComponentTimer();
    const patches = [...this.componentQueue.values()];
    this.componentQueue.clear();
    this.emit(patches);
  }

  private clearTimers(): void {
    this.clearStreamTimer();
    this.clearComponentTimer();
  }

  private clearStreamTimer(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
  }

  private clearComponentTimer(): void {
    if (this.componentTimer) {
      clearTimeout(this.componentTimer);
      this.componentTimer = undefined;
    }
  }
}

export function isStreamTextPatch(patch: RenderPatch): patch is RenderPatch & { node: StreamNode } {
  if (patch.type !== "upsert" || !patch.node) {
    return false;
  }
  return isStreamTextNode(patch.node);
}

function isStreamTextNode(node: RenderNode): node is StreamNode {
  if (node.source !== "acp-live" || (node.status !== "pending" && node.status !== "in_progress")) {
    return false;
  }
  if (node.kind !== "message" && node.kind !== "thought") {
    return false;
  }
  return node.content.length > 0 && node.content.every((block) => block.type === "text");
}

function mergeStreamPatch(previous: RenderPatch | undefined, incoming: RenderPatch): RenderPatch {
  if (!previous?.node || !incoming.node || previous.node.kind !== incoming.node.kind) {
    return incoming;
  }
  if (incoming.node.kind === "message" && previous.node.kind === "message") {
    const content = mergeAdjacentTextContentBlocks([...previous.node.content, ...incoming.node.content]);
    return {
      ...incoming,
      node: {
        ...incoming.node,
        createdAt: previous.node.createdAt,
        content,
        text: content.map(contentToText).join(""),
        streaming: previous.node.streaming || incoming.node.streaming
      }
    };
  }
  if (incoming.node.kind === "thought" && previous.node.kind === "thought") {
    return {
      ...incoming,
      node: {
        ...incoming.node,
        createdAt: previous.node.createdAt,
        content: mergeAdjacentTextContentBlocks([...previous.node.content, ...incoming.node.content])
      }
    };
  }
  return incoming;
}

function mergeAdjacentTextContentBlocks(blocks: ContentBlock[]): ContentBlock[] {
  const merged: ContentBlock[] = [];
  for (const block of blocks) {
    const previous = merged[merged.length - 1];
    if (previous?.type === "text" && block.type === "text") {
      merged[merged.length - 1] = {
        ...previous,
        text: `${previous.text}${block.text}`
      };
      continue;
    }
    merged.push(block);
  }
  return merged;
}

function contentToText(content: ContentBlock): string {
  if (content.type === "text") {
    const text = (content as { text?: unknown }).text;
    return typeof text === "string" ? text : "";
  }
  return `[${content.type || "content"}]`;
}

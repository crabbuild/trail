import type { ContentBlock } from "./acpTypes";
import type { ToolCallContent, ToolCallLocation } from "./acpTypes";
import type { DiffNode, MessageNode, RenderNode, RenderPatch, TerminalNode, ThoughtNode, ToolNode } from "./renderModel";

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
type QueuedPatchKind = "stream" | "component";

interface QueuedPatchEntry {
  kind: QueuedPatchKind;
  key: string;
}

export class RenderStreamScheduler {
  private timer: ReturnType<typeof setTimeout> | undefined;
  private componentTimer: ReturnType<typeof setTimeout> | undefined;
  private readonly queue = new Map<string, RenderPatch>();
  private readonly componentQueue = new Map<string, RenderPatch>();
  private readonly queueOrder: QueuedPatchEntry[] = [];
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
    const patches = this.queuedPatches();
    this.queue.clear();
    this.componentQueue.clear();
    this.queueOrder.splice(0);
    this.emit(patches);
  }

  dispose(): void {
    this.clearTimers();
    this.queue.clear();
    this.componentQueue.clear();
    this.queueOrder.splice(0);
  }

  stats(): Readonly<RenderStreamSchedulerStats> {
    return this.counters;
  }

  private pushStreamPatch(patch: RenderPatch & { node: StreamNode }): void {
    this.counters.received += 1;
    let key = renderPatchCoalesceKey(patch) ?? patch.node.id;

    let previous = this.queue.get(key);
    if (!previous) {
      const compatibleKey = queuedCompatibleStreamPatchKey(this.queue, patch.node);
      if (compatibleKey) {
        key = compatibleKey;
        previous = this.queue.get(key);
      }
    }
    if (previous) {
      this.counters.coalesced += 1;
    } else {
      this.trackQueuedPatch("stream", key);
    }
    this.queue.set(key, mergeStreamPatch(previous, patch));
    this.schedule();
  }

  private pushComponentPatch(patch: RenderPatch): void {
    this.counters.received += 1;
    const key = renderPatchCoalesceKey(patch);
    if (!key) {
      this.flush();
      this.emit([patch]);
      return;
    }

    const previous = this.componentQueue.get(key);
    if (previous) {
      this.counters.coalesced += 1;
    } else {
      this.trackQueuedPatch("component", key);
    }
    this.componentQueue.set(key, mergeComponentPatch(previous, patch));
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
    const patches = this.takeQueuedPatchesThrough("stream");
    this.clearTimersForEmptyQueues();
    this.emit(patches);
  }

  private flushComponents(): void {
    this.clearComponentTimer();
    const patches = this.takeQueuedPatchesThrough("component");
    this.clearTimersForEmptyQueues();
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

  private trackQueuedPatch(kind: QueuedPatchKind, key: string): void {
    if (!this.queueOrder.some((entry) => entry.kind === kind && entry.key === key)) {
      this.queueOrder.push({ kind, key });
    }
  }

  private queuedPatches(kind?: QueuedPatchKind): RenderPatch[] {
    const patches: RenderPatch[] = [];
    for (const entry of this.queueOrder) {
      if (kind && entry.kind !== kind) {
        continue;
      }
      const patch = entry.kind === "stream" ? this.queue.get(entry.key) : this.componentQueue.get(entry.key);
      if (patch) {
        patches.push(patch);
      }
    }
    return patches;
  }

  private takeQueuedPatchesThrough(kind: QueuedPatchKind): RenderPatch[] {
    let lastIndex = -1;
    for (let index = 0; index < this.queueOrder.length; index += 1) {
      if (this.queueOrder[index]?.kind === kind) {
        lastIndex = index;
      }
    }
    if (lastIndex < 0) {
      return [];
    }
    const entries = this.queueOrder.splice(0, lastIndex + 1);
    const patches: RenderPatch[] = [];
    for (const entry of entries) {
      const queue = entry.kind === "stream" ? this.queue : this.componentQueue;
      const patch = queue.get(entry.key);
      queue.delete(entry.key);
      if (patch) {
        patches.push(patch);
      }
    }
    return patches;
  }

  private clearTimersForEmptyQueues(): void {
    if (!this.queue.size) {
      this.clearStreamTimer();
    }
    if (!this.componentQueue.size) {
      this.clearComponentTimer();
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

function renderPatchCoalesceKey(patch: RenderPatch): string | undefined {
  const node = patch.node;
  if (!node) {
    return undefined;
  }
  return [
    node.id,
    node.kind,
    node.taskId,
    node.lane,
    node.turnId || "",
    node.acpSessionId || "",
    node.source
  ].join("\u0000");
}

function queuedCompatibleStreamPatchKey(
  queue: Map<string, RenderPatch>,
  incoming: StreamNode
): string | undefined {
  for (const [key, patch] of queue) {
    const previous = patch.node;
    if (previous && isCompatibleStreamPatchNode(previous, incoming)) {
      return key;
    }
  }
  return undefined;
}

function isCompatibleStreamPatchNode(previous: RenderNode, incoming: StreamNode): previous is StreamNode {
  if (
    previous.kind !== incoming.kind ||
    previous.taskId !== incoming.taskId ||
    previous.lane !== incoming.lane ||
    previous.turnId !== incoming.turnId ||
    !compatibleOptionalScopeValue(previous.acpSessionId, incoming.acpSessionId) ||
    previous.source !== incoming.source
  ) {
    return false;
  }
  if (previous.kind === "message" && incoming.kind === "message" && previous.role !== incoming.role) {
    return false;
  }
  if (previous.acpMessageId && incoming.acpMessageId) {
    return previous.acpMessageId === incoming.acpMessageId;
  }
  return previous.acpMessageId !== incoming.acpMessageId;
}

function compatibleOptionalScopeValue(left: string | undefined, right: string | undefined): boolean {
  return left === right || left === undefined || right === undefined;
}

function mergeStreamPatch(previous: RenderPatch | undefined, incoming: RenderPatch): RenderPatch {
  if (!previous?.node || !incoming.node || previous.node.kind !== incoming.node.kind) {
    return incoming;
  }
  if (incoming.node.kind === "message" && previous.node.kind === "message") {
    const content = mergeStreamTextContent(previous.node.content, incoming.node.content);
    return {
      ...incoming,
      node: {
        ...incoming.node,
        id: previous.node.id,
        acpSessionId: incoming.node.acpSessionId || previous.node.acpSessionId,
        provider: incoming.node.provider || previous.node.provider,
        acpMessageId: previous.node.acpMessageId || incoming.node.acpMessageId,
        createdAt: previous.node.createdAt ?? incoming.node.createdAt,
        timelineOrder: previous.node.timelineOrder ?? incoming.node.timelineOrder,
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
        id: previous.node.id,
        acpSessionId: incoming.node.acpSessionId || previous.node.acpSessionId,
        provider: incoming.node.provider || previous.node.provider,
        acpMessageId: previous.node.acpMessageId || incoming.node.acpMessageId,
        createdAt: previous.node.createdAt ?? incoming.node.createdAt,
        timelineOrder: previous.node.timelineOrder ?? incoming.node.timelineOrder,
        content: mergeStreamTextContent(previous.node.content, incoming.node.content)
      }
    };
  }
  return incoming;
}

function mergeComponentPatch(previous: RenderPatch | undefined, incoming: RenderPatch): RenderPatch {
  if (!previous?.node || !incoming.node || previous.node.kind !== incoming.node.kind) {
    return incoming;
  }
  if (previous.node.kind === "terminal" && incoming.node.kind === "terminal") {
    return {
      ...incoming,
      node: mergeTerminalComponentPatch(previous.node, incoming.node)
    };
  }
  if (previous.node.kind === "diff" && incoming.node.kind === "diff") {
    return {
      ...incoming,
      node: mergeDiffComponentPatch(previous.node, incoming.node)
    };
  }
  if (previous.node.kind === "tool" && incoming.node.kind === "tool") {
    return {
      ...incoming,
      node: mergeToolComponentPatch(previous.node, incoming.node)
    };
  }
  return incoming;
}

function mergeTerminalComponentPatch(previous: TerminalNode, incoming: TerminalNode): TerminalNode {
  const merged: TerminalNode = {
    ...incoming,
    createdAt: previous.createdAt ?? incoming.createdAt,
    status: mergedComponentStatus(previous.status, incoming.status)
  };
  assignOptional(merged, "title", incoming.title ?? previous.title);
  assignOptional(merged, "command", incoming.command ?? previous.command);
  assignOptional(merged, "cwd", incoming.cwd ?? previous.cwd);
  assignOptional(merged, "terminalStatus", mergedComponentStatus(previous.terminalStatus, incoming.terminalStatus));
  assignOptional(merged, "exitCode", incoming.exitCode ?? previous.exitCode);
  assignOptional(merged, "elapsedMs", incoming.elapsedMs ?? previous.elapsedMs);
  assignOptional(merged, "output", mergeTerminalText(previous.output, incoming.output, incoming, ["outputDelta", "output_delta"]));
  assignOptional(merged, "stdout", mergeTerminalText(previous.stdout, incoming.stdout, incoming, ["stdoutDelta", "stdout_delta"]));
  assignOptional(merged, "stderr", mergeTerminalText(previous.stderr, incoming.stderr, incoming, ["stderrDelta", "stderr_delta"]));
  return merged;
}

function mergeTerminalText(
  existing: string | undefined,
  incoming: string | undefined,
  incomingSource: unknown,
  deltaKeys: string[]
): string | undefined {
  const delta = terminalDeltaText(incomingSource, deltaKeys);
  if (delta !== undefined) {
    return `${existing || ""}${delta}`;
  }
  return incoming ?? existing;
}

function terminalDeltaText(source: unknown, deltaKeys: string[]): string | undefined {
  const sourceRecord = asRecord(source);
  const rawRecord = asRecord(sourceRecord.raw);
  for (const record of [rawRecord, sourceRecord]) {
    for (const key of deltaKeys) {
      const value = stringField(record, key);
      if (value !== undefined) {
        return value;
      }
    }
  }
  return undefined;
}

function mergeDiffComponentPatch(previous: DiffNode, incoming: DiffNode): DiffNode {
  const merged: DiffNode = {
    ...incoming,
    createdAt: previous.createdAt ?? incoming.createdAt,
    status: mergedComponentStatus(previous.status, incoming.status),
    path: incoming.path || previous.path
  };
  if (incoming.oldText === undefined && previous.oldText !== undefined) {
    merged.oldText = previous.oldText;
  }
  return merged;
}

function mergedComponentStatus<TStatus extends string | undefined>(previous: TStatus, incoming: TStatus): TStatus {
  return isFinalStatus(previous) && isActiveStatus(incoming) ? previous : incoming;
}

function isActiveStatus(status: string | undefined): boolean {
  const normalized = normalizeStatus(status);
  return normalized === "pending" || normalized === "in-progress" || normalized === "running";
}

function isFinalStatus(status: string | undefined): boolean {
  const normalized = normalizeStatus(status);
  return normalized === "completed" || normalized === "succeeded" || normalized === "failed" || normalized === "cancelled" || normalized === "canceled";
}

function normalizeStatus(status: string | undefined): string {
  return String(status || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function assignOptional<TObject extends object, TKey extends keyof TObject>(
  target: TObject,
  key: TKey,
  value: TObject[TKey] | undefined
): void {
  if (value !== undefined) {
    target[key] = value;
  }
}

function mergeToolComponentPatch(previous: ToolNode, incoming: ToolNode): ToolNode {
  const explicitStatus = hasExplicitToolStatus(incoming);
  return {
    ...incoming,
    createdAt: previous.createdAt ?? incoming.createdAt,
    status: explicitStatus ? mergedComponentStatus(previous.status, incoming.status) : previous.status,
    title: incoming.title && incoming.title !== "Tool call" ? incoming.title : previous.title,
    toolKind: incoming.toolKind !== "other" ? incoming.toolKind : previous.toolKind,
    toolStatus: explicitStatus ? mergedComponentStatus(previous.toolStatus, incoming.toolStatus) : previous.toolStatus,
    locations: incoming.locations.length ? mergeToolLocations(previous.locations, incoming.locations) : previous.locations,
    content: incoming.content.length ? mergeToolContent(previous.content, incoming.content) : previous.content,
    rawInput: incoming.rawInput ?? previous.rawInput,
    rawOutput: incoming.rawOutput ?? previous.rawOutput
  };
}

function hasExplicitToolStatus(node: ToolNode): boolean {
  const raw = node.raw;
  return Boolean(
    raw &&
      typeof raw === "object" &&
      (typeof (raw as { status?: unknown }).status === "string" ||
        typeof (raw as { state?: unknown }).state === "string")
  );
}

function mergeToolLocations(existing: ToolCallLocation[], incoming: ToolCallLocation[]): ToolCallLocation[] {
  const seen = new Set(existing.map((location) => `${location.path}:${location.line ?? ""}`));
  const merged = [...existing];
  for (const location of incoming) {
    const key = `${location.path}:${location.line ?? ""}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    merged.push(location);
  }
  return merged;
}

function mergeToolContent(existing: ToolCallContent[], incoming: ToolCallContent[]): ToolCallContent[] {
  const merged = [...existing];
  for (const item of incoming) {
    const key = stableContentKey(item);
    const index = merged.findIndex((candidate) => stableContentKey(candidate) === key);
    if (index >= 0) {
      merged[index] = mergeToolContentItem(merged[index]!, item) ?? item;
      continue;
    }
    merged.push(item);
  }
  return merged;
}

function mergeToolContentItem(existing: ToolCallContent, incoming: ToolCallContent): ToolCallContent | undefined {
  const existingRecord = existing as Record<string, unknown>;
  const incomingRecord = incoming as Record<string, unknown>;
  if (
    existingRecord.type === "terminal" &&
    incomingRecord.type === "terminal" &&
    typeof existingRecord.terminalId === "string" &&
    existingRecord.terminalId === incomingRecord.terminalId
  ) {
    return mergeTerminalContentRecord(existingRecord, incomingRecord) as ToolCallContent;
  }
  return undefined;
}

function mergeTerminalContentRecord(
  existing: Record<string, unknown>,
  incoming: Record<string, unknown>
): Record<string, unknown> {
  const merged = {
    ...existing,
    ...incoming
  };
  assignMergedTerminalText(merged, existing, incoming, "output", ["outputDelta", "output_delta"]);
  assignMergedTerminalText(merged, existing, incoming, "stdout", ["stdoutDelta", "stdout_delta"]);
  assignMergedTerminalText(merged, existing, incoming, "stderr", ["stderrDelta", "stderr_delta"]);
  return merged;
}

function assignMergedTerminalText(
  target: Record<string, unknown>,
  existing: Record<string, unknown>,
  incoming: Record<string, unknown>,
  key: "output" | "stdout" | "stderr",
  deltaKeys: string[]
): void {
  const existingValue = stringField(existing, key);
  const incomingValue = stringField(incoming, key);
  const merged = mergeTerminalText(existingValue, incomingValue, incoming, deltaKeys);
  if (merged !== undefined) {
    target[key] = merged;
  }
}

function stableContentKey(content: ToolCallContent): string {
  const record = content as Record<string, unknown>;
  if (record.type === "terminal" && typeof record.terminalId === "string") {
    return `terminal:${record.terminalId}`;
  }
  try {
    return JSON.stringify(content);
  } catch {
    return String(content);
  }
}

function mergeAdjacentTextContentBlocks(blocks: ContentBlock[]): ContentBlock[] {
  const merged: ContentBlock[] = [];
  for (const block of blocks) {
    const previous = merged[merged.length - 1];
    if (previous?.type === "text" && block.type === "text") {
      const previousText = textContentValue(previous);
      const blockText = textContentValue(block);
      if (previousText === undefined || blockText === undefined) {
        merged.push(block);
        continue;
      }
      merged[merged.length - 1] = {
        ...previous,
        text: `${previousText}${blockText}`
      };
      continue;
    }
    merged.push(block);
  }
  return merged;
}

function mergeStreamTextContent(previous: ContentBlock[], incoming: ContentBlock[]): ContentBlock[] {
  const previousText = textContentBlocksToText(previous);
  const incomingText = textContentBlocksToText(incoming);
  if (incomingText.startsWith(previousText)) {
    return mergeAdjacentTextContentBlocks(incoming);
  }
  if (previousText.startsWith(incomingText)) {
    return mergeAdjacentTextContentBlocks(previous);
  }
  return mergeAdjacentTextContentBlocks([...previous, ...incoming]);
}

function textContentBlocksToText(blocks: ContentBlock[]): string {
  return blocks.map(contentToText).join("");
}

function contentToText(content: ContentBlock): string {
  if (content.type === "text") {
    return textContentValue(content) ?? "";
  }
  return `[${content.type || "content"}]`;
}

function textContentValue(content: ContentBlock): string | undefined {
  if (content.type !== "text") {
    return undefined;
  }
  const record = content as Record<string, unknown>;
  const fallbackValues: string[] = [];
  for (const key of ["text", "content", "value"]) {
    const value = stringField(record, key);
    if (value === undefined) {
      continue;
    }
    if (value.length > 0) {
      return value;
    }
    fallbackValues.push(value);
  }
  return fallbackValues[0];
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

import type { PromptResponse } from "./acpTypes";
import type { CompletionNode, RenderReduceContext, RenderStatus } from "./renderModel";

export function promptCompletionNode(
  response: unknown,
  context: RenderReduceContext
): CompletionNode {
  const record = asRecord(response);
  const stopReason = typeof record.stopReason === "string" ? record.stopReason : "unknown";
  const status = statusForStopReason(stopReason);
  return {
    id: `completion:${context.currentTurnId || context.taskId}`,
    kind: "completion",
    taskId: context.taskId,
    lane: context.lane,
    turnId: context.currentTurnId,
    acpSessionId: context.acpSessionId,
    provider: context.provider,
    source: "acp-live",
    status,
    updatedAt: context.now(),
    raw: response,
    stopReason,
    label: labelForStopReason(stopReason),
    checkpointPending: stopReason === "end_turn"
  };
}

export function statusForStopReason(stopReason: string): RenderStatus {
  if (stopReason === "end_turn") {
    return "pending";
  }
  if (stopReason === "cancelled") {
    return "cancelled";
  }
  return "failed";
}

export function labelForStopReason(stopReason: string): string {
  switch (stopReason) {
    case "end_turn":
      return "Turn complete; checkpoint pending";
    case "max_tokens":
      return "Stopped after reaching the token limit";
    case "max_turn_requests":
      return "Stopped after reaching the turn request limit";
    case "refusal":
      return "Agent refused to continue";
    case "cancelled":
      return "Turn cancelled";
    default:
      return `Stopped: ${stopReason || "unknown"}`;
  }
}

function asRecord(value: unknown): PromptResponse {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as PromptResponse) : {};
}

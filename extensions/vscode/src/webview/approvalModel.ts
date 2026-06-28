export type ApprovalTone = "info" | "risk" | "success" | "warning";
export type ApprovalDecisionTone = "default" | "primary" | "risk" | "warning";

export interface ApprovalToneInput {
  status: string;
  toolKind: string;
}

export function approvalStateLabel(status: string): string {
  switch (status) {
    case "completed":
      return "Approved";
    case "cancelled":
      return "Rejected";
    case "failed":
      return "Failed";
    case "pending":
      return "Needs decision";
    default:
      return status || "Pending";
  }
}

export function approvalTone({ status, toolKind }: ApprovalToneInput): ApprovalTone {
  if (status === "completed") {
    return "success";
  }
  if (status === "cancelled" || status === "failed") {
    return "risk";
  }
  if (toolKind === "execute" || toolKind === "delete") {
    return "risk";
  }
  if (toolKind === "edit" || toolKind === "move") {
    return "warning";
  }
  return "info";
}

export function approvalScopeLabel(locationCount: number, lane: string): string {
  if (locationCount > 0) {
    return `${locationCount} affected location${locationCount === 1 ? "" : "s"}`;
  }
  return lane ? `Lane ${lane}` : "No file scope reported";
}

export function approvalImpactText(toolKind: string, locationCount: number): string {
  const scope = locationCount > 0 ? approvalScopeLabel(locationCount, "") : "the current task";
  switch (toolKind) {
    case "execute":
      return `The agent is asking to run a command that can inspect or change ${scope}.`;
    case "delete":
      return `The agent is asking to delete content in ${scope}.`;
    case "edit":
      return `The agent is asking to edit ${scope}.`;
    case "move":
      return `The agent is asking to move or rename content in ${scope}.`;
    case "read":
      return `The agent is asking to read ${scope}.`;
    case "fetch":
      return "The agent is asking to fetch external context before continuing.";
    default:
      return `The agent is asking to continue with a ${toolKind || "tool"} action.`;
  }
}

export function approvalDecisionTone({ status, toolKind }: ApprovalToneInput): ApprovalDecisionTone {
  if (status !== "pending") {
    return "default";
  }
  switch (toolKind) {
    case "read":
    case "fetch":
    case "search":
    case "think":
      return "primary";
    case "delete":
      return "risk";
    case "edit":
    case "move":
    case "execute":
      return "warning";
    default:
      return "default";
  }
}

export function approvalDecisionDescription(toolKind: string): string {
  switch (toolKind) {
    case "read":
      return "Allow read-only context.";
    case "fetch":
      return "Allow external context.";
    case "search":
      return "Allow workspace search.";
    case "edit":
      return "Allow edit after reviewing preview.";
    case "move":
      return "Allow move or rename after reviewing scope.";
    case "execute":
      return "Allow command after reviewing risk.";
    case "delete":
      return "Allow only if the destructive target is correct.";
    default:
      return "Allow provider tool action.";
  }
}

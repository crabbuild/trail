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
      return `Run a command in ${scope}.`;
    case "delete":
      return `Delete content in ${scope}.`;
    case "edit":
      return `Edit ${scope}.`;
    case "move":
      return `Move or rename content in ${scope}.`;
    case "read":
      return `Read ${scope}.`;
    case "fetch":
      return "Fetch external context.";
    default:
      return `Continue with a ${toolKind || "tool"} action.`;
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

export function approvalActionLabel(label: string, optionId: string): string {
  const raw = String(label || optionId || "").trim();
  const value = `${raw} ${optionId || ""}`.toLowerCase();
  if (/\balways\b/.test(value) && /\b(allow|approve)\b/.test(value)) {
    return "Always allow";
  }
  if (/\b(allow|approve)\b/.test(value)) {
    return "Allow";
  }
  if (/\b(reject|deny|decline|cancel|refuse|disallow)\b/.test(value)) {
    return "Reject";
  }
  if (!raw) {
    return "Allow";
  }
  return raw.length > 28 ? `${raw.slice(0, 25)}...` : raw;
}

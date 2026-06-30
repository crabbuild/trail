import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"
import {
  Bot,
  ChevronDown,
  Check,
  CircleAlert,
  CircleX,
  Clock3,
  Diff,
  ExternalLink,
  FileDiff,
  FileText,
  LoaderCircle,
  PanelRightOpen,
  Search,
  Settings,
  ShieldCheck,
  Terminal,
  Wrench,
  X,
  type LucideIcon
} from "lucide-react"

import { Button } from "@/webview/components/ui/button"
import { ButtonGroup } from "@/webview/components/ui/button-group"
import { Badge } from "@/webview/components/ui/badge"
import { Card, CardContent } from "@/webview/components/ui/card"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger
} from "@/webview/components/ui/collapsible"
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger
} from "@/webview/components/ui/hover-card"
import { cn } from "@/webview/lib/utils"
import type {
  ToolAction,
  ToolFact,
  ToolPresentation,
  ToolStat
} from "./toolModel"

type ToolCardModel = Pick<
  ToolPresentation,
  | "icon"
  | "kind"
  | "operationLabel"
  | "openByDefault"
  | "riskLabel"
  | "riskTone"
  | "statusLabel"
  | "tone"
>

export interface ToolCallCardLocation {
  path: string
  line?: number | undefined
}

export interface ToolCallApprovalAction {
  kind: "approve" | "reject"
  optionId?: string | undefined
  label: string
  description: string
  tone: "default" | "primary" | "risk" | "warning"
  disabled: boolean
}

export interface ToolCallApprovalProps {
  requestId: string
  status: string
  statusLabel: string
  tone: "info" | "risk" | "success" | "warning"
  resolved: boolean
  title: string
  resolvedNote: string
  impactText: string
  actions: ToolCallApprovalAction[]
}

export interface ToolCallStructuredRow {
  label: string
  value: string
}

export interface ToolCallStructuredDetails {
  kind: "background_process" | "task"
  title: string
  description?: string | undefined
  rows: ToolCallStructuredRow[]
  output?: string | undefined
  outputLabel?: string | undefined
}

export interface ToolCallCardProps {
  nodeId: string
  rawToolKind: string
  title: string
  subtitle: string
  status: string
  terminal: boolean
  readPreview: boolean
  model: ToolCardModel
  stats: ToolStat[]
  facts: ToolFact[]
  actions: ToolAction[]
  locations: ToolCallCardLocation[]
  contentHtml: string
  approval?: ToolCallApprovalProps | undefined
  details?: ToolCallStructuredDetails | undefined
}

export interface ToolCallCardCallbacks {
  onOpenLocation(location: { path?: string | undefined; line?: number | undefined }): void
}

export interface MountToolCallCardsOptions extends ToolCallCardCallbacks {
  getProps(nodeId: string): ToolCallCardProps | undefined
  ids?: ReadonlySet<string> | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastToolCallCardPropsJson = new Map<string, string>()

const ICONS: Record<string, LucideIcon> = {
  changed: FileDiff,
  close: X,
  diagnostics: CircleAlert,
  diff: Diff,
  file: FileText,
  open: ExternalLink,
  process: Terminal,
  review: PanelRightOpen,
  search: Search,
  settings: Settings,
  task: Bot,
  terminal: Terminal,
  tool: Wrench
}

type EditLifecycleState = "approval" | "pending" | "running" | "ready" | "complete" | "recorded" | "failed" | "cancelled"

interface EditLifecycleView {
  state: EditLifecycleState
  title: string
  detail: string
}

const EDIT_LIFECYCLE_ICONS: Record<EditLifecycleState, LucideIcon> = {
  approval: ShieldCheck,
  pending: LoaderCircle,
  running: LoaderCircle,
  ready: Diff,
  complete: Check,
  recorded: Check,
  failed: CircleAlert,
  cancelled: CircleX
}

export function ToolCallCard({
  props
}: {
  props: ToolCallCardProps
  callbacks: ToolCallCardCallbacks
}) {
  const shouldShowEditReceipt =
    props.model.kind === "edit" &&
    props.status === "completed" &&
    !props.contentHtml.trim()
  const isActive = props.model.openByDefault || shouldShowEditReceipt
  const [open, setOpen] = React.useState(isActive)
  const Icon = ICONS[props.model.icon] ?? Wrench
  const readPermissionTarget = props.model.kind === "read" && props.approval ? toolTargetLabel(props) : ""
  const displayTitle = readPermissionTarget ? `Read ${readPermissionTarget}` : props.title
  const compactMeta = (props.terminal && props.status === "completed") || props.model.kind === "think" || Boolean(readPermissionTarget)
  const pendingApproval = Boolean(props.approval && !props.approval.resolved)

  React.useEffect(() => {
    setOpen(isActive)
  }, [isActive, props.nodeId])

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <Card
        size="sm"
        data-tool-card=""
        data-open={open ? "" : undefined}
        className={cn(
          "card-body tool-card relative gap-0 overflow-hidden rounded-md border border-border bg-card py-0 text-card-foreground shadow-none ring-0",
          `tool-tone-${props.model.tone}`,
          open ? "is-open" : ""
        )}
      >
          <CollapsibleTrigger
            className="tool-summary grid w-full grid-cols-[auto_minmax(0,1fr)_auto_auto] items-center gap-2 border-0 bg-transparent px-3 py-2 text-left hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
            aria-label={`${open ? "Collapse" : "Expand"} ${displayTitle}`}
          >
            <span className="summary-icon tool-summary-icon inline-flex size-7 shrink-0 items-center justify-center rounded-md border border-border bg-muted text-muted-foreground">
              <Icon data-icon="inline-start" aria-hidden="true" />
            </span>
            <span className="tool-summary-main min-w-0">
              <span className="tool-title block truncate text-sm font-medium">
                {displayTitle}
              </span>
              <span className="tool-subtitle block truncate text-xs text-muted-foreground">
                {props.subtitle}
              </span>
            </span>
            <ToolSummaryMeta compact={compactMeta} model={props.model} status={props.status} />
            <ChevronDown
              aria-hidden="true"
              className={cn(
                "tool-disclosure-icon size-4 shrink-0 text-muted-foreground transition-transform",
                open ? "rotate-180" : ""
              )}
            />
          </CollapsibleTrigger>
          <CollapsibleContent>
            <CardContent
              className={cn(
                "tool-detail space-y-3 px-3 pb-3 pt-1",
                props.readPreview ? "tool-detail-read" : "",
                props.terminal ? "tool-detail-terminal" : ""
              )}
            >
              {props.approval ? (
                <ToolApprovalPanel
                  approval={props.approval}
                  editTarget={props.model.kind === "edit" ? toolTargetLabel(props) || "Workspace edit" : undefined}
                  readTarget={readPermissionTarget || undefined}
                />
              ) : null}
              {pendingApproval ? null : props.terminal ? (
                <HtmlBlock html={props.contentHtml} />
              ) : (
                <ToolDetail props={props} />
              )}
            </CardContent>
          </CollapsibleContent>
        </Card>
    </Collapsible>
  )
}

function ToolDetail({
  props
}: {
  props: ToolCallCardProps
}) {
  const isEdit = props.model.kind === "edit"
  const isThink = props.model.kind === "think"
  const editLifecycle = editLifecycleForTool(props)
  if (isEdit || isThink) {
    return (
      <>
        {editLifecycle ? <ToolEditLifecycle lifecycle={editLifecycle} /> : null}
        {props.contentHtml ? (
          <div
            className={cn("tool-content", isEdit ? "edit-tool-content" : "think-tool-content")}
            dangerouslySetInnerHTML={{ __html: props.contentHtml }}
          />
        ) : null}
      </>
    )
  }

  return (
    <>
      {props.details ? <ToolStructuredDetails details={props.details} /> : null}
      {editLifecycle ? <ToolEditLifecycle lifecycle={editLifecycle} /> : null}
      {props.contentHtml ? (
        <div
          className="tool-content"
          dangerouslySetInnerHTML={{ __html: props.contentHtml }}
        />
      ) : null}
    </>
  )
}

function ToolStructuredDetails({ details }: { details: ToolCallStructuredDetails }) {
  const Icon = details.kind === "task" ? Bot : Terminal
  const hasOutput = Boolean(details.output?.trim())
  return (
    <section
      className={cn("tool-structured-details", `tool-structured-${details.kind}`)}
      aria-label={details.title}
    >
      <div className="tool-structured-heading">
        <Icon className="tool-structured-icon" aria-hidden="true" />
        <span className="tool-structured-copy">
          <strong>{details.title}</strong>
          {details.description ? <span>{details.description}</span> : null}
        </span>
      </div>
      {details.rows.length ? (
        <dl className="tool-structured-fields">
          {details.rows.map((row) => (
            <div className="tool-structured-field" key={`${row.label}:${row.value}`}>
              <dt>{row.label}</dt>
              <dd>{row.value}</dd>
            </div>
          ))}
        </dl>
      ) : null}
      {hasOutput ? (
        <div className="tool-structured-output-block">
          <span className="tool-structured-output-label">{details.outputLabel || "Output"}</span>
          <pre className="tool-structured-output" tabIndex={0}>{details.output}</pre>
        </div>
      ) : null}
    </section>
  )
}

function ToolEditLifecycle({ lifecycle }: { lifecycle: EditLifecycleView }) {
  const Icon = EDIT_LIFECYCLE_ICONS[lifecycle.state]
  const active = lifecycle.state === "approval" || lifecycle.state === "pending" || lifecycle.state === "running"
  const loading = lifecycle.state === "pending" || lifecycle.state === "running"
  return (
    <section
      className={cn("edit-lifecycle", `edit-lifecycle-${lifecycle.state}`)}
      data-edit-lifecycle-state={lifecycle.state}
      aria-label="Edit status"
      role={active ? "status" : undefined}
    >
      <Icon
        className={cn("edit-lifecycle-icon", loading ? "edit-lifecycle-loading-icon" : "")}
        aria-hidden="true"
      />
      <span className="edit-lifecycle-copy">
        <strong>{lifecycle.title}</strong>
        <span>{lifecycle.detail}</span>
      </span>
    </section>
  )
}

function editLifecycleForTool(props: ToolCallCardProps): EditLifecycleView | undefined {
  if (props.model.kind !== "edit") {
    return undefined
  }

  const hasPreview = Boolean(props.contentHtml.trim())
  const target = toolTargetLabel(props)
  const targetPhrase = target ? `for ${target}` : "for the workspace"

  if (props.approval && !props.approval.resolved) {
    return undefined
  }

  if (props.status === "failed") {
    return {
      state: "failed",
      title: "Edit failed",
      detail: `The edit ${targetPhrase} did not finish.`
    }
  }

  if (props.status === "cancelled") {
    return {
      state: "cancelled",
      title: "Edit cancelled",
      detail: "The edit stopped before changes were applied."
    }
  }

  if (props.status === "completed" && hasPreview) {
    return {
      state: "complete",
      title: "Finished editing",
      detail: `Changes ${targetPhrase} are ready below.`
    }
  }

  if (props.status === "completed") {
    return {
      state: "recorded",
      title: "Edit complete",
      detail: target
        ? `No diff preview was returned for ${target}.`
        : "No diff preview was returned."
    }
  }

  if (hasPreview) {
    return {
      state: "ready",
      title: "Review changes",
      detail: `Diff preview ${targetPhrase} is ready below.`
    }
  }

  if (props.status === "pending") {
    return {
      state: "pending",
      title: "Preparing edit",
      detail: `Generating changes ${targetPhrase}.`
    }
  }

  return {
    state: "running",
    title: "Preparing edit",
    detail: `Generating changes ${targetPhrase}.`
  }
}

function toolTargetLabel(props: ToolCallCardProps): string {
  const locationPath = props.locations.find((location) => location.path)?.path
  const factPath = props.facts.find((fact) => fact.label === "Path")?.value
  const subtitlePath = props.subtitle.match(/\bin\s+(.+)$/)?.[1]
  const titlePath = props.title.match(/[^\s()'"]+\.(?:css|go|html?|jsx?|jsonc?|lock|mdx?|mjs|[mc]?ts|py|rs|sh|text|tsx?|txt|xml|ya?ml)\b/i)?.[0]
  return compactEditTargetLabel(String(locationPath || factPath || subtitlePath || titlePath || "").trim())
}

function compactEditTargetLabel(value: string): string {
  if (!value) {
    return ""
  }
  const normalized = value.replace(/^file:\/\//i, "").replace(/\\/g, "/")
  const parts = normalized.split("/").filter(Boolean)
  if (!parts.length) {
    return value
  }
  if (normalized.startsWith("/") || /^[A-Za-z]:\//.test(normalized)) {
    return parts[parts.length - 1] || value
  }
  return parts.length > 2 ? parts.slice(-2).join("/") : normalized
}

function ToolApprovalPanel({
  approval,
  editTarget,
  readTarget
}: {
  approval: ToolCallApprovalProps
  editTarget?: string | undefined
  readTarget?: string | undefined
}) {
  const ApprovalIcon = approvalPanelIcon(approval)
  const isEdit = typeof editTarget === "string"
  const isRead = typeof readTarget === "string"
  const title = isEdit
    ? approval.resolved
      ? "Edit decision recorded"
      : "Approve edit?"
    : isRead
      ? approval.resolved
        ? "Read decision recorded"
        : "Allow read?"
    : approval.resolved
      ? "Permission decided"
      : approval.title
  const detail = isEdit
    ? approval.resolved
      ? approval.resolvedNote
      : editTarget || "Workspace edit"
    : isRead
      ? approval.resolved
        ? approval.resolvedNote
        : readTarget || "Workspace file"
    : approval.resolved
      ? approval.resolvedNote
      : approval.impactText
  return (
    <section
      className={cn(
        "tool-approval",
        isEdit ? "tool-approval-edit" : "",
        isRead ? "tool-approval-read" : "",
        `approval-tone-${approval.tone}`,
        approval.resolved ? "tool-approval-resolved" : ""
      )}
      aria-label="Permission request"
    >
      <div className="tool-approval-heading">
        <ApprovalIcon className="tool-approval-icon" aria-hidden="true" />
        <div className="tool-approval-copy">
          <span className="tool-approval-title">
            {title}
          </span>
          <p className={approval.resolved ? "approval-resolved-note" : "approval-impact"}>{detail}</p>
        </div>
      </div>
      {approval.resolved ? null : <ToolApprovalDecision approval={approval} />}
    </section>
  )
}

function ToolApprovalDecision({
  approval
}: {
  approval: ToolCallApprovalProps
}) {
  const approveActions = approval.actions.filter((action) => action.kind === "approve")
  const rejectAction = approval.actions.find((action) => action.kind === "reject")
  return (
    <div className="approval-decision" role="group" aria-label="Permission decision">
      {approveActions.length ? (
        <ButtonGroup className="approval-option-list" aria-label="Approval options">
          {approveActions.map((action) => (
            <ToolApprovalButton
              key={`${action.kind}-${action.optionId ?? action.label}`}
              action={action}
              requestId={approval.requestId}
            />
          ))}
        </ButtonGroup>
      ) : null}
      {rejectAction ? (
        <ToolApprovalButton action={rejectAction} requestId={approval.requestId} />
      ) : null}
    </div>
  )
}

function ToolApprovalButton({
  action,
  requestId
}: {
  action: ToolCallApprovalAction
  requestId: string
}) {
  const isReject = action.kind === "reject"
  const isPrimaryApprove = isPrimaryApprovalAction(action)
  const variant = isReject || action.tone === "risk" ? "destructive" : isPrimaryApprove ? "default" : "outline"
  const ApprovalIcon = approvalActionIcon(action)
  return (
    <Button
      type="button"
      data-action={action.kind}
      data-request-id={requestId}
      data-option-id={action.optionId}
      title={action.description}
      aria-label={`${action.label}. ${action.description}`}
      disabled={action.disabled}
      variant={variant}
      size="sm"
      className={cn(
        isReject ? "danger approval-reject" : "approval-option",
        isReject ? "" : `approval-option-${action.tone}`,
        isPrimaryApprove ? "primary" : ""
      )}
    >
      <ApprovalIcon data-icon="inline-start" aria-hidden="true" />
      <span className="approval-decision-copy">
        <span>{action.label}</span>
      </span>
    </Button>
  )
}

function approvalPanelIcon(approval: ToolCallApprovalProps): LucideIcon {
  if (approval.status === "completed") {
    return Check
  }
  if (approval.status === "cancelled" || approval.status === "failed") {
    return CircleX
  }
  return ShieldCheck
}

function approvalActionIcon(action: ToolCallApprovalAction): LucideIcon {
  if (action.kind === "reject") {
    return CircleX
  }
  const value = `${action.label} ${action.optionId ?? ""}`.toLowerCase()
  if (value.includes("always")) {
    return ShieldCheck
  }
  return Check
}

function isPrimaryApprovalAction(action: ToolCallApprovalAction): boolean {
  if (action.kind !== "approve" || action.tone === "risk") {
    return false
  }
  const value = `${action.label} ${action.optionId ?? ""}`.toLowerCase()
  return !/\b(always|forever|persist)\b/.test(value)
}

function ToolSummaryMeta({
  compact,
  model,
  status
}: {
  compact: boolean
  model: ToolCardModel
  status: string
}) {
  if (compact) {
    return status === "completed" ? null : (
      <span className="tool-summary-meta">
        <ToolMetaHover
          title="Tool status"
          label={model.statusLabel}
          description={toolStatusDescription(status, model.statusLabel)}
        >
          <ToolMetaIconBadge
            className={cn("tool-status", `tool-status-${status}`)}
            icon={statusIcon(status, model.statusLabel)}
            label={model.statusLabel}
          />
        </ToolMetaHover>
      </span>
    )
  }

  return (
    <span className="tool-summary-meta flex min-w-0 flex-wrap justify-end gap-1">
      <ToolMetaHover
        title="Tool operation"
        label={model.operationLabel}
        description={toolOperationDescription(model)}
      >
        <ToolMetaIconBadge
          className={cn("tool-kind", `tool-kind-${model.tone}`)}
          icon={operationIcon(model)}
          label={model.operationLabel}
        />
      </ToolMetaHover>
      {model.riskTone === "ok" ? null : (
        <ToolMetaHover
          title="Tool risk"
          label={model.riskLabel}
          description={toolRiskDescription(model.riskTone, model.riskLabel)}
        >
          <ToolMetaIconBadge
            className={cn("tool-risk-badge", `tool-risk-badge-${model.riskTone}`)}
            icon={riskIcon(model.riskTone, model.riskLabel)}
            label={model.riskLabel}
            variant={model.riskTone === "risk" ? "destructive" : "secondary"}
          />
        </ToolMetaHover>
      )}
      {status === "completed" ? null : (
        <ToolMetaHover
          title="Tool status"
          label={model.statusLabel}
          description={toolStatusDescription(status, model.statusLabel)}
        >
          <ToolMetaIconBadge
            className={cn("tool-status", `tool-status-${status}`)}
            icon={statusIcon(status, model.statusLabel)}
            label={model.statusLabel}
          />
        </ToolMetaHover>
      )}
    </span>
  )
}

function ToolMetaIconBadge({
  className,
  icon: Icon,
  label,
  variant = "outline"
}: {
  className: string
  icon: LucideIcon
  label: string
  variant?: React.ComponentProps<typeof Badge>["variant"]
}) {
  return (
    <Badge
      aria-label={label}
      className={cn("tool-meta-icon-badge", className)}
      title={label}
      variant={variant}
    >
      <Icon data-icon="inline-start" aria-hidden="true" />
    </Badge>
  )
}

function ToolMetaHover({
  title,
  label,
  description,
  children
}: {
  title: string
  label: string
  description: string
  children: React.ReactNode
}) {
  return (
    <HoverCard>
      <HoverCardTrigger
        className="tool-meta-hover-trigger"
        delay={100}
        closeDelay={120}
        render={<span />}
      >
        {children}
      </HoverCardTrigger>
      <HoverCardContent className="tool-meta-hover-card" side="top" align="end">
        <div className="tool-meta-hover-content">
          <span className="tool-meta-hover-title">{title}</span>
          <strong>{label}</strong>
          <span>{description}</span>
        </div>
      </HoverCardContent>
    </HoverCard>
  )
}

function HtmlBlock({ html }: { html: string }) {
  return <div dangerouslySetInnerHTML={{ __html: html }} />
}

function operationIcon(model: ToolCardModel): LucideIcon {
  return ICONS[model.icon] ?? Wrench
}

function riskIcon(tone: ToolCardModel["riskTone"], label: string): LucideIcon {
  if (tone === "risk") {
    return CircleAlert
  }
  if (/approval/i.test(label)) {
    return ShieldCheck
  }
  return CircleAlert
}

function statusIcon(status: string, label: string): LucideIcon {
  if (status === "completed") {
    return Check
  }
  if (status === "failed" || status === "cancelled") {
    return CircleX
  }
  if (/decision|approval/i.test(label)) {
    return CircleAlert
  }
  if (status === "pending" || status === "in_progress") {
    return Clock3
  }
  return CircleAlert
}

function toolOperationDescription(model: ToolCardModel): string {
  switch (model.tone) {
    case "change":
      return "This tool can change workspace files or reviewable patches."
    case "file":
      return "This tool reads or previews workspace resources."
    case "query":
      return "This tool searches or fetches information for the agent."
    case "terminal":
      return "This tool runs a command and reports terminal output."
    case "agent":
      return "This tool delegates work to another agent context."
    case "risk":
      return "This tool needs extra attention before you trust the result."
    default:
      return `Provider tool classified as ${model.kind}.`
  }
}

function toolRiskDescription(tone: ToolCardModel["riskTone"], label: string): string {
  switch (tone) {
    case "risk":
      return `${label} can affect files, commands, or state and should be inspected.`
    case "warning":
      return `${label} means the tool includes evidence worth reviewing.`
    default:
      return "No unusual risk signal was detected for this tool call."
  }
}

function toolStatusDescription(status: string, label: string): string {
  switch (status) {
    case "pending":
      return "The provider has announced this tool call but it has not finished yet."
    case "in_progress":
      return "The tool call is still running."
    case "failed":
      return "The tool call failed."
    case "cancelled":
      return "The tool call was cancelled before completion."
    default:
      return `The current tool status is ${label}.`
  }
}

export function mountToolCallCards(options: MountToolCallCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-tool-call-card-root]").forEach((element) => {
    const nodeId = element.dataset.toolNodeId
    if (!nodeId) {
      return
    }
    if (options.ids && !options.ids.has(nodeId)) {
      activeIds.add(nodeId)
      return
    }
    const props = options.getProps(nodeId)
    if (!props) {
      return
    }
    activeIds.add(nodeId)
    const currentJson = JSON.stringify(props)
    const mounted = mountedRoots.get(nodeId)
    if (
      currentJson === lastToolCallCardPropsJson.get(nodeId) &&
      mounted?.element === element &&
      mounted.element.isConnected
    ) {
      return
    }
    lastToolCallCardPropsJson.set(nodeId, currentJson)
    const root = mounted?.element === element ? mounted.root : createRoot(element)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mountedRoots.set(nodeId, { element, root })
    }
    flushSync(() => {
      root.render(
        <ToolCallCard
          props={props}
          callbacks={{ onOpenLocation: options.onOpenLocation }}
        />
      )
    })
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (activeIds.has(nodeId) && mounted.element.isConnected) {
      return
    }
    mounted.root.unmount()
    mountedRoots.delete(nodeId)
    lastToolCallCardPropsJson.delete(nodeId)
  })
}

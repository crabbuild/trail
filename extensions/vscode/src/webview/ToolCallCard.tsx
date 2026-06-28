import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"
import {
  ChevronDown,
  CircleAlert,
  Diff,
  ExternalLink,
  FileDiff,
  FileText,
  PanelRightOpen,
  Search,
  Settings,
  Terminal,
  Wrench,
  X,
  type LucideIcon
} from "lucide-react"

import { Badge } from "@/webview/components/ui/badge"
import {
  Breadcrumb,
  BreadcrumbEllipsis,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator
} from "@/webview/components/ui/breadcrumb"
import { Card, CardContent } from "@/webview/components/ui/card"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger
} from "@/webview/components/ui/collapsible"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuTrigger
} from "@/webview/components/ui/context-menu"
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger
} from "@/webview/components/ui/hover-card"
import { Separator } from "@/webview/components/ui/separator"
import { cn } from "@/webview/lib/utils"
import { InlineActions, type InlineAction, type InlineActionTone } from "./InlineActions"
import { RawDetails, type RawDetailsView } from "./RawDetails"
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
  rawDetails?: RawDetailsView | undefined
}

export interface ToolCallCardCallbacks {
  onOpenLocation(location: { path?: string | undefined; line?: number | undefined }): void
}

export interface MountToolCallCardsOptions extends ToolCallCardCallbacks {
  getProps(nodeId: string): ToolCallCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()

const ICONS: Record<string, LucideIcon> = {
  changed: FileDiff,
  close: X,
  diagnostics: CircleAlert,
  diff: Diff,
  file: FileText,
  open: ExternalLink,
  review: PanelRightOpen,
  search: Search,
  settings: Settings,
  terminal: Terminal,
  tool: Wrench
}

export function ToolCallCard({
  props,
  callbacks
}: {
  props: ToolCallCardProps
  callbacks: ToolCallCardCallbacks
}) {
  const [open, setOpen] = React.useState(props.model.openByDefault)
  const [rawOpen, setRawOpen] = React.useState(Boolean(props.rawDetails?.defaultOpen))
  const cardRef = React.useRef<HTMLDivElement>(null)
  const Icon = ICONS[props.model.icon] ?? Wrench
  const compactMeta = props.terminal && props.status === "completed"

  React.useEffect(() => {
    setRawOpen(Boolean(props.rawDetails?.defaultOpen))
  }, [props.rawDetails?.defaultOpen, props.rawDetails?.id])

  const handleAction = React.useCallback(
    (action: ToolAction, event: React.MouseEvent<HTMLElement>) => {
      event.preventDefault()
      event.stopPropagation()
      if (action.kind === "openLocation") {
        callbacks.onOpenLocation({ path: action.path, line: action.line })
        return
      }
      setOpen(true)
      if (action.kind === "inspectDetails") {
        setRawOpen(true)
      }
      window.requestAnimationFrame(() => {
        if (action.kind === "focusDiff") {
          focusToolDiff(cardRef.current)
          return
        }
        inspectToolDetails(cardRef.current)
      })
    },
    [callbacks]
  )

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <ToolContextMenu actions={props.actions} onAction={handleAction}>
        <Card
          ref={cardRef}
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
            aria-label={`${open ? "Collapse" : "Expand"} ${props.title}`}
          >
            <span className="summary-icon tool-summary-icon inline-flex size-7 shrink-0 items-center justify-center rounded-md border border-border bg-muted text-muted-foreground">
              <Icon data-icon="inline-start" aria-hidden="true" />
            </span>
            <span className="tool-summary-main min-w-0">
              <span className="tool-title block truncate text-sm font-medium">
                {props.title}
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
              {props.terminal ? (
                <HtmlBlock html={props.contentHtml} />
              ) : (
                <ToolDetail
                  props={props}
                  rawOpen={rawOpen}
                  setRawOpen={setRawOpen}
                  onAction={handleAction}
                />
              )}
            </CardContent>
          </CollapsibleContent>
        </Card>
      </ToolContextMenu>
    </Collapsible>
  )
}

function ToolDetail({
  props,
  rawOpen,
  setRawOpen,
  onAction
}: {
  props: ToolCallCardProps
  rawOpen: boolean
  setRawOpen(open: boolean): void
  onAction(action: ToolAction, event: React.MouseEvent<HTMLElement>): void
}) {
  return (
    <>
      {props.readPreview ? null : (
        <>
          <ToolActionBar actions={props.actions} onAction={onAction} />
          <ToolEvidenceStrip stats={props.stats} />
          <ToolFacts facts={props.facts} />
          <ToolLocations locations={props.locations} />
        </>
      )}
      {props.contentHtml ? (
        <div
          className="tool-content"
          dangerouslySetInnerHTML={{ __html: props.contentHtml }}
        />
      ) : null}
      {props.rawDetails ? (
        <RawDetails
          details={props.rawDetails}
          open={rawOpen}
          onOpenChange={setRawOpen}
        />
      ) : null}
    </>
  )
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
          <Badge className={cn("tool-status min-w-0 max-w-full truncate", `tool-status-${status}`)} variant="outline">
            {model.statusLabel}
          </Badge>
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
        <Badge className={cn("tool-kind min-w-0 max-w-full truncate", `tool-kind-${model.tone}`)} variant="outline">
          {model.operationLabel}
        </Badge>
      </ToolMetaHover>
      {model.riskTone === "ok" ? null : (
        <ToolMetaHover
          title="Tool risk"
          label={model.riskLabel}
          description={toolRiskDescription(model.riskTone, model.riskLabel)}
        >
          <Badge
            className={cn("tool-risk-badge min-w-0 max-w-full truncate", `tool-risk-badge-${model.riskTone}`)}
            variant={model.riskTone === "risk" ? "destructive" : "secondary"}
          >
            {riskShortLabel(model.riskLabel)}
          </Badge>
        </ToolMetaHover>
      )}
      {status === "completed" ? null : (
        <ToolMetaHover
          title="Tool status"
          label={model.statusLabel}
          description={toolStatusDescription(status, model.statusLabel)}
        >
          <Badge className={cn("tool-status min-w-0 max-w-full truncate", `tool-status-${status}`)} variant="outline">
            {model.statusLabel}
          </Badge>
        </ToolMetaHover>
      )}
    </span>
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

function ToolActionBar({
  actions,
  onAction
}: {
  actions: ToolAction[]
  onAction(action: ToolAction, event: React.MouseEvent<HTMLElement>): void
}) {
  if (!actions.length) {
    return null
  }
  return (
    <InlineActions
      props={{
        id: "tool-card-actions",
        className: "tool-card-actions",
        ariaLabel: "Tool actions",
        actions: actions.map((action, index) => toolInlineAction(action, index)),
        onAction: (inlineAction, event) => {
          const index = inlineAction.data?.["tool-action-index"]
          const action = typeof index === "string" ? actions[Number.parseInt(index, 10)] : undefined
          if (action) {
            onAction(action, event)
          }
        }
      }}
    />
  )
}

function toolInlineAction(action: ToolAction, index: number): InlineAction {
  const Icon = ICONS[actionIcon(action.kind)] ?? Wrench
  return {
    action: toolActionDomName(action.kind),
    label: action.label,
    tone: toolActionTone(action.tone),
    ariaLabel: `${action.label}. ${action.description}`,
    tooltip: action.description,
    icon: <Icon data-icon="inline-start" aria-hidden="true" />,
    className: "tool-card-action",
    data: {
      "tool-action-index": String(index),
      path: action.path,
      line: typeof action.line === "number" ? String(action.line) : undefined
    }
  }
}

function toolActionTone(tone: ToolAction["tone"]): InlineActionTone {
  if (tone === "primary" || tone === "danger") {
    return tone
  }
  return "default"
}

function ToolContextMenu({
  actions,
  children,
  onAction
}: {
  actions: ToolAction[]
  children: React.ReactNode
  onAction(action: ToolAction, event: React.MouseEvent<HTMLElement>): void
}) {
  if (!actions.length) {
    return <>{children}</>
  }
  return (
    <ContextMenu>
      <ContextMenuTrigger
        className="tool-context-trigger select-text"
        render={<div />}
      >
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent className="tool-context-menu" side="right" align="start">
        <ContextMenuGroup>
          <ContextMenuLabel>Tool actions</ContextMenuLabel>
          {actions.map((action, index) => {
            const Icon = ICONS[actionIcon(action.kind)] ?? Wrench
            return (
              <ContextMenuItem
                key={`${action.kind}-${action.path ?? ""}-${index}`}
                className="tool-context-menu-item"
                data-action={toolActionDomName(action.kind)}
                data-path={action.path}
                data-line={action.line}
                variant={action.tone === "danger" ? "destructive" : "default"}
                onClick={(event) => onAction(action, event)}
              >
                <Icon aria-hidden="true" />
                <span>{action.label}</span>
              </ContextMenuItem>
            )
          })}
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  )
}

function ToolEvidenceStrip({ stats }: { stats: ToolStat[] }) {
  if (!stats.length) {
    return null
  }
  return (
    <div className="tool-evidence-strip flex flex-wrap gap-1" aria-label="Tool evidence">
      {stats.map((stat) => (
        <Badge
          key={`${stat.label}-${stat.value}`}
          className={cn(
            "tool-stat min-w-0 max-w-full items-baseline gap-1",
            `tool-stat-${stat.tone}`
          )}
          variant="outline"
        >
          <b className="font-mono tabular-nums">{stat.value}</b>
          <span className="truncate text-muted-foreground">{stat.label}</span>
        </Badge>
      ))}
    </div>
  )
}

function ToolFacts({ facts }: { facts: ToolFact[] }) {
  if (!facts.length) {
    return null
  }
  return (
    <div className="tool-facts flex flex-wrap gap-1" aria-label="Tool facts">
      {facts.map((fact) => (
        <Badge
          key={`${fact.label}-${fact.value}`}
          className="tool-fact min-w-0 max-w-full items-baseline gap-1"
          variant="secondary"
        >
          <span className="tool-fact-label truncate text-muted-foreground">
            {fact.label}
          </span>
          <Separator className="tool-fact-separator" orientation="vertical" />
          <span className="tool-fact-value min-w-0 overflow-auto font-mono text-xs">
            {fact.value}
          </span>
        </Badge>
      ))}
    </div>
  )
}

function ToolLocations({ locations }: { locations: ToolCallCardLocation[] }) {
  if (!locations.length) {
    return null
  }
  return (
    <div className="tool-locations flex flex-wrap gap-1" aria-label="Tool locations">
      {locations.map((location, index) => (
        <ToolLocationBreadcrumb
          key={`${location.path}-${location.line ?? ""}-${index}`}
          location={location}
        />
      ))}
    </div>
  )
}

function ToolLocationBreadcrumb({ location }: { location: ToolCallCardLocation }) {
  const crumb = locationBreadcrumb(location.path)
  const lineLabel = typeof location.line === "number" ? `:${location.line}` : ""

  return (
    <Breadcrumb
      className="resource-chip tool-location-breadcrumb min-w-0 max-w-full justify-start"
      aria-label={`Tool location ${location.path}${lineLabel}`}
    >
      <BreadcrumbList className="tool-location-breadcrumb-list flex-nowrap gap-1 text-xs">
        {crumb.collapsed ? (
          <>
            <BreadcrumbItem className="min-w-0">
              <BreadcrumbEllipsis className="tool-location-ellipsis" />
            </BreadcrumbItem>
            <BreadcrumbSeparator className="tool-location-separator" />
          </>
        ) : null}
        {crumb.parts.map((part, index) => {
          const last = index === crumb.parts.length - 1
          return (
            <React.Fragment key={`${part}-${index}`}>
              {index > 0 ? <BreadcrumbSeparator className="tool-location-separator" /> : null}
              <BreadcrumbItem className="min-w-0">
                {last ? (
                  <BreadcrumbPage className="tool-location-page truncate">
                    {part}
                  </BreadcrumbPage>
                ) : (
                  <BreadcrumbLink
                    className="tool-location-segment truncate"
                    render={<span />}
                  >
                    {part}
                  </BreadcrumbLink>
                )}
              </BreadcrumbItem>
            </React.Fragment>
          )
        })}
        {lineLabel ? (
          <>
            <BreadcrumbSeparator className="tool-location-separator" />
            <BreadcrumbItem className="min-w-0">
              <Badge className="tool-location-line" variant="secondary">
                {lineLabel}
              </Badge>
            </BreadcrumbItem>
          </>
        ) : null}
      </BreadcrumbList>
    </Breadcrumb>
  )
}

function HtmlBlock({ html }: { html: string }) {
  return <div dangerouslySetInnerHTML={{ __html: html }} />
}

function focusToolDiff(card: HTMLElement | null): void {
  const diff = card?.querySelector<HTMLElement>(".diff-preview")
  if (!diff) {
    return
  }
  diff.setAttribute("tabindex", "-1")
  diff.scrollIntoView({ block: "nearest", inline: "nearest" })
  diff.focus()
}

function inspectToolDetails(card: HTMLElement | null): void {
  const rawSummary = card?.querySelector<HTMLElement>(".raw-summary")
  if (rawSummary) {
    rawSummary.scrollIntoView({ block: "nearest", inline: "nearest" })
    rawSummary.focus()
    return
  }
  const fallback = card?.querySelector<HTMLElement>(
    ".tool-content, .tool-evidence-strip, .tool-summary"
  )
  fallback?.setAttribute("tabindex", "-1")
  fallback?.scrollIntoView({ block: "nearest", inline: "nearest" })
  fallback?.focus()
}

function actionIcon(kind: ToolAction["kind"]): string {
  switch (kind) {
    case "openLocation":
      return "open"
    case "focusDiff":
      return "diff"
    case "inspectDetails":
      return "diagnostics"
    default:
      return "tool"
  }
}

function toolActionDomName(kind: ToolAction["kind"]): string {
  switch (kind) {
    case "focusDiff":
      return "focusToolDiff"
    case "inspectDetails":
      return "inspectToolDetails"
    default:
      return kind
  }
}

function riskShortLabel(label: string): string {
  if (label === "Workspace change") {
    return "change"
  }
  if (label === "Needs inspection") {
    return "inspect"
  }
  return label.toLowerCase()
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
      return "The tool call failed; inspect details for redacted input and output."
    case "cancelled":
      return "The tool call was cancelled before completion."
    default:
      return `The current tool status is ${label}.`
  }
}

function locationBreadcrumb(path: string): { collapsed: boolean; parts: string[] } {
  const normalized = path.replace(/\\/g, "/")
  const parts = normalized.split("/").filter(Boolean)
  if (!parts.length) {
    return { collapsed: false, parts: [path] }
  }
  return {
    collapsed: parts.length > 2,
    parts: parts.slice(-2)
  }
}

export function mountToolCallCards(options: MountToolCallCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-tool-call-card-root]").forEach((element) => {
    const nodeId = element.dataset.toolNodeId
    if (!nodeId) {
      return
    }
    const props = options.getProps(nodeId)
    if (!props) {
      return
    }
    activeIds.add(nodeId)
    const mounted = mountedRoots.get(nodeId)
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
  })
}

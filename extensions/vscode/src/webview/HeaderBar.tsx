import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import { ButtonGroup } from "@/webview/components/ui/button-group"
import { Card, CardContent } from "@/webview/components/ui/card"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/webview/components/ui/collapsible"
import { cn } from "@/webview/lib/utils"
import { useFloatingDisclosure } from "./floatingDisclosure"
import type { LaneMapDrawerProps } from "./LaneMapDrawer"
import type {
  ToolbarAction,
  ToolbarCapability,
  ToolbarChip,
  ToolbarModel,
  ToolbarRunState
} from "./toolbarModel"

export interface HeaderUsageView {
  used: number
  size: number
}

export interface HeaderIconAction {
  action: string
  label: string
  iconHtml: string
  disabled?: boolean | undefined
  active?: boolean | undefined
  ariaControls?: string | undefined
  ariaExpanded?: boolean | undefined
  ariaPressed?: boolean | undefined
}

export interface HeaderBarProps {
  id: string
  title: string
  status?: string | undefined
  showStatusPill: boolean
  toolbar: ToolbarModel
  usage?: HeaderUsageView | undefined
  detailsIconHtml: string
  capabilitiesIconHtml: string
  primaryActionIconHtml: string
  laneMap?: LaneMapDrawerProps | undefined
  inspectActions: HeaderIconAction[]
  runActions: HeaderIconAction[]
}

export interface MountHeaderBarsOptions {
  getProps(id: string): HeaderBarProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const LazyLaneMapDrawer = React.lazy(async () => {
  const module = await import("./LaneMapDrawer.js")
  return { default: module.LaneMapDrawer }
})

const mountedRoots = new Map<string, MountedRoot>()
const lastHeaderBarPropsJson = new Map<string, string>()

export function HeaderBar({ props }: { props: HeaderBarProps }) {
  return (
    <>
      <div className="header-main">
        <div className="title-block">
          <div className="eyebrow">
            <ToolbarRunStateBadge runState={props.toolbar.runState} />
            {props.showStatusPill && props.status ? (
              <Badge
                className={cn("status", `status-${classSuffix(props.status)}`)}
                variant="outline"
              >
                {props.status}
              </Badge>
            ) : null}
          </div>
          <h1>{props.title}</h1>
        </div>
      </div>
      <div className="header-actions" aria-label="Task actions">
        <ButtonGroup className="header-action-group header-action-context" aria-label="Primary task action">
          <ToolbarActionButton
            action={props.toolbar.primaryAction}
            iconHtml={props.primaryActionIconHtml}
          />
        </ButtonGroup>
        <ButtonGroup className="header-action-group" aria-label="Inspect task">
          <HeaderDetails
            toolbar={props.toolbar}
            usage={props.usage}
            iconHtml={props.detailsIconHtml}
          />
          <ToolbarCapabilities
            toolbar={props.toolbar}
            iconHtml={props.capabilitiesIconHtml}
            reviewAction={props.inspectActions.find((action) => action.action === "toggleReview")}
            settingsAction={props.inspectActions.find((action) => action.action === "openSettings")}
          />
          {props.laneMap ? <LaneMapToolbarButton laneMap={props.laneMap} /> : null}
          {props.inspectActions.map((action) => (
            <HeaderIconButton key={action.action} action={action} />
          ))}
        </ButtonGroup>
        <ButtonGroup className="header-action-group" aria-label="Run controls">
          {props.runActions.map((action) => (
            <HeaderIconButton key={action.action} action={action} />
          ))}
        </ButtonGroup>
      </div>
    </>
  )
}

function LaneMapToolbarButton({ laneMap }: { laneMap: LaneMapDrawerProps }) {
  const [open, setOpen] = React.useState(false)
  return (
    <>
      <Button
        type="button"
        className={open ? "active" : undefined}
        data-header-icon-only="true"
        data-lane-map-trigger="true"
        title="Lane map"
        aria-label="Open lane map"
        aria-expanded={open}
        aria-controls="lane-map-drawer"
        onClick={() => setOpen(true)}
        variant={open ? "outline" : "ghost"}
        size="icon-sm"
      >
        <span
          data-icon="inline-start"
          dangerouslySetInnerHTML={{ __html: laneMap.mapIconHtml }}
        />
      </Button>
      {open ? (
        <React.Suspense fallback={null}>
          <LazyLaneMapDrawer open={open} onOpenChange={setOpen} props={laneMap} />
        </React.Suspense>
      ) : null}
    </>
  )
}

function HeaderDetails({
  iconHtml,
  toolbar,
  usage
}: {
  iconHtml: string
  toolbar: ToolbarModel
  usage?: HeaderUsageView | undefined
}) {
  const disclosure = useFloatingDisclosure()
  return (
    <Collapsible
      className="header-details"
      data-floating-open={disclosure.open ? "true" : undefined}
      open={disclosure.open}
      onOpenChange={disclosure.setOpen}
      ref={disclosure.rootRef}
    >
      <CollapsibleTrigger
        className="header-details-trigger"
        title="Show agent details"
        ref={disclosure.triggerRef}
      >
        <span
          data-icon="inline-start"
          dangerouslySetInnerHTML={{ __html: iconHtml }}
        />
        <span className="sr-only">Agent details</span>
      </CollapsibleTrigger>
      <CollapsibleContent keepMounted>
        <Card className="header-detail-body" size="sm">
          <CardContent className="header-detail-content">
            {usage ? (
              <div className="header-detail-usage">
                <ContextMeter usage={usage} />
              </div>
            ) : null}
            <div className="header-detail-chips">
              {toolbar.statusChips.map((chip) => (
                <ToolbarChipBadge key={chip.id} chip={chip} />
              ))}
            </div>
          </CardContent>
        </Card>
      </CollapsibleContent>
    </Collapsible>
  )
}

function ToolbarRunStateBadge({ runState }: { runState: ToolbarRunState }) {
  return (
    <Badge
      className={cn("toolbar-run-state", `toolbar-run-${runState.tone}`)}
      title={runState.detail}
      variant="outline"
    >
      <span className="toolbar-run-dot" aria-hidden="true" />
      <span>{runState.label}</span>
    </Badge>
  )
}

function ToolbarChipBadge({ chip }: { chip: ToolbarChip }) {
  const fullLabel = `${chip.label}: ${chip.value}`
  return (
    <Badge
      className={cn("toolbar-chip", `toolbar-chip-${chip.tone}`)}
      title={fullLabel}
      aria-label={chip.accessibilityLabel}
      variant="outline"
    >
      <span>{chip.label}</span>
      <strong>{chip.displayValue}</strong>
    </Badge>
  )
}

function ToolbarCapabilities({
  iconHtml,
  reviewAction,
  settingsAction,
  toolbar
}: {
  iconHtml: string
  reviewAction?: HeaderIconAction | undefined
  settingsAction?: HeaderIconAction | undefined
  toolbar: ToolbarModel
}) {
  const disclosure = useFloatingDisclosure()
  const label = `CrabDB capabilities: ${toolbar.capabilitySummary}`
  return (
    <Collapsible
      className="toolbar-capabilities"
      data-floating-open={disclosure.open ? "true" : undefined}
      open={disclosure.open}
      onOpenChange={disclosure.setOpen}
      ref={disclosure.rootRef}
    >
      <CollapsibleTrigger
        className="toolbar-capabilities-trigger"
        title={label}
        aria-label={label}
        ref={disclosure.triggerRef}
      >
        <span
          data-icon="inline-start"
          dangerouslySetInnerHTML={{ __html: iconHtml }}
        />
        <span className="sr-only">CrabDB capabilities</span>
        <b>{toolbar.capabilitySummary}</b>
      </CollapsibleTrigger>
      <CollapsibleContent keepMounted>
        <Card className="toolbar-capability-grid" size="sm" aria-label="CrabDB capability matrix">
          <CardContent className="toolbar-capability-content">
            <ToolbarCapabilityGroup toolbar={toolbar} group="workflow" label="Workflow" />
            <ToolbarCapabilityGroup toolbar={toolbar} group="input" label="Input" />
            <div className="toolbar-capability-actions" aria-label="Capability actions">
              {reviewAction ? <CapabilityActionButton action={reviewAction} /> : null}
              {settingsAction ? <CapabilityActionButton action={settingsAction} /> : null}
            </div>
          </CardContent>
        </Card>
      </CollapsibleContent>
    </Collapsible>
  )
}

function ToolbarCapabilityGroup({
  group,
  label,
  toolbar
}: {
  group: ToolbarCapability["group"]
  label: string
  toolbar: ToolbarModel
}) {
  const capabilities = toolbar.capabilities.filter((capability) => capability.group === group)
  if (!capabilities.length) {
    return null
  }
  return (
    <section
      className={cn("toolbar-capability-section", `toolbar-capability-section-${group}`)}
      aria-label={`${label} capabilities`}
    >
      <span className="toolbar-capability-group-label">{label}</span>
      <div className="toolbar-capability-list">
        {capabilities.map((capability) => (
          <Card
            key={capability.id}
            size="sm"
            className={cn("toolbar-capability", capability.enabled ? "on" : "off")}
            data-capability={capability.id}
            aria-label={`${capability.label}: ${capability.enabled ? "ready" : "unavailable"}`}
          >
            <strong>{capability.label}</strong>
            <small>{capability.detail}</small>
          </Card>
        ))}
      </div>
    </section>
  )
}

function ToolbarActionButton({
  action,
  iconHtml
}: {
  action: ToolbarAction
  iconHtml: string
}) {
  const variant = action.tone === "danger" ? "destructive" : action.tone === "primary" ? "default" : "outline"
  return (
    <Button
      type="button"
      className={cn(
        "toolbar-action-button",
        action.tone === "primary" ? "primary" : "",
        action.tone === "danger" ? "danger" : ""
      )}
      data-action={action.action}
      title={action.detail}
      aria-label={`${action.label}. ${action.detail}`}
      aria-disabled={action.disabled ? "true" : undefined}
      disabled={action.disabled}
      variant={variant}
      size="sm"
    >
      <span
        data-icon="inline-start"
        dangerouslySetInnerHTML={{ __html: iconHtml }}
      />
      <span>{action.label}</span>
    </Button>
  )
}

function HeaderIconButton({ action }: { action: HeaderIconAction }) {
  return (
    <Button
      type="button"
      className={action.active ? "active" : undefined}
      data-action={action.action}
      data-header-icon-only="true"
      title={action.label}
      aria-label={action.label}
      aria-pressed={action.ariaPressed}
      aria-expanded={action.ariaExpanded}
      aria-controls={action.ariaControls}
      disabled={action.disabled}
      variant={action.active ? "outline" : "ghost"}
      size="icon-sm"
    >
      <span
        data-icon="inline-start"
        dangerouslySetInnerHTML={{ __html: action.iconHtml }}
      />
    </Button>
  )
}

function CapabilityActionButton({ action }: { action: HeaderIconAction }) {
  return (
    <Button
      type="button"
      data-action={action.action}
      title={action.label}
      disabled={action.disabled}
      variant="outline"
      size="sm"
    >
      <span
        data-icon="inline-start"
        dangerouslySetInnerHTML={{ __html: action.iconHtml }}
      />
      <span>{action.label}</span>
    </Button>
  )
}

function ContextMeter({ usage }: { usage: HeaderUsageView }) {
  const pct = usage.size > 0 ? Math.min(100, Math.round((usage.used / usage.size) * 100)) : 0
  const tone = pct >= 90 ? "risk" : pct >= 70 ? "review" : "ok"
  return (
    <div className="context-meter" title={`${usage.used} / ${usage.size} tokens`}>
      <span>{pct}%</span>
      <progress
        className={cn("meter", tone)}
        value={pct}
        max={100}
        aria-label={`Context usage ${pct}%`}
      />
    </div>
  )
}

function classSuffix(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9_-]/g, "-")
}

export function mountHeaderBars(options: MountHeaderBarsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-header-bar-root]").forEach((element) => {
    const id = element.dataset.headerBarId
    if (!id) {
      return
    }
    const props = options.getProps(id)
    if (!props) {
      return
    }
    activeIds.add(id)
    let mounted = mountedRoots.get(id)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(id, mounted)
    }
    const currentJson = JSON.stringify(props)
    if (currentJson === lastHeaderBarPropsJson.get(id)) {
      return
    }
    lastHeaderBarPropsJson.set(id, currentJson)
    mounted.root.render(<HeaderBar props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
      lastHeaderBarPropsJson.delete(id)
    }
  })
}

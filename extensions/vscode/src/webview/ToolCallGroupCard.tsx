import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"
import { Check, ChevronDown, CircleAlert, CircleX, Clock3, Wrench, type LucideIcon } from "lucide-react"

import { Card, CardContent } from "@/webview/components/ui/card"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger
} from "@/webview/components/ui/collapsible"
import { cn } from "@/webview/lib/utils"
import {
  ToolCallCard,
  type ToolCallCardCallbacks,
  type ToolCallCardProps
} from "./ToolCallCard"

export interface ToolCallGroupCardProps {
  id: string
  title: string
  detail: string
  status: string
  statusLabel: string
  items: ToolCallCardProps[]
}

export interface MountToolCallGroupCardsOptions extends ToolCallCardCallbacks {
  getProps(id: string): ToolCallGroupCardProps | undefined
  ids?: ReadonlySet<string> | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastToolCallGroupPropsJson = new Map<string, string>()

export function ToolCallGroupCard({
  callbacks,
  props
}: {
  callbacks: ToolCallCardCallbacks
  props: ToolCallGroupCardProps
}) {
  const active = props.status === "pending" || props.status === "in_progress"
  const [open, setOpen] = React.useState(active)
  const StatusIcon = groupStatusIcon(props.status)

  React.useEffect(() => {
    setOpen(active)
  }, [active, props.id])

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <Card
        size="sm"
        data-tool-call-group=""
        data-open={open ? "" : undefined}
        className={cn(
          "card-body tool-group-card relative gap-0 overflow-hidden rounded-md border border-border bg-card py-0 text-card-foreground shadow-none ring-0",
          `tool-group-${props.status}`,
          open ? "is-open" : ""
        )}
      >
        <CollapsibleTrigger
          className="tool-group-summary grid w-full grid-cols-[auto_minmax(0,1fr)_auto_auto] items-center gap-2 border-0 bg-transparent px-2 py-1.5 text-left hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
          aria-label={`${open ? "Collapse" : "Expand"} ${props.title}. ${props.detail}`}
        >
          <span className="summary-icon tool-summary-icon inline-flex size-6 shrink-0 items-center justify-center rounded-md border border-border bg-muted text-muted-foreground">
            <Wrench data-icon="ml-2 inline-start" aria-hidden="true" />
          </span>
          <span className="tool-group-main min-w-0">
            <span className="tool-title block truncate text-sm font-medium text-card-foreground">
              {props.title}
            </span>
          </span>
          <span
            className={cn("tool-group-status", `tool-group-status-${props.status}`)}
            aria-label={`${props.items.length} ${props.items.length === 1 ? "tool call" : "tool calls"}, ${props.statusLabel}`}
            title={`${props.items.length} ${props.items.length === 1 ? "tool call" : "tool calls"} · ${props.statusLabel}`}
          >
            <span className="tool-group-count">{props.items.length}</span>
            <StatusIcon aria-hidden="true" />
          </span>
          <ChevronDown
            aria-hidden="true"
            className={cn(
              "tool-disclosure-icon size-4 shrink-0 text-muted-foreground transition-transform",
              open ? "rotate-180" : ""
            )}
          />
        </CollapsibleTrigger>
        <CollapsibleContent>
          <CardContent className="tool-group-content grid gap-2 px-3 pb-3 pt-1">
            {props.items.map((item) => (
              <ToolCallCard
                key={item.nodeId}
                props={item}
                callbacks={callbacks}
              />
            ))}
          </CardContent>
        </CollapsibleContent>
      </Card>
    </Collapsible>
  )
}

function groupStatusIcon(status: string): LucideIcon {
  if (status === "completed") {
    return Check
  }
  if (status === "failed") {
    return CircleAlert
  }
  if (status === "cancelled") {
    return CircleX
  }
  if (status === "pending" || status === "in_progress") {
    return Clock3
  }
  return Wrench
}

export function mountToolCallGroupCards(options: MountToolCallGroupCardsOptions): void {
  const activeIds = new Set<string>()
  flushSync(() => {
    document.querySelectorAll<HTMLElement>("[data-tool-call-group-root]").forEach((element) => {
      const id = element.dataset.toolCallGroupId
      if (!id) {
        return
      }
      if (options.ids && !options.ids.has(id)) {
        activeIds.add(id)
        return
      }
      const props = options.getProps(id)
      if (!props) {
        return
      }
      activeIds.add(id)
      const currentJson = JSON.stringify(props)
      let mounted = mountedRoots.get(id)
      if (
        currentJson === lastToolCallGroupPropsJson.get(id) &&
        mounted?.element === element &&
        mounted.element.isConnected
      ) {
        return
      }
      lastToolCallGroupPropsJson.set(id, currentJson)
      if (!mounted || mounted.element !== element) {
        mounted?.root.unmount()
        mounted = {
          element,
          root: createRoot(element)
        }
        mountedRoots.set(id, mounted)
      }
      mounted.root.render(<ToolCallGroupCard props={props} callbacks={options} />)
    })
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
      lastToolCallGroupPropsJson.delete(id)
    }
  })
}

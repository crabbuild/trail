import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"
import { ChevronDown, Wrench } from "lucide-react"

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
  const [open, setOpen] = React.useState(false)

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <Card
        size="sm"
        data-tool-call-group=""
        data-open={open ? "" : undefined}
        className={cn(
          "card-body tool-group-card relative gap-0 overflow-hidden rounded-md border border-transparent bg-muted/10 py-0 text-muted-foreground shadow-none ring-0",
          `tool-group-${props.status}`,
          open ? "is-open" : ""
        )}
      >
        <CollapsibleTrigger
          className="tool-group-summary grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 border-0 px-2 py-1.5 text-left bg-muted/10 hover:bg-muted/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
          aria-label={`${open ? "Collapse" : "Expand"} ${props.title}. ${props.detail}`}
        >
          <span className="summary-icon tool-summary-icon inline-flex size-5 shrink-0 items-center justify-center text-muted-foreground">
            <Wrench data-icon="ml-2 inline-start" aria-hidden="true" />
          </span>
          <span className="tool-group-main min-w-0">
            <span className="tool-title block truncate text-sm font-normal text-muted-foreground">
              {props.title}
            </span>
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

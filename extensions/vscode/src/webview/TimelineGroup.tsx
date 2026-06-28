import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from "@/webview/components/ui/accordion"
import { Badge } from "@/webview/components/ui/badge"
import { cn } from "@/webview/lib/utils"

export interface TimelineGroupCardProps {
  id: string
  label: string
  detail: string
  status: string
  statusLabel: string
  laneLabel: string
  iconHtml: string
  bodyHtml: string
  open: boolean
}

export interface MountTimelineGroupsOptions {
  getProps(id: string): TimelineGroupCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()

export function TimelineGroupCard({ props }: { props: TimelineGroupCardProps }) {
  return (
    <Accordion className="timeline-group-accordion" defaultValue={props.open ? [props.id] : undefined}>
      <AccordionItem className="timeline-group-item" value={props.id}>
        <AccordionTrigger className="timeline-group-summary">
          <span
            className="summary-icon"
            dangerouslySetInnerHTML={{ __html: props.iconHtml }}
          />
          <span className="timeline-group-main">
            <span className="timeline-group-title">{props.label}</span>
            <span className="timeline-group-detail">{props.detail}</span>
          </span>
          <span className="timeline-group-meta">
            <Badge className={cn("tool-status", `tool-status-${props.status}`)} variant="outline">
              {props.statusLabel}
            </Badge>
            <Badge className="event-chip" variant="outline">
              {props.laneLabel}
            </Badge>
          </span>
        </AccordionTrigger>
        <AccordionContent className="timeline-group-body" keepMounted>
          <div dangerouslySetInnerHTML={{ __html: props.bodyHtml }} />
        </AccordionContent>
      </AccordionItem>
    </Accordion>
  )
}

export function mountTimelineGroups(options: MountTimelineGroupsOptions): void {
  const activeIds = new Set<string>()
  flushSync(() => {
    document.querySelectorAll<HTMLElement>("[data-timeline-group-root]").forEach((element) => {
      const id = element.dataset.timelineGroupId
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
      mounted.root.render(<TimelineGroupCard props={props} />)
    })
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
    }
  })
}

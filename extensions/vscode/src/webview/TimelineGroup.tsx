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
  bodyItems: TimelineGroupBodyItem[]
  open: boolean
}

export interface TimelineGroupBodyItem {
  id: string
  className: string
  html: string
  preserveDom?: boolean | undefined
}

export interface MountTimelineGroupsOptions {
  getProps(id: string): TimelineGroupCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastTimelineGroupPropsJson = new Map<string, string>()

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
          {props.bodyItems.map((item) => (
            <TimelineGroupBodySlot key={item.id} item={item} />
          ))}
        </AccordionContent>
      </AccordionItem>
    </Accordion>
  )
}

function TimelineGroupBodySlot({ item }: { item: TimelineGroupBodyItem }) {
  return (
    <div
      className={item.className}
      data-timeline-group-body-item=""
      data-node-id={item.id}
    >
      {item.preserveDom ? (
        <StableHtmlSlot slotId={item.id} html={item.html} />
      ) : (
        <div className="stable-html-slot" dangerouslySetInnerHTML={{ __html: item.html }} />
      )}
    </div>
  )
}

const StableHtmlSlot = React.memo(
  function StableHtmlSlot({
    html,
    slotId
  }: {
    html: string
    slotId: string
  }) {
    return <div className="stable-html-slot" data-stable-html-slot={slotId} dangerouslySetInnerHTML={{ __html: html }} />
  },
  (previous, next) => previous.slotId === next.slotId
)

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
      const currentJson = timelineGroupPropsSignature(props)
      let mounted = mountedRoots.get(id)
      if (
        currentJson === lastTimelineGroupPropsJson.get(id) &&
        mounted?.element === element &&
        mounted.element.isConnected
      ) {
        activeIds.add(id)
        return
      }
      lastTimelineGroupPropsJson.set(id, currentJson)
      activeIds.add(id)
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
      lastTimelineGroupPropsJson.delete(id)
    }
  })
}

function timelineGroupPropsSignature(props: TimelineGroupCardProps): string {
  return JSON.stringify({
    id: props.id,
    label: props.label,
    detail: props.detail,
    status: props.status,
    statusLabel: props.statusLabel,
    laneLabel: props.laneLabel,
    iconHtml: props.iconHtml,
    open: props.open,
    bodyItems: props.bodyItems.map((item) => ({
      id: item.id,
      className: item.className,
      preserveDom: item.preserveDom,
      html: item.preserveDom ? undefined : item.html
    }))
  })
}

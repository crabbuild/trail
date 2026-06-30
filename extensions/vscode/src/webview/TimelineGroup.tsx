import * as React from "react"
import { Accordion as AccordionPrimitive } from "@base-ui/react/accordion"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"
import { ChevronDown, ChevronUp, Copy } from "lucide-react"

import {
  Accordion,
  AccordionContent,
  AccordionItem
} from "@/webview/components/ui/accordion"
import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import { cn } from "@/webview/lib/utils"
import { useSyncedAccordionValue } from "./syncedAccordionState"

export interface TimelineGroupCardProps {
  id: string
  label: string
  detail: string
  status: string
  statusLabel: string
  laneId: string
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
  ids?: ReadonlySet<string> | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastTimelineGroupPropsJson = new Map<string, string>()

export function TimelineGroupCard({ props }: { props: TimelineGroupCardProps }) {
  const [openValue, setOpenValue] = useSyncedAccordionValue(timelineGroupOpenValue(props))

  return (
    <Accordion className="timeline-group-accordion" value={openValue} onValueChange={setOpenValue}>
      <AccordionItem className="timeline-group-item" value={props.id}>
        <AccordionPrimitive.Header className="timeline-group-header">
          <AccordionPrimitive.Trigger
            data-slot="accordion-trigger"
            className="timeline-group-summary"
          >
            <span
              className="summary-icon timeline-group-icon"
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
            </span>
            <span className="timeline-group-disclosure" aria-hidden="true">
              <ChevronDown
                data-slot="accordion-trigger-icon"
                className="timeline-group-disclosure-down"
              />
              <ChevronUp
                data-slot="accordion-trigger-icon"
                className="timeline-group-disclosure-up"
              />
            </span>
          </AccordionPrimitive.Trigger>
          {props.laneId ? (
            <Button
              type="button"
              className="timeline-group-copy-id"
              data-action="copyTimelineGroupId"
              data-target={props.laneId}
              variant="ghost"
              size="icon-xs"
              title="Copy ID"
              aria-label="Copy ID"
            >
              <Copy data-icon="inline-start" aria-hidden="true" />
            </Button>
          ) : null}
        </AccordionPrimitive.Header>
        <AccordionContent className="timeline-group-body" keepMounted>
          {props.bodyItems.map((item) => (
            <TimelineGroupBodySlot key={item.id} item={item} />
          ))}
        </AccordionContent>
      </AccordionItem>
    </Accordion>
  )
}

function timelineGroupOpenValue(props: TimelineGroupCardProps): string[] {
  return props.open ? [props.id] : []
}

function TimelineGroupBodySlot({ item }: { item: TimelineGroupBodyItem }) {
  return (
    <div
      className={item.className}
      data-timeline-group-body-item=""
      data-node-id={item.id}
    >
      {item.preserveDom ? (
        <StableHtmlSlot shellSignature={stableHtmlShellSignature(item.html)} slotId={item.id} html={item.html} />
      ) : (
        <div className="stable-html-slot" dangerouslySetInnerHTML={{ __html: item.html }} />
      )}
    </div>
  )
}

const StableHtmlSlot = React.memo(
  function StableHtmlSlot({
    html,
    shellSignature: _shellSignature,
    slotId
  }: {
    html: string
    shellSignature: string
    slotId: string
  }) {
    const rootRef = React.useRef<HTMLDivElement | null>(null)
    const initialHtml = React.useRef(html)

    React.useLayoutEffect(() => {
      syncStableHtmlShell(rootRef.current, html)
    }, [html, _shellSignature])

    return (
      <div
        ref={rootRef}
        className="stable-html-slot"
        data-stable-html-slot={slotId}
        dangerouslySetInnerHTML={{ __html: initialHtml.current }}
      />
    )
  },
  (previous, next) =>
    previous.slotId === next.slotId &&
    previous.shellSignature === next.shellSignature &&
    previous.html === next.html
)

export function mountTimelineGroups(options: MountTimelineGroupsOptions): void {
  const activeIds = new Set<string>()
  flushSync(() => {
    document.querySelectorAll<HTMLElement>("[data-timeline-group-root]").forEach((element) => {
      const id = element.dataset.timelineGroupId
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
    laneId: props.laneId,
    iconHtml: props.iconHtml,
    open: props.open,
    bodyItems: props.bodyItems.map((item) => ({
      id: item.id,
      className: item.className,
      preserveDom: item.preserveDom,
      html: item.html
    }))
  })
}

function stableHtmlShellSignature(html: string): string {
  return html.trim().match(/^<([a-z][\w:-]*)(?:\s[^>]*)?>/i)?.[0] ?? html
}

function syncStableHtmlShell(root: HTMLDivElement | null, html: string): void {
  if (!root) {
    return
  }
  const next = stableHtmlFirstElement(html)
  const current = root.firstElementChild
  if (!next || !current || next.tagName !== current.tagName) {
    root.innerHTML = html
    return
  }
  syncElementAttributes(current, next)
}

function stableHtmlFirstElement(html: string): Element | undefined {
  const template = document.createElement("template")
  template.innerHTML = html.trim()
  return template.content.firstElementChild ?? undefined
}

function syncElementAttributes(current: Element, next: Element): void {
  for (const attr of Array.from(current.attributes)) {
    if (!next.hasAttribute(attr.name)) {
      current.removeAttribute(attr.name)
    }
  }
  for (const attr of Array.from(next.attributes)) {
    if (current.getAttribute(attr.name) !== attr.value) {
      current.setAttribute(attr.name, attr.value)
    }
  }
}

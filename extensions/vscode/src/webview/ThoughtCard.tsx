import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from "@/webview/components/ui/accordion"
import { Badge } from "@/webview/components/ui/badge"
import { Card, CardContent } from "@/webview/components/ui/card"

export interface ThoughtCardProps {
  nodeId: string
  title: string
  detail: string
  statusLabel: string
  iconHtml: string
  contentHtml: string
  emptyText: string
}

export interface MountThoughtCardsOptions {
  getProps(nodeId: string): ThoughtCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()

export function ThoughtCard({ props }: { props: ThoughtCardProps }) {
  return (
    <Card
      data-thought-card=""
      className="card-body event-card event-info thought-card"
      size="sm"
    >
      <CardContent className="thought-card-content">
        <Accordion className="thought-accordion">
          <AccordionItem className="thought-accordion-item" value={props.nodeId}>
            <AccordionTrigger className="event-summary thought-summary">
              <span
                className="summary-icon"
                dangerouslySetInnerHTML={{ __html: props.iconHtml }}
              />
              <span className="event-main">
                <span className="event-title">{props.title}</span>
                <span className="event-detail">{props.detail}</span>
              </span>
              <Badge className="tool-status" variant="outline">
                {props.statusLabel}
              </Badge>
            </AccordionTrigger>
            <AccordionContent className="thought-panel" keepMounted>
              {props.contentHtml ? (
                <div
                  className="markdown event-content"
                  dangerouslySetInnerHTML={{ __html: props.contentHtml }}
                />
              ) : (
                <p className="muted thought-empty">{props.emptyText}</p>
              )}
            </AccordionContent>
          </AccordionItem>
        </Accordion>
      </CardContent>
    </Card>
  )
}

export function mountThoughtCards(options: MountThoughtCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-thought-card-root]").forEach((element) => {
    const nodeId = element.dataset.thoughtNodeId
    if (!nodeId) {
      return
    }
    const props = options.getProps(nodeId)
    if (!props) {
      return
    }
    activeIds.add(nodeId)
    let mounted = mountedRoots.get(nodeId)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(nodeId, mounted)
    }
    mounted.root.render(<ThoughtCard props={props} />)
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(nodeId)
    }
  })
}

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
import { Card, CardContent } from "@/webview/components/ui/card"
import { StreamdownMarkdown } from "./StreamdownMarkdown"

export interface ThoughtCardProps {
  nodeId: string
  title: string
  detail: string
  statusLabel: string
  iconHtml: string
  contentHtml: string
  contentMode?: "html" | "stream-text" | undefined
  contentText?: string | undefined
  emptyText: string
}

export interface MountThoughtCardsOptions {
  getProps(nodeId: string): ThoughtCardProps | undefined
  ids?: ReadonlySet<string> | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastThoughtCardPropsJson = new Map<string, string>()

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
              {props.contentMode === "stream-text" ? (
                <StreamdownMarkdown className="event-content" streaming text={props.contentText || ""} />
              ) : props.contentHtml ? (
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
    if (options.ids && !options.ids.has(nodeId)) {
      activeIds.add(nodeId)
      return
    }
    const props = options.getProps(nodeId)
    if (!props) {
      return
    }
    const currentJson = JSON.stringify(props)
    let mounted = mountedRoots.get(nodeId)
    if (
      currentJson === lastThoughtCardPropsJson.get(nodeId) &&
      mounted?.element === element &&
      mounted.element.isConnected
    ) {
      activeIds.add(nodeId)
      return
    }
    lastThoughtCardPropsJson.set(nodeId, currentJson)
    activeIds.add(nodeId)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(nodeId, mounted)
    }
    flushSync(() => {
      mounted.root.render(<ThoughtCard props={props} />)
    })
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(nodeId)
      lastThoughtCardPropsJson.delete(nodeId)
    }
  })
}

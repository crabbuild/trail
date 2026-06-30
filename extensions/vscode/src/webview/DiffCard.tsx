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

export interface DiffCardStat {
  label: string
}

export interface DiffCardProps {
  nodeId: string
  path: string
  subtitle: string
  iconHtml: string
  stats: DiffCardStat[]
  previewHtml: string
}

export interface MountDiffCardsOptions {
  getProps(nodeId: string): DiffCardProps | undefined
  ids?: ReadonlySet<string> | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastDiffCardPropsJson = new Map<string, string>()

export function DiffCard({ props }: { props: DiffCardProps }) {
  return (
    <Card data-diff-card="" className="card-body diff-card" size="sm">
      <CardContent className="diff-card-content">
        <Accordion className="diff-accordion">
          <AccordionItem className="diff-accordion-item" value={props.nodeId}>
            <AccordionTrigger className="tool-summary diff-summary">
              <span
                className="summary-icon"
                dangerouslySetInnerHTML={{ __html: props.iconHtml }}
              />
              <span className="tool-summary-main">
                <span className="tool-title">{props.path}</span>
                <span className="tool-subtitle">{props.subtitle}</span>
              </span>
              <span className="tool-summary-meta">
                {props.stats.map((stat) => (
                  <Badge key={stat.label} className="diff-stat" variant="outline">
                    {stat.label}
                  </Badge>
                ))}
              </span>
            </AccordionTrigger>
            <AccordionContent className="diff-panel" keepMounted>
              <div dangerouslySetInnerHTML={{ __html: props.previewHtml }} />
            </AccordionContent>
          </AccordionItem>
        </Accordion>
      </CardContent>
    </Card>
  )
}

export function mountDiffCards(options: MountDiffCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-diff-card-root]").forEach((element) => {
    const nodeId = element.dataset.diffNodeId
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
    let mounted = mountedRoots.get(nodeId)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(nodeId, mounted)
    }
    const currentJson = JSON.stringify(props)
    if (currentJson === lastDiffCardPropsJson.get(nodeId)) {
      activeIds.add(nodeId)
      return
    }
    lastDiffCardPropsJson.set(nodeId, currentJson)
    flushSync(() => {
      mounted.root.render(<DiffCard props={props} />)
    })
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(nodeId)
      lastDiffCardPropsJson.delete(nodeId)
    }
  })
}

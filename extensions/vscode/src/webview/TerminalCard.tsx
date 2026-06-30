import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from "@/webview/components/ui/accordion"
import { Badge } from "@/webview/components/ui/badge"
import { Card, CardContent, CardHeader } from "@/webview/components/ui/card"
import { Separator } from "@/webview/components/ui/separator"
import { cn } from "@/webview/lib/utils"
import { InlineActions } from "./InlineActions"
import type { TerminalTone } from "./terminalModel"

export interface TerminalTranscriptRow {
  id: string
  kind: "in" | "out" | "err"
  label: string
  title: string
  detail: string
  textHtml: string
  language?: string | undefined
  meta?: string | undefined
  tone: TerminalTone
  truncated: boolean
  empty: boolean
  openByDefault: boolean
}

export interface TerminalCardProps {
  nodeId: string
  status: string
  tone: TerminalTone
  title: string
  subtitle: string
  statusLabel: string
  iconHtml: string
  openIconHtml: string
  rows: TerminalTranscriptRow[]
}

export interface MountTerminalCardsOptions {
  getProps(nodeId: string): TerminalCardProps | undefined
  ids?: ReadonlySet<string> | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastTerminalCardPropsJson = new Map<string, string>()

export function TerminalCard({ props }: { props: TerminalCardProps }) {
  const commandRow = props.rows.find((row) => row.kind === "in")
  const outputRows = props.rows.filter((row) => row.kind !== "in")
  const openValues = outputRows
    .filter((row) => row.openByDefault)
    .map((row) => row.id)

  return (
    <Card
      size="sm"
      data-terminal-card=""
      className={cn("card-body terminal-card", `terminal-tone-${props.tone}`)}
    >
      <CardHeader className="tool-summary terminal-summary">
        <span
          className="summary-icon"
          dangerouslySetInnerHTML={{ __html: props.iconHtml }}
        />
        <span className="tool-summary-main">
          <span className="tool-title">{props.title}</span>
          <span className="tool-subtitle">{props.subtitle}</span>
        </span>
        <span className="tool-summary-meta">
          <Badge className="tool-kind" variant="outline">
            Terminal
          </Badge>
          <Badge
            className={cn("tool-status", `tool-status-${props.status}`)}
            variant="outline"
          >
            {props.statusLabel}
          </Badge>
        </span>
      </CardHeader>
      <CardContent className="terminal-card-content">
        <div className={cn("terminal-transcript", `terminal-tone-${props.tone}`)}>
          {commandRow ? <StaticRow row={commandRow} /> : null}
          {commandRow && outputRows.length ? <Separator className="terminal-section-separator" /> : null}
          {outputRows.length ? (
            <Accordion
              multiple
              keepMounted
              defaultValue={openValues}
              className="terminal-transcript-sections"
            >
              {outputRows.map((row) => (
                <AccordionItem
                  key={row.id}
                  value={row.id}
                  className={cn(
                    "terminal-transcript-row",
                    `terminal-transcript-${row.kind}`,
                    `terminal-tone-${row.tone}`
                  )}
                >
                  <span className="terminal-transcript-label">{row.label}</span>
                  <div className="terminal-transcript-cell">
                    <AccordionTrigger className="terminal-section-trigger">
                      <span>{row.title}</span>
                      <Badge variant="outline">{row.detail}</Badge>
                    </AccordionTrigger>
                    <AccordionContent className="terminal-section-content">
                      <TerminalCode row={row} />
                    </AccordionContent>
                  </div>
                </AccordionItem>
              ))}
            </Accordion>
          ) : commandRow ? null : (
            <StaticRow
              row={{
                id: "empty",
                kind: "out",
                label: "OUT",
                title: "Output",
                detail: "empty",
                textHtml: "",
                tone: "muted",
                truncated: false,
                empty: true,
                openByDefault: true
              }}
            />
          )}
          <InlineActions
            props={{
              id: `terminal-actions:${props.nodeId}`,
              className: "terminal-transcript-actions",
              ariaLabel: "Terminal actions",
              actions: [
                {
                  action: "openTerminal",
                  label: "Open terminal",
                  title: "Open terminal",
                  ariaLabel: "Open terminal",
                  iconHtml: props.openIconHtml,
                  iconOnly: true,
                  data: { "node-id": props.nodeId }
                }
              ]
            }}
          />
        </div>
      </CardContent>
    </Card>
  )
}

function StaticRow({ row }: { row: TerminalTranscriptRow }) {
  return (
    <div
      className={cn(
        "terminal-transcript-row",
        `terminal-transcript-${row.kind}`,
        `terminal-tone-${row.tone}`
      )}
    >
      <span className="terminal-transcript-label">{row.label}</span>
      <div className="terminal-transcript-cell">
        <TerminalCode row={row} />
      </div>
    </div>
  )
}

function TerminalCode({ row }: { row: TerminalTranscriptRow }) {
  return (
    <>
      <pre
        className={cn("terminal-transcript-code code", row.empty ? "terminal-transcript-empty" : "")}
        data-highlight-language={row.language}
        tabIndex={0}
        dangerouslySetInnerHTML={{ __html: row.textHtml }}
      />
      {row.meta ? <small className="terminal-transcript-note">{row.meta}</small> : null}
      {row.truncated ? (
        <small className="terminal-transcript-note">truncated at 24,000 chars</small>
      ) : null}
    </>
  )
}

export function mountTerminalCards(options: MountTerminalCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-terminal-card-root]").forEach((element) => {
    const nodeId = element.dataset.terminalNodeId
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
    const currentJson = JSON.stringify(props)
    if (currentJson === lastTerminalCardPropsJson.get(nodeId)) {
      return
    }
    lastTerminalCardPropsJson.set(nodeId, currentJson)
    let mounted = mountedRoots.get(nodeId)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(nodeId, mounted)
    }
    mounted.root.render(<TerminalCard props={props} />)
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(nodeId)
      lastTerminalCardPropsJson.delete(nodeId)
    }
  })
}

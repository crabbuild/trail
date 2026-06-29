import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from "@/webview/components/ui/accordion"
import { Alert, AlertDescription, AlertTitle } from "@/webview/components/ui/alert"
import { Badge } from "@/webview/components/ui/badge"
import { Card, CardContent, CardHeader } from "@/webview/components/ui/card"
import { Separator } from "@/webview/components/ui/separator"
import { cn } from "@/webview/lib/utils"
import { InlineActions, type InlineActionTone } from "./InlineActions"
import { RawDetails, type RawDetailsView } from "./RawDetails"
import type { EventActionTone, EventTone } from "./eventModel"

export interface EventCardFact {
  label: string
  value: string
  shortValue: string
  active: boolean
}

export interface EventCardCallout {
  title: string
  detail: string
  tone: EventTone
}

export interface EventCardAction {
  action: string
  label: string
  tone: EventActionTone
  target?: string | undefined
  iconHtml: string
}

export interface EventCardProps {
  nodeId: string
  tone: EventTone
  iconHtml: string
  title: string
  detail: string
  statusLabel?: string | undefined
  facts: EventCardFact[]
  chipsHtml: string
  callout?: EventCardCallout | undefined
  actions: EventCardAction[]
  meterHtml: string
  contentHtml: string
  rawDetails?: RawDetailsView | undefined
  variant?: "card" | "checkpoint" | undefined
  defaultOpen?: boolean | undefined
}

export interface MountEventCardsOptions {
  getProps(nodeId: string): EventCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastEventCardPropsJson = new Map<string, string>()

export function EventCard({ props }: { props: EventCardProps }) {
  const hasBody = eventCardHasBody(props)

  if (props.variant === "checkpoint") {
    return <CheckpointSeparator props={props} hasBody={hasBody} />
  }

  return (
    <Card
      size="sm"
      data-event-card=""
      className={cn(
        "card-body event-card audit-event",
        `event-${props.tone}`,
        `audit-${props.tone}`
      )}
    >
      <CardHeader className="event-summary audit-event-summary">
        <span
          className="summary-icon"
          dangerouslySetInnerHTML={{ __html: props.iconHtml }}
        />
        <span className="event-main">
          <span className="event-title">{props.title}</span>
          <span className="event-detail">{props.detail}</span>
        </span>
        {props.statusLabel ? (
          <Badge className="tool-status" variant="outline">
            {props.statusLabel}
          </Badge>
        ) : null}
      </CardHeader>
      {hasBody ? (
        <CardContent className="event-card-content">
          <EventCardBody props={props} />
        </CardContent>
      ) : null}
    </Card>
  )
}

function CheckpointSeparator({
  props,
  hasBody
}: {
  props: EventCardProps
  hasBody: boolean
}) {
  return (
    <Accordion
      className="checkpoint-separator"
      defaultValue={props.defaultOpen ? [props.nodeId] : undefined}
      data-event-card=""
    >
      <AccordionItem className="checkpoint-separator-item" value={props.nodeId}>
        <AccordionTrigger className="checkpoint-separator-summary">
          <span
            className="summary-icon checkpoint-separator-icon"
            dangerouslySetInnerHTML={{ __html: props.iconHtml }}
          />
          <span className="event-main checkpoint-separator-main">
            <span className="event-title">{props.title}</span>
            <span className="event-detail">{props.detail}</span>
          </span>
          <span className="checkpoint-separator-rule" aria-hidden="true" />
          {props.statusLabel ? (
            <Badge className="tool-status checkpoint-separator-status" variant="outline">
              {props.statusLabel}
            </Badge>
          ) : null}
        </AccordionTrigger>
        {hasBody ? (
          <AccordionContent className="checkpoint-separator-body" keepMounted>
            <div className="event-card-content checkpoint-separator-content">
              <EventCardBody props={props} />
            </div>
          </AccordionContent>
        ) : null}
      </AccordionItem>
    </Accordion>
  )
}

function EventCardBody({ props }: { props: EventCardProps }) {
  return (
    <>
      <EventFacts facts={props.facts} />
      <HtmlBlock className="event-chips" html={props.chipsHtml} />
      <EventCallout callout={props.callout} />
      <HtmlBlock html={props.meterHtml} />
      <HtmlBlock html={props.contentHtml} />
      {props.rawDetails ? <RawDetails details={props.rawDetails} /> : null}
      {props.actions.length ? (
        <>
          <Separator className="event-card-separator" />
          <EventActions actions={props.actions} />
        </>
      ) : null}
    </>
  )
}

function eventCardHasBody(props: EventCardProps): boolean {
  return (
    props.facts.length > 0 ||
    Boolean(props.chipsHtml) ||
    Boolean(props.callout) ||
    Boolean(props.meterHtml) ||
    Boolean(props.contentHtml) ||
    Boolean(props.rawDetails) ||
    props.actions.length > 0
  )
}

function EventFacts({ facts }: { facts: EventCardFact[] }) {
  if (!facts.length) {
    return null
  }
  return (
    <div className="event-facts" aria-label="Event facts">
      {facts.map((fact) => (
        <Badge
          key={`${fact.label}-${fact.value}`}
          className={cn("event-fact", fact.active ? "active" : "")}
          title={`${fact.label}: ${fact.value}`}
          variant="outline"
        >
          <b>{fact.label}</b>
          {fact.shortValue}
        </Badge>
      ))}
    </div>
  )
}

function EventCallout({ callout }: { callout?: EventCardCallout | undefined }) {
  if (!callout) {
    return null
  }
  return (
    <Alert
      className={cn("event-callout", `event-callout-${callout.tone}`)}
      variant={callout.tone === "risk" ? "destructive" : "default"}
    >
      <AlertTitle>
        <strong>{callout.title}</strong>
      </AlertTitle>
      <AlertDescription>
        <span>{callout.detail}</span>
      </AlertDescription>
    </Alert>
  )
}

function EventActions({ actions }: { actions: EventCardAction[] }) {
  return (
    <InlineActions
      props={{
        id: "event-actions",
        className: "event-action-row",
        ariaLabel: "Event actions",
        actions: actions.map((action) => ({
          action: action.action,
          label: action.label,
          tone: eventActionTone(action.tone),
          iconHtml: action.iconHtml,
          data: action.target ? { target: action.target } : undefined
        }))
      }}
    />
  )
}

function eventActionTone(tone: EventActionTone): InlineActionTone {
  if (tone === "primary") {
    return "primary"
  }
  if (tone === "danger") {
    return "danger"
  }
  return "default"
}

function HtmlBlock({
  html,
  className
}: {
  html: string
  className?: string | undefined
}) {
  if (!html) {
    return null
  }
  return <div className={className} dangerouslySetInnerHTML={{ __html: html }} />
}

export function mountEventCards(options: MountEventCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-event-card-root]").forEach((element) => {
    const nodeId = element.dataset.eventNodeId
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
    const currentJson = JSON.stringify(props)
    if (currentJson === lastEventCardPropsJson.get(nodeId)) {
      return
    }
    lastEventCardPropsJson.set(nodeId, currentJson)
    mounted.root.render(<EventCard props={props} />)
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(nodeId)
      lastEventCardPropsJson.delete(nodeId)
    }
  })
}

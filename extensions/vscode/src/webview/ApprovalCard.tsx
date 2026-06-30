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
import { Button } from "@/webview/components/ui/button"
import { ButtonGroup } from "@/webview/components/ui/button-group"
import { Card, CardContent, CardHeader } from "@/webview/components/ui/card"
import { cn } from "@/webview/lib/utils"
import type { ApprovalDecisionTone, ApprovalTone } from "./approvalModel"
import { useSyncedAccordionValue } from "./syncedAccordionState"

export interface ApprovalCardAction {
  kind: "approve" | "reject"
  optionId?: string | undefined
  label: string
  description: string
  tone: ApprovalDecisionTone | "risk"
  iconHtml: string
  disabled: boolean
}

export interface ApprovalCardMeta {
  label: string
  iconHtml?: string | undefined
}

export interface ApprovalCardDisclosure {
  id: string
  title: string
  meta?: string | undefined
  className: string
  contentClassName: string
  contentHtml: string
  defaultOpen: boolean
}

export interface ApprovalCardProps {
  nodeId: string
  requestId: string
  status: string
  statusLabel: string
  tone: ApprovalTone
  resolved: boolean
  summaryIconHtml: string
  title: string
  detail: string
  resolvedNote: string
  impactText: string
  meta: ApprovalCardMeta[]
  locationsHtml: string
  preview?: ApprovalCardDisclosure | undefined
  requestDetails: ApprovalCardDisclosure
  actions: ApprovalCardAction[]
}

export interface MountApprovalCardsOptions {
  getProps(nodeId: string): ApprovalCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastApprovalCardPropsJson = new Map<string, string>()

export function ApprovalCard({ props }: { props: ApprovalCardProps }) {
  return (
    <Card
      size="sm"
      data-approval-card=""
      className={cn(
        "card-body approval-card event-card",
        `event-${props.tone}`,
        props.resolved ? "approval-card-resolved" : ""
      )}
    >
      <CardHeader className="event-summary approval-summary-row">
        <span
          className="summary-icon"
          dangerouslySetInnerHTML={{ __html: props.summaryIconHtml }}
        />
        <span className="event-main">
          <span className="event-title">{props.title}</span>
          <span className="event-detail">{props.detail}</span>
        </span>
      </CardHeader>
      <CardContent className="approval-card-content">
        {props.resolved ? (
          <p className="approval-resolved-note">{props.resolvedNote}</p>
        ) : (
          <>
            <p className="approval-impact">{props.impactText}</p>
            <ApprovalDisclosures disclosures={[props.preview].filter(Boolean) as ApprovalCardDisclosure[]} />
            <ApprovalDecision props={props} />
          </>
        )}
        <ApprovalDisclosures disclosures={[props.requestDetails]} />
      </CardContent>
    </Card>
  )
}

function ApprovalDisclosures({ disclosures }: { disclosures: ApprovalCardDisclosure[] }) {
  const [openValues, setOpenValues] = useSyncedAccordionValue(approvalDisclosureOpenValues(disclosures))

  if (!disclosures.length) {
    return null
  }

  return (
    <Accordion className="approval-disclosures" value={openValues} onValueChange={setOpenValues}>
      {disclosures.map((disclosure) => (
        <AccordionItem
          key={disclosure.id}
          className={cn("approval-disclosure", disclosure.className)}
          value={disclosure.id}
        >
          <AccordionTrigger className="approval-disclosure-summary">
            <span>{disclosure.title}</span>
            {disclosure.meta ? (
              <Badge className="approval-disclosure-meta" variant="outline">
                {disclosure.meta}
              </Badge>
            ) : null}
          </AccordionTrigger>
          <AccordionContent className="approval-disclosure-panel" keepMounted>
            <div
              className={disclosure.contentClassName}
              dangerouslySetInnerHTML={{ __html: disclosure.contentHtml }}
            />
          </AccordionContent>
        </AccordionItem>
      ))}
    </Accordion>
  )
}

function approvalDisclosureOpenValues(disclosures: ApprovalCardDisclosure[]): string[] {
  return disclosures.filter((disclosure) => disclosure.defaultOpen).map((disclosure) => disclosure.id)
}

function ApprovalDecision({ props }: { props: ApprovalCardProps }) {
  const approveActions = props.actions.filter((action) => action.kind === "approve")
  const rejectAction = props.actions.find((action) => action.kind === "reject")
  return (
    <div className="approval-decision" role="group" aria-label="Permission decision">
      {approveActions.length ? (
        <ButtonGroup className="approval-option-list" aria-label="Approval options">
          {approveActions.map((action) => (
            <ApprovalButton
              key={`${action.kind}-${action.optionId ?? action.label}`}
              action={action}
              requestId={props.requestId}
            />
          ))}
        </ButtonGroup>
      ) : null}
      {rejectAction ? (
        <ApprovalButton action={rejectAction} requestId={props.requestId} />
      ) : null}
    </div>
  )
}

function ApprovalButton({
  action,
  requestId
}: {
  action: ApprovalCardAction
  requestId: string
}) {
  const isReject = action.kind === "reject"
  const isPrimaryApprove = isPrimaryApprovalAction(action)
  const variant = isReject || action.tone === "risk" ? "destructive" : isPrimaryApprove ? "default" : "outline"
  return (
    <Button
      type="button"
      data-action={action.kind}
      data-request-id={requestId}
      data-option-id={action.optionId}
      title={action.description}
      aria-label={`${action.label}. ${action.description}`}
      disabled={action.disabled}
      variant={variant}
      size="sm"
      className={cn(
        isReject ? "danger approval-reject" : "approval-option",
        isReject ? "" : `approval-option-${action.tone}`,
        isPrimaryApprove ? "primary" : ""
      )}
    >
      <span
        data-icon="inline-start"
        dangerouslySetInnerHTML={{ __html: action.iconHtml }}
      />
      <span className="approval-decision-copy">
        <span>{action.label}</span>
      </span>
    </Button>
  )
}

function isPrimaryApprovalAction(action: ApprovalCardAction): boolean {
  if (action.kind !== "approve" || action.tone === "risk") {
    return false
  }
  const value = `${action.label} ${action.optionId ?? ""}`.toLowerCase()
  return !/\b(always|forever|persist)\b/.test(value)
}

export function mountApprovalCards(options: MountApprovalCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-approval-card-root]").forEach((element) => {
    const nodeId = element.dataset.approvalNodeId
    if (!nodeId) {
      return
    }
    const props = options.getProps(nodeId)
    if (!props) {
      return
    }
    activeIds.add(nodeId)
    const currentJson = JSON.stringify(props)
    if (currentJson === lastApprovalCardPropsJson.get(nodeId)) {
      return
    }
    lastApprovalCardPropsJson.set(nodeId, currentJson)
    let mounted = mountedRoots.get(nodeId)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(nodeId, mounted)
    }
    flushSync(() => {
      mounted.root.render(<ApprovalCard props={props} />)
    })
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      lastApprovalCardPropsJson.delete(nodeId)
      mountedRoots.delete(nodeId)
    }
  })
}

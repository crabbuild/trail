import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import { ButtonGroup } from "@/webview/components/ui/button-group"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle
} from "@/webview/components/ui/card"
import { cn } from "@/webview/lib/utils"
import type {
  ReviewAction,
  ReviewActionGroup,
  ReviewGate,
  ReviewMetric,
  ReviewReadiness
} from "./reviewModel"

export interface ReviewDrawerProps {
  id: string
  readiness: ReviewReadiness
  sectionsHtml: string
  actionIcons: Record<string, string>
  refreshAction: ReviewAction
}

export interface MountReviewDrawersOptions {
  getProps(id: string): ReviewDrawerProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastReviewDrawerPropsJson = new Map<string, string>()

export function ReviewDrawer({ props }: { props: ReviewDrawerProps }) {
  return (
    <>
      <ReviewCommandCenter
        readiness={props.readiness}
        refreshAction={props.refreshAction}
        actionIcons={props.actionIcons}
      />
      <section className="review-section review-gate-section">
        <h3>Readiness gates</h3>
        <ReviewGateStack gates={props.readiness.gates} />
      </section>
      <div
        className="review-section-stack"
        dangerouslySetInnerHTML={{ __html: props.sectionsHtml }}
      />
      <ReviewActionRail
        groups={props.readiness.actionGroups}
        actionIcons={props.actionIcons}
      />
    </>
  )
}

function ReviewCommandCenter({
  actionIcons,
  readiness,
  refreshAction
}: {
  actionIcons: Record<string, string>
  readiness: ReviewReadiness
  refreshAction: ReviewAction
}) {
  return (
    <Card
      size="sm"
      className={cn("review-command-center", `review-command-${readiness.tone}`)}
      aria-label="Review readiness"
      data-review-command=""
    >
      <CardHeader className="review-command-header">
        <div className="review-hero">
          <div className="review-hero-copy">
            <span className="review-kicker">Review gate</span>
            <CardTitle>
              <h2>{readiness.headline}</h2>
            </CardTitle>
            <p>{readiness.description}</p>
          </div>
          <Badge
            className={cn("review-status", `status-${classSuffix(readiness.statusLabel)}`)}
            variant="outline"
          >
            {readiness.statusLabel}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="review-command-content">
        <ButtonGroup className="review-primary-row" aria-label="Review readiness actions">
          <ReviewActionButton
            action={readiness.primaryAction}
            iconHtml={actionIcons[readiness.primaryAction.action] || ""}
            primary
          />
          <ReviewActionButton
            action={refreshAction}
            iconHtml={actionIcons[refreshAction.action] || ""}
          />
        </ButtonGroup>
        <div className="review-metrics" aria-label="Review evidence">
          {readiness.metrics.map((metric) => (
            <ReviewMetricCard key={metric.label} metric={metric} />
          ))}
        </div>
      </CardContent>
    </Card>
  )
}

function ReviewMetricCard({ metric }: { metric: ReviewMetric }) {
  return (
    <Card
      size="sm"
      className={cn("review-metric", `review-metric-${metric.tone}`)}
    >
      <span>{metric.label}</span>
      <strong>{metric.value}</strong>
    </Card>
  )
}

function ReviewGateStack({ gates }: { gates: ReviewGate[] }) {
  return (
    <div className="review-gate-stack" role="list">
      {gates.map((gate) => (
        <Card
          key={gate.id}
          size="sm"
          role="listitem"
          className={cn("review-gate", `review-gate-${gate.tone}`)}
        >
          <span className="review-gate-mark" aria-hidden="true" />
          <span className="review-gate-copy">
            <strong>{gate.label}</strong>
            <small>{gate.detail}</small>
          </span>
          <Badge className="review-gate-value" variant="outline">
            {gate.value}
          </Badge>
        </Card>
      ))}
    </div>
  )
}

function ReviewActionRail({
  actionIcons,
  groups
}: {
  actionIcons: Record<string, string>
  groups: ReviewActionGroup[]
}) {
  return (
    <nav className="review-actions" aria-label="Review actions">
      {groups.map((group) => (
        <ReviewActionGroupView
          key={group.id}
          group={group}
          actionIcons={actionIcons}
        />
      ))}
    </nav>
  )
}

function ReviewActionGroupView({
  actionIcons,
  group
}: {
  actionIcons: Record<string, string>
  group: ReviewActionGroup
}) {
  return (
    <Card
      size="sm"
      className={cn("review-action-group", `review-action-group-${group.id}`)}
      aria-label={group.label}
    >
      <CardHeader className="review-action-group-heading">
        <CardTitle>
          <strong>{group.label}</strong>
        </CardTitle>
        <span>{group.detail}</span>
      </CardHeader>
      <CardContent>
        <ButtonGroup className="review-action-list" aria-label={group.label}>
          {group.actions.map((action) => (
            <ReviewActionButton
              key={action.action}
              action={action}
              iconHtml={actionIcons[action.action] || ""}
              primary={group.id === "next" && action.tone === "primary"}
            />
          ))}
        </ButtonGroup>
      </CardContent>
    </Card>
  )
}

function ReviewActionButton({
  action,
  iconHtml,
  primary = false
}: {
  action: ReviewAction
  iconHtml: string
  primary?: boolean
}) {
  const title = action.disabled ? action.disabledReason || action.description : action.description
  const variant = action.tone === "danger" ? "destructive" : action.tone === "primary" || primary ? "default" : "outline"
  return (
    <Button
      type="button"
      className={cn(
        primary ? "review-primary-action" : "",
        action.tone === "primary" ? "primary" : "",
        action.tone === "danger" ? "danger" : "",
        `review-action-${action.tone}`
      )}
      data-action={action.action}
      data-review-icon-only="true"
      title={title}
      aria-label={`${action.label}. ${title}`}
      disabled={action.disabled}
      variant={variant}
      size={primary ? "icon-lg" : "icon-sm"}
    >
      <span
        data-icon="inline-start"
        dangerouslySetInnerHTML={{ __html: iconHtml }}
      />
      <span className="review-action-copy sr-only">
        <span>{action.label}</span>
        <small>{action.description}</small>
      </span>
    </Button>
  )
}

function classSuffix(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9_-]/g, "-")
}

export function mountReviewDrawers(options: MountReviewDrawersOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-review-drawer-root]").forEach((element) => {
    const id = element.dataset.reviewDrawerId
    if (!id) {
      return
    }
    const props = options.getProps(id)
    if (!props) {
      return
    }
    activeIds.add(id)
    const propsJson = JSON.stringify(props)
    if (lastReviewDrawerPropsJson.get(id) === propsJson) {
      return
    }
    lastReviewDrawerPropsJson.set(id, propsJson)
    let mounted = mountedRoots.get(id)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(id, mounted)
    }
    mounted.root.render(<ReviewDrawer props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
      lastReviewDrawerPropsJson.delete(id)
    }
  })
}

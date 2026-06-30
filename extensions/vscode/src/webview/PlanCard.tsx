import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"

import { Badge } from "@/webview/components/ui/badge"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle
} from "@/webview/components/ui/card"
import { Checkbox } from "@/webview/components/ui/checkbox"
import { Separator } from "@/webview/components/ui/separator"
import { cn } from "@/webview/lib/utils"

export interface PlanCardEntry {
  id: string
  title: string
  status: string
  statusClass: string
  priority?: string | undefined
}

export interface PlanCardProps {
  nodeId: string
  title: string
  detail: string
  entries: PlanCardEntry[]
  emptyText: string
}

export interface MountPlanCardsOptions {
  getProps(nodeId: string): PlanCardProps | undefined
  ids?: ReadonlySet<string> | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastPlanCardPropsJson = new Map<string, string>()

export function PlanCard({ props }: { props: PlanCardProps }) {
  return (
    <Card data-plan-card="" className="card-body plan-card" size="sm">
      <CardHeader className="plan-card-header">
        <div className="plan-card-heading">
          <CardTitle className="plan-card-title">{props.title}</CardTitle>
          <CardDescription className="plan-card-detail">{props.detail}</CardDescription>
        </div>
        <CardAction className="plan-card-action">
          <Badge className="plan-card-count" variant="outline">
            {props.entries.length} {props.entries.length === 1 ? "item" : "items"}
          </Badge>
        </CardAction>
      </CardHeader>
      <Separator className="plan-card-separator" />
      <CardContent className="plan-card-content">
        {props.entries.length ? (
          <ol className="plan-list" aria-label="Plan steps">
            {props.entries.map((entry) => (
              <PlanCardRow key={entry.id} entry={entry} />
            ))}
          </ol>
        ) : (
          <p className="muted plan-empty">{props.emptyText}</p>
        )}
      </CardContent>
    </Card>
  )
}

function PlanCardRow({ entry }: { entry: PlanCardEntry }) {
  const statusVariant = planStatusVariant(entry.status)
  return (
    <li className={cn("plan-item", `plan-${entry.statusClass}`)}>
      <Checkbox
        className="plan-status-checkbox"
        checked={planStatusChecked(entry.status)}
        disabled
        aria-invalid={statusVariant === "destructive" ? true : undefined}
        aria-label={`${entry.title}: ${entry.status}`}
      />
      <Badge className="plan-status" variant={statusVariant}>
        {entry.status}
      </Badge>
      <span className="plan-title">{entry.title}</span>
      {entry.priority ? (
        <Badge className="plan-priority" variant="outline">
          {entry.priority}
        </Badge>
      ) : null}
    </li>
  )
}

function planStatusVariant(status: string): "default" | "secondary" | "destructive" | "outline" {
  const normalized = status.toLowerCase()
  if (normalized === "completed" || normalized === "done" || normalized === "success") {
    return "secondary"
  }
  if (normalized === "failed" || normalized === "cancelled" || normalized === "canceled") {
    return "destructive"
  }
  if (normalized === "in_progress" || normalized === "active" || normalized === "running") {
    return "default"
  }
  return "outline"
}

function planStatusChecked(status: string): boolean {
  const normalized = status.toLowerCase()
  return normalized === "completed" || normalized === "done" || normalized === "success"
}

export function mountPlanCards(options: MountPlanCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-plan-card-root]").forEach((element) => {
    const nodeId = element.dataset.planNodeId
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
    const propsJson = JSON.stringify(props)
    if (lastPlanCardPropsJson.get(nodeId) === propsJson) {
      return
    }
    lastPlanCardPropsJson.set(nodeId, propsJson)
    flushSync(() => {
      mounted.root.render(<PlanCard props={props} />)
    })
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(nodeId)
      lastPlanCardPropsJson.delete(nodeId)
    }
  })
}

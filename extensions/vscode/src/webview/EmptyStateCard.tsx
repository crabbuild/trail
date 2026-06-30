import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle
} from "@/webview/components/ui/empty"
import { cn } from "@/webview/lib/utils"

export interface EmptyStateAction {
  action: string
  label: string
  iconHtml: string
  tone: "primary" | "secondary"
  disabled: boolean
}

export interface EmptyStateCardProps {
  id: string
  variant: "ready" | "filtered"
  ariaLabel: string
  iconHtml: string
  roleLabel: string
  title: string
  description: string
  actions: EmptyStateAction[]
}

export interface MountEmptyStateCardsOptions {
  getProps(id: string): EmptyStateCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastEmptyStateCardPropsJson = new Map<string, string>()

export function EmptyStateCard({ props }: { props: EmptyStateCardProps }) {
  return (
    <Empty
      data-empty-state-card=""
      aria-label={props.ariaLabel}
      className={cn("empty-state", `empty-state-${props.variant}`)}
    >
      <EmptyHeader className="empty-state-copy">
        <EmptyMedia className="empty-state-media" variant="icon">
          <span
            dangerouslySetInnerHTML={{ __html: props.iconHtml }}
          />
        </EmptyMedia>
        <Badge className="empty-state-role" variant="outline">
          {props.roleLabel}
        </Badge>
        <EmptyTitle>{props.title}</EmptyTitle>
        <EmptyDescription>{props.description}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent className="mx-auto empty-actions" aria-label="Suggested next actions">
        {props.actions.map((action) => (
          <Button
            key={action.action}
            type="button"
            data-action={action.action}
            title={action.label}
            disabled={action.disabled}
            variant={action.tone === "primary" ? "default" : "outline"}
            size="sm"
            className={cn("empty-action", `empty-action-${action.tone}`)}
          >
            <span
              data-icon="inline-start"
              dangerouslySetInnerHTML={{ __html: action.iconHtml }}
            />
            <span>
              <b>{action.label}</b>
            </span>
          </Button>
        ))}
      </EmptyContent>
    </Empty>
  )
}

export function mountEmptyStateCards(options: MountEmptyStateCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-empty-state-card-root]").forEach((element) => {
    const id = element.dataset.emptyStateId
    if (!id) {
      return
    }
    const props = options.getProps(id)
    if (!props) {
      return
    }
    activeIds.add(id)
    let mounted = mountedRoots.get(id)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(id, mounted)
    }
    const propsJson = JSON.stringify(props)
    if (lastEmptyStateCardPropsJson.get(id) === propsJson) {
      activeIds.add(id)
      return
    }
    lastEmptyStateCardPropsJson.set(id, propsJson)
    mounted.root.render(<EmptyStateCard props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
      lastEmptyStateCardPropsJson.delete(id)
    }
  })
}

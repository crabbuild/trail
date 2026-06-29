import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import { Button } from "@/webview/components/ui/button"
import { ButtonGroup } from "@/webview/components/ui/button-group"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger
} from "@/webview/components/ui/tooltip"
import { cn } from "@/webview/lib/utils"

export type InlineActionTone = "default" | "primary" | "provider" | "lane" | "review" | "danger"

export interface InlineAction {
  action: string
  label: string
  detail?: string | undefined
  tone?: InlineActionTone | undefined
  title?: string | undefined
  ariaLabel?: string | undefined
  tooltip?: React.ReactNode | undefined
  icon?: React.ReactNode | undefined
  iconHtml?: string | undefined
  iconOnly?: boolean | undefined
  disabled?: boolean | undefined
  className?: string | undefined
  data?: Record<string, string | undefined> | undefined
}

export interface InlineActionsProps {
  id: string
  className?: string | undefined
  ariaLabel: string
  actions: InlineAction[]
  onAction?: ((action: InlineAction, event: React.MouseEvent<HTMLButtonElement>) => void) | undefined
}

export interface MountInlineActionsOptions {
  getProps(id: string): InlineActionsProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastInlineActionsPropsJson = new Map<string, string>()

export function InlineActions({ props }: { props: InlineActionsProps }) {
  return (
    <TooltipProvider>
      <ButtonGroup
        className={cn("inline-actions", props.className)}
        aria-label={props.ariaLabel}
        data-inline-actions=""
      >
        {props.actions.map((action) => (
          <InlineActionButton
            key={inlineActionKey(action)}
            action={action}
            onAction={props.onAction}
          />
        ))}
      </ButtonGroup>
    </TooltipProvider>
  )
}

function InlineActionButton({
  action,
  onAction
}: {
  action: InlineAction
  onAction?: ((action: InlineAction, event: React.MouseEvent<HTMLButtonElement>) => void) | undefined
}) {
  const button = (
    <Button
      type="button"
      className={cn(
        action.tone ? `inline-action-${action.tone}` : undefined,
        action.className
      )}
      data-action={action.action}
      data-inline-icon-only={action.iconOnly ? "true" : undefined}
      disabled={action.disabled}
      title={action.title ?? (action.tooltip ? undefined : action.label)}
      aria-label={action.ariaLabel ?? action.label}
      variant={inlineActionVariant(action.tone)}
      size={action.iconOnly ? "icon-sm" : "sm"}
      onClick={onAction ? (event) => onAction(action, event) : undefined}
      {...inlineActionDataAttributes(action.data)}
    >
      {action.icon ? action.icon : null}
      {!action.icon && action.iconHtml ? (
        <span
          data-icon="inline-start"
          dangerouslySetInnerHTML={{ __html: action.iconHtml }}
        />
      ) : null}
      {action.iconOnly ? <span className="sr-only">{action.label}</span> : <span>{action.label}</span>}
      {!action.iconOnly && action.detail ? <small>{action.detail}</small> : null}
    </Button>
  )

  const tooltip = action.tooltip ?? action.detail
  if (!tooltip) {
    return button
  }

  return (
    <Tooltip>
      <TooltipTrigger render={button} />
      <TooltipContent className="inline-action-tooltip" side="top" align="start">
        {tooltip}
      </TooltipContent>
    </Tooltip>
  )
}

function inlineActionVariant(tone: InlineActionTone | undefined): "default" | "outline" | "destructive" {
  if (tone === "primary") {
    return "default"
  }
  if (tone === "danger") {
    return "destructive"
  }
  return "outline"
}

function inlineActionDataAttributes(data: Record<string, string | undefined> | undefined): Record<string, string> {
  if (!data) {
    return {}
  }
  return Object.fromEntries(
    Object.entries(data)
      .filter((entry): entry is [string, string] => typeof entry[1] === "string")
      .map(([key, value]) => [`data-${key}`, value])
  )
}

function inlineActionKey(action: InlineAction): string {
  return `${action.action}:${action.label}:${JSON.stringify(action.data ?? {})}`
}

export function mountInlineActions(options: MountInlineActionsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-inline-actions-root]").forEach((element) => {
    const id = element.dataset.inlineActionsId
    if (!id) {
      return
    }
    const props = options.getProps(id)
    if (!props) {
      return
    }
    activeIds.add(id)
    const propsJson = JSON.stringify(props)
    if (lastInlineActionsPropsJson.get(id) === propsJson) {
      return
    }
    lastInlineActionsPropsJson.set(id, propsJson)
    let mounted = mountedRoots.get(id)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(id, mounted)
    }
    mounted.root.render(<InlineActions props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
      lastInlineActionsPropsJson.delete(id)
    }
  })
}

export function cleanupDetachedInlineActions(): void {
  mountedRoots.forEach((mounted, id) => {
    if (!mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
      lastInlineActionsPropsJson.delete(id)
    }
  })
}

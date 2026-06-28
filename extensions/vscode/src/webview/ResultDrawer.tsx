import * as React from "react"
import { createPortal } from "react-dom"
import { createRoot, type Root } from "react-dom/client"
import { XIcon } from "lucide-react"

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from "@/webview/components/ui/accordion"
import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle
} from "@/webview/components/ui/drawer"
import { cn } from "@/webview/lib/utils"

export interface ResultDrawerProps {
  title: string
  description: string
  badgeLabel?: string | undefined
  className?: string | undefined
  closeLabel: string
  bodyHtml: string
  widgets?: ResultDrawerWidget[] | undefined
}

export interface ResultDrawerAccordionItem {
  id: string
  title: string
  contentHtml: string
  className?: string | undefined
  triggerClassName?: string | undefined
  contentClassName?: string | undefined
}

export interface ResultDrawerAccordionWidget {
  type: "accordion"
  id: string
  className?: string | undefined
  multiple?: boolean | undefined
  defaultOpenIds?: string[] | undefined
  items: ResultDrawerAccordionItem[]
}

export type ResultDrawerWidget = ResultDrawerAccordionWidget

export interface MountResultDrawerOptions {
  props: ResultDrawerProps
  onClose(): void
}

interface MountedDrawer {
  element: HTMLElement
  root: Root
}

let mountedDrawer: MountedDrawer | undefined

export function ResultDrawer({
  onClose,
  props
}: {
  onClose: () => void
  props: ResultDrawerProps
}) {
  const bodyRef = React.useRef<HTMLDivElement>(null)
  return (
    <Drawer
      direction="right"
      open
      onOpenChange={(open) => {
        if (!open) {
          onClose()
        }
      }}
    >
      <DrawerContent className={cn("json-drawer result-drawer", props.className)}>
        <DrawerHeader className="drawer-header result-drawer-header">
          <div className="result-drawer-title">
            <DrawerTitle>{props.title}</DrawerTitle>
            <DrawerDescription>{props.description}</DrawerDescription>
          </div>
          <div className="result-drawer-actions">
            {props.badgeLabel ? (
              <Badge className="result-drawer-badge" variant="outline">
                {props.badgeLabel}
              </Badge>
            ) : null}
            <Button
              aria-label={props.closeLabel}
              data-action="closeDrawer"
              size="icon-sm"
              type="button"
              variant="ghost"
            >
              <XIcon aria-hidden="true" data-icon="inline-start" />
            </Button>
          </div>
        </DrawerHeader>
        <div
          ref={bodyRef}
          className="result-drawer-body"
          dangerouslySetInnerHTML={{ __html: props.bodyHtml }}
        />
        {props.widgets?.length ? (
          <ResultDrawerWidgetPortals
            bodyHtml={props.bodyHtml}
            bodyRef={bodyRef}
            widgets={props.widgets}
          />
        ) : null}
      </DrawerContent>
    </Drawer>
  )
}

function ResultDrawerWidgetPortals({
  bodyHtml,
  bodyRef,
  widgets
}: {
  bodyHtml: string
  bodyRef: React.RefObject<HTMLElement | null>
  widgets: ResultDrawerWidget[]
}) {
  const [hosts, setHosts] = React.useState(() => new Map<string, HTMLElement>())

  React.useLayoutEffect(() => {
    const body = bodyRef.current
    const nextHosts = new Map<string, HTMLElement>()
    if (body) {
      body.querySelectorAll<HTMLElement>("[data-result-drawer-widget]").forEach((host) => {
        const id = host.dataset.resultDrawerWidget
        if (id) {
          nextHosts.set(id, host)
        }
      })
    }
    setHosts(nextHosts)
  }, [bodyHtml, bodyRef, widgets])

  return (
    <>
      {widgets.map((widget) => {
        const host = hosts.get(widget.id)
        if (!host) {
          return null
        }
        return createPortal(<ResultDrawerWidgetView widget={widget} />, host, widget.id)
      })}
    </>
  )
}

function ResultDrawerWidgetView({ widget }: { widget: ResultDrawerWidget }) {
  if (widget.type === "accordion") {
    return <ResultDrawerAccordionWidgetView widget={widget} />
  }
  return null
}

function ResultDrawerAccordionWidgetView({ widget }: { widget: ResultDrawerAccordionWidget }) {
  const defaultValue = widget.defaultOpenIds?.length ? widget.defaultOpenIds : undefined
  return (
    <Accordion className={widget.className} defaultValue={defaultValue} multiple={widget.multiple}>
      {widget.items.map((item) => (
        <AccordionItem className={item.className} key={item.id} value={item.id}>
          <AccordionTrigger className={item.triggerClassName}>
            <span>{item.title}</span>
          </AccordionTrigger>
          <AccordionContent className={item.contentClassName} keepMounted>
            <div dangerouslySetInnerHTML={{ __html: item.contentHtml }} />
          </AccordionContent>
        </AccordionItem>
      ))}
    </Accordion>
  )
}

export function mountResultDrawer({ onClose, props }: MountResultDrawerOptions): void {
  closeResultDrawer()
  const element = document.createElement("div")
  element.dataset.resultDrawerHost = ""
  document.body.append(element)
  const root = createRoot(element)
  mountedDrawer = { element, root }
  root.render(<ResultDrawer onClose={onClose} props={props} />)
  window.requestAnimationFrame(() => {
    activeResultDrawer()?.querySelector<HTMLElement>("[data-action='closeDrawer']")?.focus()
  })
}

export function closeResultDrawer(): void {
  if (!mountedDrawer) {
    return
  }
  mountedDrawer.root.unmount()
  mountedDrawer.element.remove()
  mountedDrawer = undefined
}

function activeResultDrawer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(".json-drawer")
}

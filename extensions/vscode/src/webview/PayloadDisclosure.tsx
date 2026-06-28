import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from "@/webview/components/ui/accordion"
import { cn } from "@/webview/lib/utils"

export interface PayloadDisclosureProps {
  id: string
  className: string
  label: string
  bodyHtml: string
  defaultOpen?: boolean | undefined
}

export interface MountPayloadDisclosuresOptions {
  getProps(id: string): PayloadDisclosureProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()

export function PayloadDisclosure({ props }: { props: PayloadDisclosureProps }) {
  return (
    <Accordion
      className={cn(props.className, "payload-disclosure")}
      defaultValue={props.defaultOpen ? [props.id] : undefined}
    >
      <AccordionItem className="payload-disclosure-item" value={props.id}>
        <AccordionTrigger className={cn("payload-summary", `${props.className}-summary`)}>
          <span>{props.label}</span>
        </AccordionTrigger>
        <AccordionContent
          className={cn("payload-panel", `${props.className}-panel`)}
          keepMounted
        >
          <div dangerouslySetInnerHTML={{ __html: props.bodyHtml }} />
        </AccordionContent>
      </AccordionItem>
    </Accordion>
  )
}

export function mountPayloadDisclosures(options: MountPayloadDisclosuresOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-payload-disclosure-root]").forEach((element) => {
    const id = element.dataset.payloadDisclosureId
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
    mounted.root.render(<PayloadDisclosure props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
    }
  })
}

export function cleanupDetachedPayloadDisclosures(): void {
  mountedRoots.forEach((mounted, id) => {
    if (!mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
    }
  })
}

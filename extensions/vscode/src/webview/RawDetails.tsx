import * as React from "react"

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from "@/webview/components/ui/accordion"
import { cn } from "@/webview/lib/utils"

export interface RawDetailsView {
  id: string
  label: string
  contentHtml: string
  truncatedText?: string | undefined
  defaultOpen?: boolean | undefined
}

export function RawDetails({
  className,
  details,
  onOpenChange,
  open
}: {
  className?: string | undefined
  details: RawDetailsView
  onOpenChange?: ((open: boolean) => void) | undefined
  open?: boolean | undefined
}) {
  const value = open === undefined ? undefined : open ? [details.id] : []
  const defaultValue = details.defaultOpen ? [details.id] : undefined
  return (
    <Accordion
      className={cn("raw raw-accordion", className)}
      value={value}
      defaultValue={defaultValue}
      onValueChange={(nextValue) => onOpenChange?.(nextValue.includes(details.id))}
    >
      <AccordionItem className="raw-item" value={details.id}>
        <AccordionTrigger className="raw-summary">
          <span>{details.label}</span>
        </AccordionTrigger>
        <AccordionContent className="raw-panel" keepMounted>
          <div dangerouslySetInnerHTML={{ __html: details.contentHtml }} />
          {details.truncatedText ? <p className="muted">{details.truncatedText}</p> : null}
        </AccordionContent>
      </AccordionItem>
    </Accordion>
  )
}

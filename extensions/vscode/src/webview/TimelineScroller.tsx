import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"

import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport
} from "@/webview/components/ui/message-scroller"
import { cn } from "@/webview/lib/utils"

export interface TimelineScrollerProps {
  items: TimelineScrollerItemView[]
}

export interface TimelineScrollerItemView {
  id: string
  html: string
  className?: string | undefined
  scrollAnchor?: boolean | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

let mountedRoot: MountedRoot | undefined

export function TimelineScroller({ props }: { props: TimelineScrollerProps }) {
  return (
    <MessageScrollerProvider>
      <MessageScroller className="timeline-message-scroller">
        <MessageScrollerViewport
          id="timeline"
          className="timeline"
          aria-label="Agent transcript"
          tabIndex={-1}
        >
          <MessageScrollerContent className="timeline-scroller-content">
            {props.items.map((item) => (
              <MessageScrollerItem
                key={item.id}
                messageId={item.id}
                scrollAnchor={Boolean(item.scrollAnchor)}
                className={cn("timeline-scroller-row", item.className)}
                dangerouslySetInnerHTML={{ __html: item.html }}
              />
            ))}
          </MessageScrollerContent>
        </MessageScrollerViewport>
        <MessageScrollerButton />
      </MessageScroller>
    </MessageScrollerProvider>
  )
}

export function mountTimelineScroller(element: HTMLElement, props: TimelineScrollerProps): void {
  if (!mountedRoot || mountedRoot.element !== element) {
    mountedRoot?.root.unmount()
    mountedRoot = {
      element,
      root: createRoot(element)
    }
  }
  const root = mountedRoot.root
  flushSync(() => {
    root.render(<TimelineScroller props={props} />)
  })
}

export function cleanupTimelineScroller(): void {
  if (mountedRoot && !mountedRoot.element.isConnected) {
    mountedRoot.root.unmount()
    mountedRoot = undefined
  }
}

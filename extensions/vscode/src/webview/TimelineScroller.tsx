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
  preserveDom?: boolean | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const TIMELINE_SCROLL_MARGIN = 72
const TIMELINE_PREVIOUS_ITEM_PEEK = 64
const TIMELINE_SCROLL_EDGE_THRESHOLD = 24

let mountedRoot: MountedRoot | undefined
let lastTimelineScrollerPropsJson = ""

export function TimelineScroller({ props }: { props: TimelineScrollerProps }) {
  return (
    <MessageScrollerProvider
      autoScroll
      defaultScrollPosition="last-anchor"
      scrollEdgeThreshold={TIMELINE_SCROLL_EDGE_THRESHOLD}
      scrollMargin={TIMELINE_SCROLL_MARGIN}
      scrollPreviousItemPeek={TIMELINE_PREVIOUS_ITEM_PEEK}
    >
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
              >
                {item.preserveDom ? (
                  <StableHtmlSlot shellSignature={stableHtmlShellSignature(item.html)} slotId={item.id} html={item.html} />
                ) : (
                  <div className="stable-html-slot" dangerouslySetInnerHTML={{ __html: item.html }} />
                )}
              </MessageScrollerItem>
            ))}
          </MessageScrollerContent>
        </MessageScrollerViewport>
        <MessageScrollerButton />
      </MessageScroller>
    </MessageScrollerProvider>
  )
}

const StableHtmlSlot = React.memo(
  function StableHtmlSlot({
    html,
    shellSignature: _shellSignature,
    slotId
  }: {
    html: string
    shellSignature: string
    slotId: string
  }) {
    const rootRef = React.useRef<HTMLDivElement | null>(null)
    const initialHtml = React.useRef(html)

    React.useLayoutEffect(() => {
      syncStableHtmlShell(rootRef.current, html)
    }, [html, _shellSignature])

    return (
      <div
        ref={rootRef}
        className="stable-html-slot"
        data-stable-html-slot={slotId}
        dangerouslySetInnerHTML={{ __html: initialHtml.current }}
      />
    )
  },
  (previous, next) => previous.slotId === next.slotId && previous.shellSignature === next.shellSignature
)

export function mountTimelineScroller(element: HTMLElement, props: TimelineScrollerProps): void {
  const currentJson = timelineScrollerPropsSignature(props)
  if (
    currentJson === lastTimelineScrollerPropsJson &&
    mountedRoot?.element === element &&
    mountedRoot.element.isConnected
  ) {
    return
  }
  lastTimelineScrollerPropsJson = currentJson
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

function timelineScrollerPropsSignature(props: TimelineScrollerProps): string {
  return JSON.stringify({
    items: props.items.map((item) => ({
      id: item.id,
      className: item.className,
      scrollAnchor: item.scrollAnchor,
      preserveDom: item.preserveDom,
      html: item.preserveDom ? stableHtmlShellSignature(item.html) : item.html
    }))
  })
}

function stableHtmlShellSignature(html: string): string {
  return html.trim().match(/^<([a-z][\w:-]*)(?:\s[^>]*)?>/i)?.[0] ?? html
}

function syncStableHtmlShell(root: HTMLDivElement | null, html: string): void {
  if (!root) {
    return
  }
  const next = stableHtmlFirstElement(html)
  const current = root.firstElementChild
  if (!next || !current || next.tagName !== current.tagName) {
    root.innerHTML = html
    return
  }
  syncElementAttributes(current, next)
}

function stableHtmlFirstElement(html: string): Element | undefined {
  const template = document.createElement("template")
  template.innerHTML = html.trim()
  return template.content.firstElementChild ?? undefined
}

function syncElementAttributes(current: Element, next: Element): void {
  for (const attr of Array.from(current.attributes)) {
    if (!next.hasAttribute(attr.name)) {
      current.removeAttribute(attr.name)
    }
  }
  for (const attr of Array.from(next.attributes)) {
    if (current.getAttribute(attr.name) !== attr.value) {
      current.setAttribute(attr.name, attr.value)
    }
  }
}

export function cleanupTimelineScroller(): void {
  if (mountedRoot && !mountedRoot.element.isConnected) {
    mountedRoot.root.unmount()
    mountedRoot = undefined
    lastTimelineScrollerPropsJson = ""
  }
}

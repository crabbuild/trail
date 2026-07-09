import * as React from "react"
import {
  Streamdown,
  type Components,
  type ControlsConfig
} from "streamdown"

import { cn } from "@/webview/lib/utils"

export const TRAIL_STREAMDOWN_UPDATE_EVENT = "trail-streamdown-update"

export interface StreamdownMarkdownProps {
  className?: string | undefined
  streaming?: boolean | undefined
  text?: string | undefined
}

export type StreamdownMarkdownElement = HTMLDivElement & {
  __trailQueueStreamdownText?: ((text: string) => void) | undefined
  __trailStreamingText?: string | undefined
}

type StreamdownUpdateEvent = CustomEvent<{ text?: unknown }>

const streamdownControls: ControlsConfig = false
const streamdownRemend = { linkMode: "text-only" } as const
const streamdownLinkSafety = { enabled: false } as const

const streamdownComponents = {
  a: StreamdownAnchor
} satisfies Components

function StreamdownAnchor({
  children,
  href,
  node: _node,
  ...props
}: React.AnchorHTMLAttributes<HTMLAnchorElement> & { node?: unknown }) {
  const safeHref = safeStreamdownHref(href)
  if (!safeHref) {
    return <>{children}</>
  }
  return (
    <a
      {...props}
      href={safeHref}
      rel="noreferrer"
      target={safeHref.startsWith("#") ? undefined : "_blank"}
    >
      {children}
    </a>
  )
}

function safeStreamdownHref(value: unknown): string | undefined {
  if (typeof value !== "string") {
    return undefined
  }
  const trimmed = value.trim()
  if (!trimmed || trimmed === "streamdown:incomplete-link") {
    return undefined
  }
  if (trimmed.startsWith("#")) {
    return trimmed
  }
  try {
    const url = new URL(trimmed)
    return ["http:", "https:", "mailto:"].includes(url.protocol) ? trimmed : undefined
  } catch {
    return undefined
  }
}

export const StreamdownMarkdown = React.memo(function StreamdownMarkdown({
  className,
  streaming = false,
  text = ""
}: StreamdownMarkdownProps) {
  const rootRef = React.useRef<StreamdownMarkdownElement | null>(null)
  const queuedTextRef = React.useRef(text)
  const frameRef = React.useRef<number | undefined>(undefined)
  const [renderedText, setRenderedText] = React.useState(text)

  const flushQueuedText = React.useCallback(() => {
    frameRef.current = undefined
    setRenderedText((current) => (current === queuedTextRef.current ? current : queuedTextRef.current))
  }, [])

  const queueText = React.useCallback((nextText: string) => {
    if (queuedTextRef.current === nextText) {
      return
    }
    queuedTextRef.current = nextText
    if (rootRef.current) {
      rootRef.current.__trailStreamingText = nextText
    }
    if (frameRef.current !== undefined) {
      return
    }
    const requestFrame =
      typeof window !== "undefined" && typeof window.requestAnimationFrame === "function"
        ? window.requestAnimationFrame.bind(window)
        : (callback: FrameRequestCallback) => Number(globalThis.setTimeout(() => callback(Date.now()), 16))
    frameRef.current = requestFrame(flushQueuedText)
  }, [flushQueuedText])

  const setRoot = React.useCallback((node: StreamdownMarkdownElement | null) => {
    if (rootRef.current && rootRef.current !== node) {
      rootRef.current.__trailQueueStreamdownText = undefined
      rootRef.current.__trailStreamingText = undefined
    }
    rootRef.current = node
    if (node) {
      node.__trailStreamingText = queuedTextRef.current
      node.__trailQueueStreamdownText = queueText
    }
  }, [queueText])

  React.useEffect(() => {
    queueText(text)
  }, [queueText, text])

  React.useEffect(() => {
    const root = rootRef.current
    if (!root) {
      return undefined
    }
    const onUpdate = (event: Event) => {
      const nextText = (event as StreamdownUpdateEvent).detail?.text
      if (typeof nextText === "string") {
        queueText(nextText)
      }
    }
    root.addEventListener(TRAIL_STREAMDOWN_UPDATE_EVENT, onUpdate)
    return () => {
      root.removeEventListener(TRAIL_STREAMDOWN_UPDATE_EVENT, onUpdate)
    }
  }, [queueText])

  React.useEffect(() => {
    return () => {
      if (frameRef.current !== undefined && typeof window !== "undefined" && typeof window.cancelAnimationFrame === "function") {
        window.cancelAnimationFrame(frameRef.current)
      }
      if (rootRef.current) {
        rootRef.current.__trailQueueStreamdownText = undefined
        rootRef.current.__trailStreamingText = undefined
      }
    }
  }, [])

  return (
    <div
      ref={setRoot}
      className={cn("markdown streaming-markdown streamdown-markdown", className)}
      data-streamdown-markdown=""
      data-streaming-markdown=""
    >
      <Streamdown
        components={streamdownComponents}
        controls={streamdownControls}
        isAnimating={streaming}
        lineNumbers={false}
        linkSafety={streamdownLinkSafety}
        mode="streaming"
        parseIncompleteMarkdown={streaming}
        remend={streamdownRemend}
      >
        {renderedText}
      </Streamdown>
    </div>
  )
}, (previous, next) =>
  previous.className === next.className &&
  previous.streaming === next.streaming &&
  previous.text === next.text
)

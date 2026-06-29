import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"

import {
  Message,
  MessageContent,
  MessageGroup
} from "@/webview/components/ui/message"
import { cn } from "@/webview/lib/utils"

export interface MessageCardProps {
  nodeId: string
  role: "user" | "assistant"
  streaming: boolean
  contentHtml: string
  isSticky?: boolean
}

export interface MountMessageCardsOptions {
  getProps(nodeId: string): MessageCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
const lastMessageCardPropsJson = new Map<string, string>()

export function MessageCard({ props }: { props: MessageCardProps }) {
  const isUser = props.role === "user"
  const label = isUser ? "You" : "Agent"
  return (
    <MessageGroup data-message-card="">
      <Message
        align={isUser ? "end" : "start"}
        aria-label={`${label} message`}
        className={cn(
          "transcript-message",
          `transcript-message-${props.role}`,
          isUser && "transcript-message-user-bg",
          props.isSticky && "transcript-message-sticky"
        )}
      >
        <MessageContent
          className={cn(
            "transcript-message-content",
            isUser && "transcript-message-content-user"
          )}
        >
          <div
            className="markdown"
            dangerouslySetInnerHTML={{ __html: props.contentHtml }}
          />
          {props.streaming ? (
            <span className="sr-only" role="status">Streaming response</span>
          ) : null}
        </MessageContent>
      </Message>
    </MessageGroup>
  )
}

export function mountMessageCards(options: MountMessageCardsOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-message-card-root]").forEach((element) => {
    const nodeId = element.dataset.messageNodeId
    if (!nodeId) {
      return
    }
    const props = options.getProps(nodeId)
    if (!props) {
      return
    }
    const currentJson = JSON.stringify(props)
    let mounted = mountedRoots.get(nodeId)
    if (
      currentJson === lastMessageCardPropsJson.get(nodeId) &&
      mounted?.element === element &&
      mounted.element.isConnected
    ) {
      activeIds.add(nodeId)
      return
    }
    lastMessageCardPropsJson.set(nodeId, currentJson)
    activeIds.add(nodeId)
    if (!mounted || mounted.element !== element) {
      mounted?.root.unmount()
      mounted = {
        element,
        root: createRoot(element)
      }
      mountedRoots.set(nodeId, mounted)
    }
    flushSync(() => {
      mounted.root.render(<MessageCard props={props} />)
    })
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(nodeId)
      lastMessageCardPropsJson.delete(nodeId)
    }
  })
}

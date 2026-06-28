import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot, type Root } from "react-dom/client"

import { Badge } from "@/webview/components/ui/badge"
import { Marker, MarkerContent, MarkerIcon } from "@/webview/components/ui/marker"
import {
  Message,
  MessageAvatar,
  MessageContent,
  MessageFooter,
  MessageGroup,
  MessageHeader
} from "@/webview/components/ui/message"
import { Spinner } from "@/webview/components/ui/spinner"
import { cn } from "@/webview/lib/utils"

export interface MessageCardProps {
  nodeId: string
  role: "user" | "assistant"
  streaming: boolean
  contentHtml: string
}

export interface MountMessageCardsOptions {
  getProps(nodeId: string): MessageCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()

export function MessageCard({ props }: { props: MessageCardProps }) {
  const label = props.role === "user" ? "You" : "Agent"
  const avatarLabel = props.role === "user" ? "You" : "AI"
  return (
    <MessageGroup data-message-card="">
      <Message
        align={props.role === "user" ? "end" : "start"}
        className={cn("transcript-message", `transcript-message-${props.role}`)}
      >
        <MessageAvatar
          className={cn(
            "message-avatar border border-border text-[10px] font-semibold uppercase text-muted-foreground [letter-spacing:0]",
            `message-avatar-${props.role}`
          )}
        >
          <span aria-hidden="true">{avatarLabel}</span>
        </MessageAvatar>
        <MessageContent className="transcript-message-content">
          <MessageHeader className="message-header">
            <Marker className="message-role-marker" render={<span />}>
              <MarkerContent className="message-role-content">{label}</MarkerContent>
            </Marker>
            {props.streaming ? (
              <Badge className="message-streaming-badge" variant="secondary">
                <Spinner data-icon="inline-start" aria-hidden="true" role="presentation" />
                streaming
              </Badge>
            ) : null}
          </MessageHeader>
          <div
            className="markdown"
            dangerouslySetInnerHTML={{ __html: props.contentHtml }}
          />
          {props.streaming ? (
            <MessageFooter className="transcript-message-footer">
              <Marker className="message-streaming-status" role="status" render={<span />}>
                <MarkerIcon>
                  <Spinner aria-hidden="true" role="presentation" />
                </MarkerIcon>
                <MarkerContent>Live response</MarkerContent>
              </Marker>
            </MessageFooter>
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
    flushSync(() => {
      mounted.root.render(<MessageCard props={props} />)
    })
  })

  mountedRoots.forEach((mounted, nodeId) => {
    if (!activeIds.has(nodeId) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(nodeId)
    }
  })
}

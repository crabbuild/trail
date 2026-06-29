import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import { ButtonGroup } from "@/webview/components/ui/button-group"
import { Card, CardContent, CardFooter } from "@/webview/components/ui/card"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/webview/components/ui/collapsible"
import { cn } from "@/webview/lib/utils"
import { useFloatingDisclosure } from "./floatingDisclosure"

export type ComposerSendMode = "fast" | "draft"
export type ComposerStatusTone = "ready" | "context" | "running" | "waiting" | "warning"
export type ComposerDraftTone = "empty" | "ready" | "warning" | "limit"

export interface ComposerStatusView {
  tone: ComposerStatusTone
  label: string
  detail: string
}

export interface ComposerDraftView {
  tone: ComposerDraftTone
  label: string
  detail: string
  maxChars: number
  meterValue: number
  meterPercent: number
}

export interface ComposerRailItemView {
  id: string
  label: string
  value: string
  tone: string
}

export interface ComposerPresetView {
  id: string
  label: string
  detail: string
  iconHtml: string
}

export interface ComposerAttachmentView {
  id: string
  kind: string
  label: string
  mode: string
  title: string
}

export interface ComposerIconActionView {
  action: string
  label: string
  iconHtml: string
  disabled: boolean
}

export interface ComposerCardProps {
  id: string
  status: ComposerStatusView
  draft: ComposerDraftView
  draftValue: string
  placeholder: string
  keyShortcuts: string
  maxChars: number
  controlsDisabled: boolean
  sendBlockedReason?: string | undefined
  metricsText: string
  attachments: ComposerAttachmentView[]
  attachmentSummary: string
  railItems: ComposerRailItemView[]
  presets: ComposerPresetView[]
  sendMode: ComposerSendMode
  contextUsageHtml: string
  sessionControlsHtml: string
  contextActions: ComposerIconActionView[]
  rewindIconHtml: string
  sendIconHtml: string
  clearIconHtml: string
  settingsIconHtml: string
}

export interface MountComposerCardOptions {
  getProps(id: string): ComposerCardProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
let lastComposerCardPropsJson = ""

export function ComposerCard({ props }: { props: ComposerCardProps }) {
  return (
    <Card
      size="sm"
      data-composer-card=""
      className={cn("composer-box", `composer-box-${props.status.tone}`)}
    >
      <CardContent className="composer-card-content">
        <ComposerStatus status={props.status} />
        <ComposerAttachmentShelf props={props} />
        <ComposerInput props={props} />
      </CardContent>
      <CardFooter className="composer-actions">
        <span id="composer-meta" className="sr-only" data-composer-meta="">
          {props.metricsText}
        </span>
        <span
          id="composer-send-hint"
          className="sr-only"
          data-composer-empty-reason=""
          hidden={!props.sendBlockedReason}
        >
          {props.sendBlockedReason || ""}
        </span>
        <ButtonGroup className="composer-icon-tools" aria-label="Context attachments">
          {props.contextActions.map((action) => (
            <ComposerIconButton
              key={action.action}
              action={action.action}
              label={action.label}
              iconHtml={action.iconHtml}
              disabled={action.disabled}
            />
          ))}
        </ButtonGroup>
        <span
          className="composer-context-gauge-host"
          dangerouslySetInnerHTML={{ __html: props.contextUsageHtml }}
        />
        <ButtonGroup className="composer-action-group" aria-label="Prompt actions">
          <ComposerSessionControls props={props} />
          <ComposerIconButton
            action="rewind"
            label="Rewind latest turn"
            iconHtml={props.rewindIconHtml}
            disabled={false}
          />
          <ComposerIconButton
            action="send"
            label={props.sendBlockedReason || "Send prompt"}
            iconHtml={props.sendIconHtml}
            disabled={Boolean(props.sendBlockedReason)}
            className="primary send-button"
          />
        </ButtonGroup>
      </CardFooter>
    </Card>
  )
}

function ComposerAttachmentShelf({ props }: { props: ComposerCardProps }) {
  if (!props.attachments.length) {
    return null
  }
  return (
    <div className="attachment-shelf" aria-label="Attached context">
      <div className="attachment-shelf-header">
        <span>Attached context</span>
        <small>{props.attachmentSummary}</small>
      </div>
      <div className="attachment-list">
        {props.attachments.map((attachment) => (
          <Badge
            key={attachment.id}
            className="attachment-chip"
            title={attachment.title}
            variant="outline"
          >
            <b>{attachment.kind}</b>
            <span>{attachment.label}</span>
            <small>{attachment.mode}</small>
            <Button
              type="button"
              className="micro"
              data-action="removeAttachment"
              data-attachment-id={attachment.id}
              data-composer-icon-only="true"
              title={`Remove ${attachment.label}`}
              aria-label={`Remove ${attachment.label}`}
              variant="ghost"
              size="icon-xs"
            >
              <span
                data-icon="inline-start"
                dangerouslySetInnerHTML={{ __html: props.clearIconHtml }}
              />
            </Button>
          </Badge>
        ))}
      </div>
    </div>
  )
}

function ComposerStatus({ status }: { status: ComposerStatusView }) {
  return (
    <div
      id="composer-status"
      className="sr-only"
      role="status"
      aria-live="polite"
    >
      {status.label}. {status.detail}
    </div>
  )
}

function ComposerInput({ props }: { props: ComposerCardProps }) {
  return (
    <label className={cn("composer-input-frame", `composer-input-frame-${props.draft.tone}`)}>
      <span className="sr-only">Prompt message</span>
      <textarea
        className="composer-input"
        rows={3}
        maxLength={props.maxChars}
        placeholder={props.placeholder}
        aria-describedby="composer-status composer-draft-state composer-meta composer-send-hint"
        aria-keyshortcuts={props.keyShortcuts}
        autoComplete="off"
        spellCheck={true}
        aria-invalid={props.draft.tone === "limit" ? "true" : undefined}
        disabled={props.controlsDisabled}
        defaultValue={props.draftValue}
      />
      <span id="composer-draft-state" className="composer-input-footer">
        <span className="composer-draft-copy">
          <strong>{props.draft.label}</strong>
          <span>{props.draft.detail}</span>
        </span>
        <span
          className="composer-meter"
          role="meter"
          aria-label="Prompt length"
          aria-valuemin={0}
          aria-valuemax={props.draft.maxChars}
          aria-valuenow={props.draft.meterValue}
          style={{ "--composer-meter": `${props.draft.meterPercent}%` } as React.CSSProperties}
        >
          <span aria-hidden="true" />
        </span>
      </span>
    </label>
  )
}

function ComposerSessionControls({ props }: { props: ComposerCardProps }) {
  const disclosure = useFloatingDisclosure()
  if (!props.sessionControlsHtml) {
    return null
  }
  return (
    <Collapsible
      className="composer-controls"
      data-floating-open={disclosure.open ? "true" : undefined}
      open={disclosure.open}
      onOpenChange={disclosure.setOpen}
      ref={disclosure.rootRef}
    >
      <CollapsibleTrigger
        className="composer-controls-summary"
        title="Agent controls"
        ref={disclosure.triggerRef}
      >
        <span
          data-icon="inline-start"
          dangerouslySetInnerHTML={{ __html: props.settingsIconHtml }}
        />
        <span className="sr-only">Agent controls</span>
      </CollapsibleTrigger>
      <CollapsibleContent
        keepMounted
        className="composer-session"
        dangerouslySetInnerHTML={{ __html: props.sessionControlsHtml }}
      />
    </Collapsible>
  )
}

function ComposerIconButton({
  action,
  className,
  disabled,
  extraAttrs,
  iconHtml,
  label
}: {
  action: string
  className?: string | undefined
  disabled: boolean
  extraAttrs?: Record<string, string> | undefined
  iconHtml: string
  label: string
}) {
  return (
    <Button
      type="button"
      className={className}
      data-action={action}
      data-composer-icon-only="true"
      title={label}
      aria-label={label}
      disabled={disabled}
      variant={className?.includes("primary") ? "default" : "ghost"}
      size="icon-sm"
      {...extraAttrs}
    >
      <span
        data-icon="inline-start"
        dangerouslySetInnerHTML={{ __html: iconHtml }}
      />
    </Button>
  )
}

export function mountComposerCards(options: MountComposerCardOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-composer-card-root]").forEach((element) => {
    const id = element.dataset.composerId
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
    const currentJson = JSON.stringify(props)
    if (currentJson === lastComposerCardPropsJson) {
      return
    }
    lastComposerCardPropsJson = currentJson
    mounted.root.render(<ComposerCard props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
      lastComposerCardPropsJson = ""
    }
  })
}

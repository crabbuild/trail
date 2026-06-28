import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import { Alert, AlertDescription } from "@/webview/components/ui/alert"
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
        <ComposerContextRail items={props.railItems} />
        <ComposerUtilityRow props={props} />
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
  if (status.tone === "ready" || status.tone === "context") {
    return (
      <div id="composer-status" className="sr-only" aria-live="polite">
        {status.label}. {status.detail}
      </div>
    )
  }
  return (
    <Alert
      id="composer-status"
      className={cn("composer-run-state", `composer-run-${status.tone}`)}
      role="status"
      aria-live="polite"
      variant={status.tone === "waiting" || status.tone === "warning" ? "destructive" : "default"}
    >
      <span className="composer-state-dot" aria-hidden="true" />
      <AlertDescription className="composer-state-copy">
        <strong>{status.label}</strong>
        <span>{status.detail}</span>
      </AlertDescription>
    </Alert>
  )
}

function ComposerContextRail({ items }: { items: ComposerRailItemView[] }) {
  return (
    <div className="composer-context-rail" aria-label="Prompt context">
      {items.map((item) => {
        const label = `${item.label}: ${item.value}`
        return (
          <Badge
            key={item.id}
            className={cn("composer-context-chip", `composer-context-chip-${item.tone}`)}
            title={label}
            aria-label={label}
            variant="outline"
          >
            <span>{item.label}</span>
            <strong>{item.value}</strong>
          </Badge>
        )
      })}
    </div>
  )
}

function ComposerUtilityRow({ props }: { props: ComposerCardProps }) {
  const clearDisabled = props.controlsDisabled || !props.draftValue
  return (
    <div className="composer-utility-row" aria-label="Prompt helpers">
      <ButtonGroup className="composer-preset-list" aria-label="Prompt starters">
        {props.presets.map((preset) => (
          <Button
            key={preset.id}
            type="button"
            className="composer-preset"
            data-action="insertPromptPreset"
            data-preset-id={preset.id}
            title={preset.detail}
            disabled={props.controlsDisabled}
            variant="outline"
            size="sm"
          >
            <span
              data-icon="inline-start"
              dangerouslySetInnerHTML={{ __html: preset.iconHtml }}
            />
            <span>{preset.label}</span>
          </Button>
        ))}
      </ButtonGroup>
      <ButtonGroup className="composer-mode-toggle" aria-label="Send mode">
        <ComposerModeButton
          mode="fast"
          label="Fast"
          title="Enter sends the prompt"
          active={props.sendMode === "fast"}
          disabled={props.controlsDisabled}
        />
        <ComposerModeButton
          mode="draft"
          label="Draft"
          title="Enter inserts a new line"
          active={props.sendMode === "draft"}
          disabled={props.controlsDisabled}
        />
      </ButtonGroup>
      <ComposerIconButton
        action="clearComposerDraft"
        label="Clear draft"
        iconHtml={props.clearIconHtml}
        disabled={clearDisabled}
        className="micro composer-clear"
        extraAttrs={{ "data-composer-clear": "" }}
      />
    </div>
  )
}

function ComposerModeButton({
  active,
  disabled,
  label,
  mode,
  title
}: {
  active: boolean
  disabled: boolean
  label: string
  mode: ComposerSendMode
  title: string
}) {
  return (
    <Button
      type="button"
      className={cn("composer-mode-button", active ? "active" : "")}
      data-action="setComposerSendMode"
      data-send-mode={mode}
      aria-pressed={active ? "true" : "false"}
      title={title}
      disabled={disabled}
      variant="ghost"
      size="xs"
    >
      {label}
    </Button>
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
    mounted.root.render(<ComposerCard props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
    }
  })
}

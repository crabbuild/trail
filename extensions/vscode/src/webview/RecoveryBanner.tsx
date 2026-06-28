import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import { Alert, AlertDescription, AlertTitle } from "@/webview/components/ui/alert"
import { Badge } from "@/webview/components/ui/badge"
import { Separator } from "@/webview/components/ui/separator"
import { cn } from "@/webview/lib/utils"
import { InlineActions, type InlineActionTone } from "./InlineActions"

export type RecoveryBannerKind = "failure" | "overlap"
export type RecoveryBannerActionTone = "default" | "primary" | "provider" | "lane" | "review"

export interface RecoveryBannerAction {
  action: string
  label: string
  tone: RecoveryBannerActionTone
}

export interface RecoveryBannerPath {
  id: string
  title: string
  labels: string
}

export interface RecoveryBannerProps {
  id: string
  kind: RecoveryBannerKind
  role: "alert" | "status"
  ariaLive: "assertive" | "polite"
  eyebrow: string
  title: string
  description: string
  detail?: string | undefined
  badges: string[]
  actions: RecoveryBannerAction[]
  paths: RecoveryBannerPath[]
}

export interface MountRecoveryBannersOptions {
  getProps(id: string): RecoveryBannerProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()

export function RecoveryBanner({ props }: { props: RecoveryBannerProps }) {
  const destructive = props.kind === "failure"

  return (
    <Alert
      data-recovery-banner=""
      className={cn(
        props.kind === "failure" ? "recovery-banner" : "overlap-banner",
        `recovery-banner-${props.kind}`
      )}
      variant={destructive ? "destructive" : "default"}
      role={props.role}
      aria-live={props.ariaLive}
    >
      <div className="recovery-banner-main">
        <div className="recovery-banner-badges">
          <Badge className="recovery-banner-role" variant={destructive ? "destructive" : "outline"}>
            {props.eyebrow}
          </Badge>
          {props.badges.map((badge) => (
            <Badge key={badge} className="tool-status" variant="outline">
              {badge}
            </Badge>
          ))}
        </div>
        <AlertTitle className="recovery-banner-title">
          <h2>{props.title}</h2>
        </AlertTitle>
        <AlertDescription className="recovery-banner-description">
          <p>{props.description}</p>
          {props.detail ? <p className="muted">{props.detail}</p> : null}
        </AlertDescription>
        <OverlapPaths paths={props.paths} />
      </div>
      <RecoveryActions actions={props.actions} />
    </Alert>
  )
}

function OverlapPaths({ paths }: { paths: RecoveryBannerPath[] }) {
  if (!paths.length) {
    return null
  }

  return (
    <>
      <Separator className="recovery-banner-separator" />
      <div className="overlap-paths">
        {paths.map((path) => (
          <Badge key={path.id} className="overlap-path" variant="outline">
            <b>{path.title}</b>
            {path.labels}
          </Badge>
        ))}
      </div>
    </>
  )
}

function RecoveryActions({ actions }: { actions: RecoveryBannerAction[] }) {
  return (
    <InlineActions
      props={{
        id: "recovery-actions",
        className: "recovery-actions",
        ariaLabel: "Recovery actions",
        actions: actions.map((action) => ({
          action: action.action,
          label: action.label,
          tone: recoveryActionTone(action.tone),
          data: { "recovery-action-tone": action.tone }
        }))
      }}
    />
  )
}

function recoveryActionTone(tone: RecoveryBannerActionTone): InlineActionTone {
  if (tone === "primary") {
    return "primary"
  }
  return tone
}

export function mountRecoveryBanners(options: MountRecoveryBannersOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-recovery-banner-root]").forEach((element) => {
    const id = element.dataset.recoveryBannerId
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
    mounted.root.render(<RecoveryBanner props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
    }
  })
}

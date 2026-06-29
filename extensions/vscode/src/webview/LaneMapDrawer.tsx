import * as React from "react"
import { XIcon } from "lucide-react"

import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import { Card, CardContent } from "@/webview/components/ui/card"
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle
} from "@/webview/components/ui/drawer"
import { cn } from "@/webview/lib/utils"
import type { ToolActivitySummary } from "./timelineModel"

export interface LaneMapChipView {
  id: string
  label: string
  iconHtml: string
  active?: boolean | undefined
}

export interface LaneMapTurnLinkView {
  id: string
  href: string
  label: string
  detail: string
}

export interface LaneMapDrawerProps {
  id: string
  visibleCount: number
  visibleGroups: number
  mapIconHtml: string
  activityIconHtml: string
  chips: LaneMapChipView[]
  activity: ToolActivitySummary
  turnLinks: LaneMapTurnLinkView[]
}

export function LaneMapDrawer({
  onOpenChange,
  open,
  props
}: {
  onOpenChange(open: boolean): void
  open: boolean
  props: LaneMapDrawerProps
}) {
  return (
    <Drawer direction="right" open={open} onOpenChange={onOpenChange}>
      {open ? (
        <DrawerContent className="lane-map-drawer" id="lane-map-drawer">
          <DrawerHeader className="drawer-header lane-map-drawer-header">
            <div className="lane-map-drawer-title">
              <span
                className="summary-icon"
                dangerouslySetInnerHTML={{ __html: props.mapIconHtml }}
              />
              <div>
                <DrawerTitle>Lane map</DrawerTitle>
                <DrawerDescription>
                  {props.visibleCount} visible item{props.visibleCount === 1 ? "" : "s"} across{" "}
                  {props.visibleGroups} group{props.visibleGroups === 1 ? "" : "s"}
                </DrawerDescription>
              </div>
            </div>
            <Button
              aria-label="Close lane map"
              onClick={() => onOpenChange(false)}
              size="icon-sm"
              type="button"
              variant="ghost"
            >
              <XIcon aria-hidden="true" data-icon="inline-start" />
            </Button>
          </DrawerHeader>
          <div className="lane-map-body">
            <LaneMapSummary props={props} />
            <ToolActivityPanel activity={props.activity} iconHtml={props.activityIconHtml} />
            {props.turnLinks.length ? (
              <section className="lane-map-section" aria-label="Recent turns">
                <div className="lane-map-section-heading">
                  <span>Recent turns</span>
                  <Badge variant="outline">{props.turnLinks.length}</Badge>
                </div>
                <div className="session-map-turns">
                  {props.turnLinks.map((link) => (
                    <a key={link.id} className="session-map-turn" href={link.href}>
                      <b>{link.label}</b>
                      <span>{link.detail}</span>
                    </a>
                  ))}
                </div>
              </section>
            ) : null}
          </div>
        </DrawerContent>
      ) : null}
    </Drawer>
  )
}

function LaneMapSummary({ props }: { props: LaneMapDrawerProps }) {
  return (
    <section className="lane-map-section" aria-label="Lane context">
      <div className="event-chips lane-map-chips">
        {props.chips.map((chip) => (
          <Badge
            key={chip.id}
            className={cn("event-chip", chip.active ? "active" : "")}
            variant="outline"
          >
            <span
              data-icon="inline-start"
              dangerouslySetInnerHTML={{ __html: chip.iconHtml }}
            />
            {chip.label}
          </Badge>
        ))}
      </div>
    </section>
  )
}

function ToolActivityPanel({
  activity,
  iconHtml
}: {
  activity: ToolActivitySummary
  iconHtml: string
}) {
  return (
    <Card
      size="sm"
      className={cn("tool-activity", `tool-activity-${activity.tone}`)}
      aria-label="Visible tool activity"
    >
      <CardContent className="tool-activity-content">
        <div className="tool-activity-heading">
          <span
            className="summary-icon"
            dangerouslySetInnerHTML={{ __html: iconHtml }}
          />
          <span className="tool-activity-title">
            <span>{activity.label}</span>
            <small>{activity.detail}</small>
          </span>
        </div>
        {activity.metrics.length ? (
          <div className="tool-activity-metrics">
            {activity.metrics.map((metric) => (
              <Card
                key={`${metric.label}:${metric.value}`}
                size="sm"
                className={cn("tool-activity-metric", `tool-activity-metric-${metric.tone}`)}
              >
                <b>{metric.value}</b>
                <span>{metric.label}</span>
              </Card>
            ))}
          </div>
        ) : (
          <p className="muted">Clear the transcript filter to inspect all tool activity for this lane.</p>
        )}
        {activity.paths.length ? (
          <div className="tool-activity-paths">
            {activity.paths.map((path) => (
              <Card
                key={path.path}
                size="sm"
                className={cn("tool-activity-path", `tool-activity-path-${path.tone}`)}
                title={path.detail}
              >
                <b>{shortLabel(path.path)}</b>
                <span>{path.detail}</span>
              </Card>
            ))}
          </div>
        ) : null}
      </CardContent>
    </Card>
  )
}

function shortLabel(value: string): string {
  const normalized = value.replace(/\\/g, "/")
  const parts = normalized.split("/").filter(Boolean)
  if (parts.length <= 2) {
    return normalized || value
  }
  return `${parts.at(-2)}/${parts.at(-1)}`
}

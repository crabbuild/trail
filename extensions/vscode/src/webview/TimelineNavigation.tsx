import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from "@/webview/components/ui/accordion"
import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import { ButtonGroup } from "@/webview/components/ui/button-group"
import { Card, CardContent } from "@/webview/components/ui/card"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger
} from "@/webview/components/ui/dropdown-menu"
import { cn } from "@/webview/lib/utils"
import { ChevronDownIcon, ListFilterIcon } from "lucide-react"
import type { TimelineFilter, ToolActivitySummary } from "./timelineModel"

export interface TimelineFilterView {
  id: TimelineFilter
  label: string
  count: number
  active: boolean
}

export interface TimelineMapChipView {
  id: string
  label: string
  iconHtml: string
  active?: boolean | undefined
}

export interface TimelineTurnLinkView {
  id: string
  href: string
  label: string
  detail: string
}

export interface TimelineNavigationProps {
  id: string
  filters: TimelineFilterView[]
  query: string
  queryDetail: string
  filtered: boolean
  visibleCount: number
  searchIconHtml: string
  mapIconHtml: string
  activityIconHtml: string
  visibleGroups: number
  chips: TimelineMapChipView[]
  activity: ToolActivitySummary
  turnLinks: TimelineTurnLinkView[]
}

export interface MountTimelineNavigationOptions {
  getProps(id: string): TimelineNavigationProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()

export function TimelineNavigation({ props }: { props: TimelineNavigationProps }) {
  return (
    <>
      <TimelineToolbar props={props} />
      <SessionMap props={props} />
    </>
  )
}

function TimelineToolbar({ props }: { props: TimelineNavigationProps }) {
  return (
    <div className="timeline-toolbar" aria-label="Transcript filters">
      <div className="timeline-filter-panel">
        <TimelineFilterMenu filters={props.filters} visibleCount={props.visibleCount} />
        <ButtonGroup className="timeline-filter-group" role="group" aria-label="Transcript type">
          {props.filters.map((filter) => (
            <Button
              key={filter.id}
              type="button"
              className={cn("timeline-filter", filter.active ? "active" : "")}
              data-action="setTimelineFilter"
              data-timeline-filter={filter.id}
              aria-pressed={filter.active}
              variant={filter.active ? "default" : "ghost"}
              size="sm"
            >
              <span>{filter.label}</span>
              <b>{filter.count}</b>
            </Button>
          ))}
        </ButtonGroup>
      </div>
      <label className="timeline-search">
        <span
          data-icon="inline-start"
          dangerouslySetInnerHTML={{ __html: props.searchIconHtml }}
        />
        <span className="sr-only">Search transcript</span>
        <input
          className="timeline-search-input"
          type="search"
          defaultValue={props.query}
          placeholder="Search transcript"
          aria-label="Search transcript"
        />
      </label>
      <div className="timeline-result-count">
        <span>
          {props.visibleCount} shown{props.queryDetail}
        </span>
        {props.filtered ? (
          <Button type="button" data-action="clearTimelineSearch" variant="outline" size="sm">
            Clear
          </Button>
        ) : null}
      </div>
    </div>
  )
}

function TimelineFilterMenu({
  filters,
  visibleCount
}: {
  filters: TimelineFilterView[]
  visibleCount: number
}) {
  const activeFilter = filters.find((filter) => filter.active) ?? filters[0]
  const activeLabel = activeFilter?.label ?? "All"
  const activeCount = activeFilter?.count ?? visibleCount

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            className="timeline-filter-menu-trigger"
            variant="outline"
            size="sm"
            aria-label={`Transcript filter: ${activeLabel}`}
          />
        }
      >
        <ListFilterIcon data-icon="inline-start" aria-hidden="true" />
        <span className="timeline-filter-menu-label">{activeLabel}</span>
        <Badge className="timeline-filter-menu-count" variant="secondary">
          {activeCount}
        </Badge>
        <ChevronDownIcon data-icon="inline-end" aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent className="timeline-filter-menu" align="start">
        <DropdownMenuGroup>
          <DropdownMenuLabel>Transcript filter</DropdownMenuLabel>
          <DropdownMenuRadioGroup value={activeFilter?.id ?? "all"}>
            {filters.map((filter) => (
              <DropdownMenuRadioItem
                key={filter.id}
                className="timeline-filter-menu-item"
                value={filter.id}
                data-action="setTimelineFilter"
                data-timeline-filter={filter.id}
              >
                <span>{filter.label}</span>
                <Badge className="timeline-filter-menu-item-count" variant={filter.active ? "default" : "outline"}>
                  {filter.count}
                </Badge>
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function SessionMap({ props }: { props: TimelineNavigationProps }) {
  return (
    <Accordion className="session-map">
      <AccordionItem className="session-map-item" value="session-map">
        <AccordionTrigger className="session-map-summary">
          <span
            className="summary-icon"
            dangerouslySetInnerHTML={{ __html: props.mapIconHtml }}
          />
          <span className="session-map-title">
            <span>Lane map</span>
            <small>
              {props.visibleCount} visible item{props.visibleCount === 1 ? "" : "s"}
            </small>
          </span>
          <Badge className="session-map-count" variant="outline">
            {props.visibleGroups} group{props.visibleGroups === 1 ? "" : "s"}
          </Badge>
        </AccordionTrigger>
        <AccordionContent className="session-map-panel" keepMounted>
          <Card className="session-map-body" size="sm">
            <CardContent className="session-map-content">
              <div className="event-chips">
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
              <ToolActivityPanel
                activity={props.activity}
                iconHtml={props.activityIconHtml}
              />
              {props.turnLinks.length ? (
                <div className="session-map-turns">
                  {props.turnLinks.map((link) => (
                    <a key={link.id} className="session-map-turn" href={link.href}>
                      <b>{link.label}</b>
                      <span>{link.detail}</span>
                    </a>
                  ))}
                </div>
              ) : null}
            </CardContent>
          </Card>
        </AccordionContent>
      </AccordionItem>
    </Accordion>
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

export function mountTimelineNavigation(options: MountTimelineNavigationOptions): void {
  const activeIds = new Set<string>()
  document.querySelectorAll<HTMLElement>("[data-timeline-navigation-root]").forEach((element) => {
    const id = element.dataset.timelineNavigationId
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
    mounted.root.render(<TimelineNavigation props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
    }
  })
}

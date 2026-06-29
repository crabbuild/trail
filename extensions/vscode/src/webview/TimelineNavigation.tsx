import * as React from "react"
import { createRoot, type Root } from "react-dom/client"

import { Badge } from "@/webview/components/ui/badge"
import { Button } from "@/webview/components/ui/button"
import { ButtonGroup } from "@/webview/components/ui/button-group"
import { Card, CardContent } from "@/webview/components/ui/card"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/webview/components/ui/collapsible"
import { cn } from "@/webview/lib/utils"
import { useFloatingDisclosure } from "./floatingDisclosure"
import { ChevronDownIcon, ListFilterIcon } from "lucide-react"
import type { LaneMapDrawerProps } from "./LaneMapDrawer"
import type { TimelineFilter } from "./timelineModel"

export interface TimelineFilterView {
  id: TimelineFilter
  label: string
  count: number
  active: boolean
}

export interface TimelineNavigationProps extends LaneMapDrawerProps {
  id: string
  filters: TimelineFilterView[]
  query: string
  queryDetail: string
  filtered: boolean
  searchIconHtml: string
}

export interface MountTimelineNavigationOptions {
  getProps(id: string): TimelineNavigationProps | undefined
}

interface MountedRoot {
  element: HTMLElement
  root: Root
}

const mountedRoots = new Map<string, MountedRoot>()
let lastTimelineNavigationPropsJson = ""

export function TimelineNavigation({ props }: { props: TimelineNavigationProps }) {
  return <TimelineToolbar props={props} />
}

function TimelineToolbar({ props }: { props: TimelineNavigationProps }) {
  const disclosure = useFloatingDisclosure()
  const activeFilter = props.filters.find((filter) => filter.active) ?? props.filters[0]
  const activeLabel = activeFilter?.label ?? "All"
  const activeCount = activeFilter?.count ?? props.visibleCount
  const queryLabel = props.query.trim()
  const triggerDetail = props.filtered
    ? [activeFilter?.id === "all" ? "" : activeLabel, queryLabel ? `"${queryLabel}"` : ""].filter(Boolean).join(" / ")
    : "All transcript"
  const triggerLabel = `Transcript filters: ${triggerDetail || activeLabel}, ${props.visibleCount} shown`

  return (
    <Collapsible
      className={cn("timeline-toolbar", props.filtered ? "filtered" : "")}
      data-floating-open={disclosure.open ? "true" : undefined}
      open={disclosure.open}
      onOpenChange={disclosure.setOpen}
      ref={disclosure.rootRef}
    >
      <CollapsibleTrigger
        className="timeline-filter-trigger"
        title={triggerLabel}
        aria-label={triggerLabel}
        ref={disclosure.triggerRef}
      >
        <ListFilterIcon data-icon="inline-start" aria-hidden="true" />
        <span className="timeline-filter-trigger-copy">
          <span className="timeline-filter-trigger-label">Filter</span>
          <span className="timeline-filter-trigger-detail">{props.visibleCount} shown</span>
        </span>
        {props.filtered ? (
          <Badge className="timeline-filter-trigger-count" variant="secondary">
            {activeCount}
          </Badge>
        ) : null}
        <ChevronDownIcon data-icon="inline-end" aria-hidden="true" />
      </CollapsibleTrigger>
      <CollapsibleContent keepMounted>
        <Card className="timeline-filter-popover" size="sm" aria-label="Transcript filters">
          <CardContent className="timeline-filter-popover-content">
            <div className="timeline-filter-popover-head">
              <div>
                <span>Transcript</span>
                <strong>
                  {props.visibleCount} shown{props.queryDetail}
                </strong>
              </div>
              {props.filtered ? (
                <Button type="button" data-action="clearTimelineSearch" variant="outline" size="sm">
                  Clear
                </Button>
              ) : null}
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
            <ButtonGroup className="timeline-filter-group" role="group" aria-label="Transcript type">
              {props.filters.map((filter) => (
                <Button
                  key={filter.id}
                  type="button"
                  className={cn("timeline-filter", filter.active ? "active" : "")}
                  data-action="setTimelineFilter"
                  data-timeline-filter={filter.id}
                  aria-pressed={filter.active}
                  variant={filter.active ? "default" : "outline"}
                  size="sm"
                >
                  <span>{filter.label}</span>
                  <b>{filter.count}</b>
                </Button>
              ))}
            </ButtonGroup>
          </CardContent>
        </Card>
      </CollapsibleContent>
    </Collapsible>
  )
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
    const currentJson = JSON.stringify(props)
    if (currentJson === lastTimelineNavigationPropsJson) {
      return
    }
    lastTimelineNavigationPropsJson = currentJson
    mounted.root.render(<TimelineNavigation props={props} />)
  })

  mountedRoots.forEach((mounted, id) => {
    if (!activeIds.has(id) || !mounted.element.isConnected) {
      mounted.root.unmount()
      mountedRoots.delete(id)
      lastTimelineNavigationPropsJson = ""
    }
  })
}

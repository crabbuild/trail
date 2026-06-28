import * as React from "react"

import { FLOATING_MENU_CLOSE_EVENT, type FloatingMenuCloseDetail } from "./floatingMenu"

export interface FloatingDisclosureState {
  open: boolean
  rootRef: React.RefCallback<HTMLDivElement>
  triggerRef: React.RefCallback<HTMLButtonElement>
  setOpen(open: boolean): void
}

export function useFloatingDisclosure(): FloatingDisclosureState {
  const [open, setOpen] = React.useState(false)
  const root = React.useRef<HTMLDivElement | null>(null)
  const trigger = React.useRef<HTMLButtonElement | null>(null)
  const openRef = React.useRef(open)

  React.useEffect(() => {
    openRef.current = open
  }, [open])

  React.useEffect(() => {
    const handleClose = (event: Event) => {
      const detail = (event as CustomEvent<FloatingMenuCloseDetail>).detail || {}
      const rootElement = root.current
      if (!rootElement) {
        return
      }
      if (detail.except && (rootElement === detail.except || rootElement.contains(detail.except))) {
        return
      }
      if (!openRef.current) {
        return
      }
      if (
        detail.restoreFocus &&
        document.activeElement instanceof HTMLElement &&
        rootElement.contains(document.activeElement)
      ) {
        trigger.current?.focus({ preventScroll: true })
      }
      setOpen(false)
    }

    document.addEventListener(FLOATING_MENU_CLOSE_EVENT, handleClose)
    return () => {
      document.removeEventListener(FLOATING_MENU_CLOSE_EVENT, handleClose)
    }
  }, [])

  return {
    open,
    rootRef: React.useCallback((element: HTMLDivElement | null) => {
      root.current = element
    }, []),
    triggerRef: React.useCallback((element: HTMLButtonElement | null) => {
      trigger.current = element
    }, []),
    setOpen
  }
}

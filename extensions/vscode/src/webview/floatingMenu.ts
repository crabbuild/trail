export const FLOATING_MENU_CLOSE_EVENT = "trail-floating-menu-close";

export interface FloatingMenuCloseDetail {
  except?: HTMLElement | undefined;
  restoreFocus?: boolean | undefined;
}

export function dispatchFloatingMenuClose(detail: FloatingMenuCloseDetail = {}): void {
  document.dispatchEvent(new CustomEvent<FloatingMenuCloseDetail>(FLOATING_MENU_CLOSE_EVENT, { detail }));
}

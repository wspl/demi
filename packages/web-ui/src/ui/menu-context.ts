import { readonly, ref } from 'vue'
import type { ComputedRef, InjectionKey, Ref } from 'vue'

export const menuIconlessKey: InjectionKey<ComputedRef<boolean>> = Symbol('menuIconless')

export interface MenuRoot {
  dismiss: () => void
}

export const menuRootKey: InjectionKey<MenuRoot> = Symbol('menuRoot')

/** Delay before a submenu closes after the pointer leaves its row or panel. */
export const SUBMENU_CLOSE_DELAY_MS = 120

/**
 * One submenu per menu. Hovering another submenu row replaces the open one at once and
 * without motion, the way native menus switch siblings; leaving for a plain row or out of
 * the menu closes after a short grace period so a diagonal path into the panel survives.
 */
export interface SubmenuController {
  /** Row whose submenu is open. */
  activeId: Readonly<Ref<symbol | null>>
  /** True while the last change was a sibling switch: panels skip enter/leave motion. */
  instant: Readonly<Ref<boolean>>
  open: (id: symbol) => void
  scheduleClose: (id: symbol) => void
  close: (id: symbol) => void
  dispose: () => void
}

export function createSubmenuController(): SubmenuController {
  const activeId = ref<symbol | null>(null)
  const instant = ref(false)
  let closeTimer = 0

  function open(id: symbol): void {
    window.clearTimeout(closeTimer)
    instant.value = activeId.value != null && activeId.value !== id
    activeId.value = id
  }

  function close(id: symbol): void {
    if (activeId.value !== id)
      return
    window.clearTimeout(closeTimer)
    instant.value = false
    activeId.value = null
  }

  function scheduleClose(id: symbol): void {
    if (activeId.value !== id)
      return
    window.clearTimeout(closeTimer)
    closeTimer = window.setTimeout(() => close(id), SUBMENU_CLOSE_DELAY_MS)
  }

  return {
    activeId: readonly(activeId),
    instant: readonly(instant),
    open,
    scheduleClose,
    close,
    dispose: () => window.clearTimeout(closeTimer),
  }
}

export const menuSubmenuKey: InjectionKey<SubmenuController> = Symbol('menuSubmenu')

/** Action rows dismiss the tree. Choices, flyout hosts, and suffix controls stay. */
export function shouldDismissMenuTree(item: {
  isChoice: boolean
  hasSubmenu: boolean
  hasSuffix: boolean
}): boolean {
  return !item.isChoice && !item.hasSubmenu && !item.hasSuffix
}

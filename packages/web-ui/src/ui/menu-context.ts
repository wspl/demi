import { readonly, ref } from 'vue'
import type { Component, ComputedRef, InjectionKey, Ref } from 'vue'
import type { MenuIndicator } from './MenuItem.vue'
import type { SentenceText } from './ui-text'

/** A row of a Menu's `items`: what the row shows, as MenuItem's props of the same names. */
export interface MenuListItem {
  id: string
  /** Often content, such as a device's name, so its style is the caller's. */
  label: string
  icon?: Component
  /** A muted word at the row's end: a state or a setting's value. */
  value?: string
  indicator?: MenuIndicator
  indicatorLabel?: SentenceText
  disabledReason?: SentenceText
}

export const menuIconlessKey: InjectionKey<ComputedRef<boolean>> = Symbol('menuIconless')

export interface MenuRoot {
  dismiss: () => void
}

export const menuRootKey: InjectionKey<MenuRoot> = Symbol('menuRoot')

/** A row of a Menu whose rows are its slot's rather than `items`: what the keyboard reads and acts on. */
export interface MenuSlotRow {
  label: () => string | undefined
  el: () => HTMLElement | null
  /** Opens the row's submenu with the keys in it; false when the row has none to open. */
  enter: () => boolean
}

/**
 * How a Menu's keyboard reaches the rows in its slot: each MenuItem
 * registers itself, shows the focused look while it is `focused`, and marks
 * the part of its label `highlight` gives (a type-select query's prefix).
 */
export interface MenuSlotKeyboard {
  register: (row: MenuSlotRow) => () => void
  focused: Readonly<Ref<MenuSlotRow | null>>
  highlight: (label: string) => readonly number[] | null
}

export const menuSlotKeyboardKey: InjectionKey<MenuSlotKeyboard> = Symbol('menuSlotKeyboard')

/**
 * What a row gives the menu in its submenu: whether the keyboard opened it,
 * so that menu takes the keys at its first row, and the way back, which
 * closes the submenu and returns the keys to the row (Left Arrow or Escape,
 * as in a macOS menu).
 */
export interface MenuSubmenuKeys {
  entered: Readonly<Ref<boolean>>
  leave: () => void
}

export const menuSubmenuKeysKey: InjectionKey<MenuSubmenuKeys> = Symbol('menuSubmenuKeys')

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

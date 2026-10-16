import { computed, readonly, ref } from 'vue'
import { usePhone } from './phone'
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

/**
 * How a Menu lays out the leading slot of its rows, which it alone decides
 * from what its rows are, as macOS's menus do:
 * - A menu whose rows have icons keeps the icon there, and a checked row
 *   shows its check at the trailing end.
 * - A menu whose rows have no icons has no icon column; when a row can be
 *   checked, the check takes the leading slot, and every other row keeps
 *   the slot's width empty so labels align.
 */
export interface MenuLayout {
  /** No row has an icon: the leading slot holds no icons. */
  iconless: ComputedRef<boolean>
  /** The leading slot holds the checks: the rows have no icons and some can be checked. */
  checkLeading: ComputedRef<boolean>
}

/** What a menu's layout needs of one of its rows. */
export interface MenuRowShape {
  /** The row shows an icon or a status dot in the leading slot. */
  hasIcon: boolean
  /** The row says whether it is checked, so the menu reserves a place for its check. */
  checkable: boolean
}

/** The leading slot of a menu whose rows are `rows`; `iconless` forces whether they have icons. */
export function menuLeadingSlot(rows: readonly MenuRowShape[], iconless?: boolean): { iconless: boolean; checkLeading: boolean } {
  const noIcons = iconless ?? !rows.some((row) => row.hasIcon)
  return { iconless: noIcons, checkLeading: noIcons && rows.some((row) => row.checkable) }
}

export const menuLayoutKey: InjectionKey<MenuLayout> = Symbol('menuLayout')

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
  /** The row shows an icon or a status dot in the leading slot. */
  hasIcon: () => boolean
  /** The row says whether it is checked, so the menu reserves a place for its check. */
  checkable: () => boolean
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

/**
 * A menu row's height in the layout the page is in now, live: 28 px, or the
 * menu's own `desktop` height, and on a phone a finger's 44 (`base.css`), in
 * a sheet or beside a field alike. Rows stand 1 px apart, so a list's pitch
 * is one more.
 */
export function useMenuRowHeight(desktop: () => number | undefined = () => undefined): ComputedRef<number> {
  const phone = usePhone()
  return computed(() => (phone.value ? 44 : desktop() ?? 28))
}

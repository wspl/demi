/**
 * Chrome-like tab strip motion: lock the current width, then collapse or grow
 * in flow.
 */

import type { InjectionKey } from 'vue'
import { clamp } from '@demicodes/utils'
import { prefersReducedMotion } from '../ui/stream-reveal'

/**
 * Provided by a window whose tabs narrow to their icons on a phone
 * (`SessionOverlay`: the Agents and Running windows): there only the selected
 * `TabItem` shows its name, as Chrome's narrowed tabs show the active one's
 * title. /
 */
export const iconTabsKey: InjectionKey<true> = Symbol('icon-tabs')

/**
 * The least width a strip's tab keeps, its mark, a few characters of its
 * title and its close slot: the strip scrolls before a tab narrows further.
 */
export const TAB_MIN_PX = 88

export const TAB_TRANSITION = 'tabs'
export const TAB_WIDTH_MS = 200

const leaveParents = new WeakMap<HTMLElement, HTMLElement>()
const enterWidths = new WeakMap<HTMLElement, number>()
/** The width a closing tab collapses from. */
const leaveWidths = new WeakMap<HTMLElement, number>()
/** The width a tab that stays moves to while others close. */
const closeWidths = new WeakMap<HTMLElement, number>()

function measureOpenWidth(tab: HTMLElement): number {
  const prevWidth = tab.style.width
  const prevFlex = tab.style.flex
  const prevMin = tab.style.minWidth
  // No inline width or flex: the width the strip's sharing gives the tab is
  // what it opens to.
  tab.style.width = ''
  tab.style.flex = ''
  tab.style.minWidth = ''
  void tab.offsetWidth
  const width = tab.getBoundingClientRect().width
  tab.style.width = prevWidth
  tab.style.flex = prevFlex
  tab.style.minWidth = prevMin
  return width
}

function tabEl(el: Element): HTMLElement {
  return el as HTMLElement
}

/** The gap between a strip's tabs. */
export function stripGap(strip: HTMLElement): number {
  const gap = Number.parseFloat(getComputedStyle(strip).columnGap)
  return Number.isFinite(gap) ? gap : 0
}

function columnGap(el: HTMLElement): number {
  const parent = el.parentElement
  if (!parent) {
    return 0
  }
  return stripGap(parent)
}

/**
 * True while the tab is on its way out and no longer part of the settled
 * layout.
 */
export function isLeavingTab(tab: Element): boolean {
  return tab.classList.contains(`${TAB_TRANSITION}-leave-active`)
}

/**
 * Where `tab` will sit once the strip's motion ends: tabs on their way out
 * take no room, a tab on its way in takes the width it opens to.
 */
export function settledTabBounds(strip: HTMLElement, tab: HTMLElement): { left: number; right: number } {
  const gap = stripGap(strip)
  let left = 0
  for (const child of strip.children) {
    if (child === tab) {
      break
    }
    if (child instanceof HTMLElement && !isLeavingTab(child)) {
      left += settledTabWidth(child) + gap
    }
  }
  return { left, right: left + settledTabWidth(tab) }
}

/**
 * Where the strip scrolls to so the selected `tab` shows whole and clear of
 * the edge fades, or null when it already does. A view too narrow for the
 * tab and a fade on each side shows the tab whole without that room, and the
 * fades give way beside it (`fadeRoom`): the selected tab is never under a
 * fade. The tab is never wider than the view (`--tab-room`).
 */
export function revealScroll(
  view: { scrollLeft: number; clientWidth: number; scrollWidth: number },
  tab: { left: number; right: number },
  fade: number,
): number | null {
  const margin = tab.right - tab.left + 2 * fade <= view.clientWidth ? fade : 0
  const latest = Math.max(0, tab.left - margin)
  const earliest = tab.right + margin - view.clientWidth
  const wanted = clamp(view.scrollLeft, earliest, latest)
  // The last tab's fade reaches past the strip's end: the end is as far as it
  // goes.
  const target = Math.min(wanted, Math.max(0, view.scrollWidth - view.clientWidth))
  return Math.abs(target - view.scrollLeft) < 1 ? null : target
}

/**
 * How wide each edge fade may be so it never lies over the selected tab:
 * the room between the view's edge and the tab, in the view's coordinates.
 * The fades give way to the selected tab while it shows whole, and while
 * the strip scrolls to reveal it (`revealing`). One the user scrolled under
 * an edge is cut there as any tab is, under that edge's fade; without a
 * selected tab in the strip the fades keep their own width.
 */
export function fadeRoom(
  clientWidth: number,
  selected: { left: number; right: number } | null,
  revealing: boolean,
): { before: number; after: number } {
  const whole = selected !== null && selected.left >= 0 && selected.right <= clientWidth
  if (!selected || (!whole && !revealing)) {
    return { before: Number.POSITIVE_INFINITY, after: Number.POSITIVE_INFINITY }
  }
  return { before: Math.max(0, selected.left), after: Math.max(0, clientWidth - selected.right) }
}

/** A mutation record of a strip, as `tabsChanged` reads it. */
export interface StripMutation<T> {
  type: string
  target: unknown
  addedNodes: Iterable<T>
  removedNodes: Iterable<T>
}

/**
 * Whether `records`, observed on `strip` and its tabs, hold a tab becoming
 * active, coming or going: what moves the strip's view. A tab's own content
 * changing, as a page's title does, is none of these. Nor is a node both
 * added and removed in the same batch: a render that takes a tab out and
 * puts it back, or the shallow copy of a tab that Vue's transition group puts
 * in the strip for a moment on each render to read its move class. Either
 * would pull the strip away from where the user scrolled it. `isTab` tells
 * an element from a text or comment node.
 */
export function tabsChanged<T>(
  strip: unknown,
  records: readonly StripMutation<T>[],
  isTab: (node: T) => boolean,
): boolean {
  if (records.some((record) => record.type === 'attributes')) {
    return true
  }
  const own = records.filter((record) => record.target === strip)
  const added = own.flatMap((record) => [...record.addedNodes]).filter(isTab)
  const removed = own.flatMap((record) => [...record.removedNodes]).filter(isTab)
  return added.some((node) => !removed.includes(node)) || removed.some((node) => !added.includes(node))
}

/**
 * How far an edge fade stays solid before it fades, so that it covers a tab
 * mark the edge cuts: at a scrolled edge a mark shows whole or not at all,
 * never sliced. `edge` and `marks` are in the strip's view coordinates;
 * `side` is the side of the view the edge is on.
 */
export function cutMarkCover(
  edge: number,
  side: 'start' | 'end',
  marks: readonly { left: number; right: number }[],
): number {
  const cut = marks.find((mark) => mark.left < edge && edge < mark.right)
  if (!cut) {
    return 0
  }
  return Math.ceil(side === 'start' ? cut.right - edge : edge - cut.left)
}

function clearWidthLock(tab: HTMLElement): void {
  tab.style.width = ''
  tab.style.flex = ''
  tab.style.transition = ''
}

/**
 * Whether a tab enters whole, in the frame it appears: the one the user is
 * shown, whose content shows at once, so the strip never lags it; any tab
 * when motion is reduced. Tabs added behind the shown one grow into place.
 */
function entersWhole(tab: HTMLElement): boolean {
  return tab.getAttribute('aria-selected') === 'true' || prefersReducedMotion()
}

export function beforeEnterTab(el: Element): void {
  const tab = tabEl(el)
  if (entersWhole(tab)) {
    // Over the transition classes the strip's group adds: no fade, no growth.
    tab.style.transition = 'none'
    tab.style.opacity = '1'
    return
  }
  tab.style.minWidth = '0'
  tab.style.overflow = 'hidden'
  tab.style.opacity = '0'
}

export function enterTab(el: Element): void {
  const tab = tabEl(el)
  if (entersWhole(tab)) {
    return
  }
  const width = measureOpenWidth(tab)
  enterWidths.set(tab, width)
  tab.style.flex = '0 0 auto'
  tab.style.transition = 'none'
  tab.style.width = '0'
  void tab.offsetWidth
  tab.style.transition = ''
  tab.style.width = `${width}px`
  tab.style.opacity = ''
}

/**
 * The width a tab will have once its entrance ends; its current width
 * otherwise.
 */
export function settledTabWidth(tab: HTMLElement): number {
  return enterWidths.get(tab) ?? closeWidths.get(tab) ?? tab.getBoundingClientRect().width
}

/**
 * How wide the content of `scroller`, which holds `strip`, will be once the
 * strip's motion ends: a closing tab takes no room and gives back its gap,
 * and every other tab takes its settled width.
 */
export function settledScrollWidth(scroller: HTMLElement, strip: HTMLElement): number {
  const gap = stripGap(strip)
  let change = 0
  for (const child of strip.children) {
    if (!isStyledEl(child)) {
      continue
    }
    const width = child.getBoundingClientRect().width
    if (isLeavingTab(child)) {
      const margin = Number.parseFloat(getComputedStyle(child).marginRight) || 0
      change -= width + margin + gap
    } else {
      change += settledTabWidth(child) - width
    }
  }
  return scroller.scrollWidth + change
}

/**
 * How far the strip's closing tabs have collapsed, from 0 to 1, the least of
 * them; null when none is closing. The tabs that stay move with them.
 */
export function closingProgress(strip: HTMLElement): number | null {
  let progress: number | null = null
  for (const child of strip.children) {
    const from = isStyledEl(child) ? leaveWidths.get(child) : undefined
    if (from === undefined || from <= 0) {
      continue
    }
    const done = clamp(1 - child.getBoundingClientRect().width / from, 0, 1)
    progress = progress === null ? done : Math.min(progress, done)
  }
  return progress
}

export function afterEnterTab(el: Element): void {
  const tab = tabEl(el)
  enterWidths.delete(tab)
  tab.style.width = ''
  tab.style.minWidth = ''
  tab.style.overflow = ''
  tab.style.flex = ''
  tab.style.opacity = ''
  tab.style.transition = ''
}

function isStyledEl(node: unknown): node is HTMLElement {
  return (
    typeof node === 'object' &&
    node !== null &&
    'style' in node &&
    'getBoundingClientRect' in node
  )
}

function stillLeaving(parent: HTMLElement): boolean {
  return [...parent.children].some(
    (child) => isStyledEl(child) && child.classList.contains('tabs-leave-active'),
  )
}

/**
 * Holds `tab` and each other tab that stays at its width, out of the strip's
 * sharing (`flex: 0 0 auto`), all read before any is written, so the strip
 * is laid out once and not once per tab. A tab on its way in keeps growing.
 */
function lockWidths(tab: HTMLElement): void {
  const siblings = [...tab.parentElement?.children ?? []].filter(
    (child): child is HTMLElement =>
      isStyledEl(child) && child !== tab && !child.classList.contains('tabs-leave-active') && !enterWidths.has(child),
  )
  const held = [tab, ...siblings]
  const widths = held.map((each) => each.getBoundingClientRect().width)
  held.forEach((each, index) => {
    each.style.width = `${widths[index]}px`
    each.style.flex = '0 0 auto'
  })
}

/**
 * After a close, hands the remaining tabs back to the strip's sharing. They
 * already moved with the close (`settleStaying`); only a change the close
 * brought about, such as the strip no longer overflowing, eases them on.
 */
function releaseStrip(parent: HTMLElement | null): void {
  if (!parent?.children) {
    return
  }
  if (stillLeaving(parent)) {
    return
  }

  const tabs = [...parent.children].filter(isStyledEl)
  if (tabs.length === 0) {
    return
  }

  const from = tabs.map((tab) => tab.getBoundingClientRect().width)

  for (const tab of tabs) {
    closeWidths.delete(tab)
    tab.style.transition = 'none'
    tab.style.width = ''
    tab.style.flex = ''
  }
  void parent.offsetWidth
  const to = tabs.map((tab) => tab.getBoundingClientRect().width)

  if (
    prefersReducedMotion() ||
    tabs.every((_, index) => Math.abs(from[index]! - to[index]!) < 0.5)
  ) {
    for (const tab of tabs) {
      clearWidthLock(tab)
    }
    return
  }

  for (let index = 0; index < tabs.length; index++) {
    const tab = tabs[index]!
    tab.style.flex = '0 0 auto'
    tab.style.width = `${from[index]}px`
  }
  void parent.offsetWidth
  for (let index = 0; index < tabs.length; index++) {
    const tab = tabs[index]!
    tab.style.transition = `width ${TAB_WIDTH_MS}ms ease-out`
    tab.style.width = `${to[index]}px`
    const finish = (event?: TransitionEvent): void => {
      if (event && event.propertyName !== 'width') {
        return
      }
      tab.removeEventListener('transitionend', finish)
      clearWidthLock(tab)
    }
    tab.addEventListener('transitionend', finish)
  }
}

export function beforeLeaveTab(el: Element): void {
  const tab = tabEl(el)
  if (tab.parentElement) {
    leaveParents.set(tab, tab.parentElement)
  }
  // A closing tab is no tab any more: the selection has passed on, so it fades
  // out as an unselected one and the strip shows one selected tab throughout.
  tab.setAttribute('aria-selected', 'false')
  tab.style.setProperty('--tab-active', 'var(--tab-row)')
  // A tab closed with others, as Close Others closes them one by one, may
  // hold the width motion an earlier one gave it: its own close fades it.
  closeWidths.delete(tab)
  tab.style.transition = ''
  lockWidths(tab)
  leaveWidths.set(tab, Number.parseFloat(tab.style.width) || 0)
  tab.style.minWidth = '0'
  tab.style.overflow = 'hidden'
}

export function leaveTab(el: Element): void {
  const tab = tabEl(el)
  tab.style.width = '0'
  tab.style.marginRight = `-${columnGap(tab)}px`
  tab.style.opacity = '0'
  if (tab.parentElement) {
    settleStaying(tab.parentElement, [tab])
  }
}

/**
 * `transition` for a tab, with the width motion a close gave it while that
 * runs: a transition that leaves width out would end it at once.
 */
export function keepingCloseMotion(tab: HTMLElement, transition: string): string {
  if (!closeWidths.has(tab)) {
    return transition
  }
  const width = `width ${TAB_WIDTH_MS}ms ease-out`
  return transition && transition !== 'none' ? `${transition}, ${width}` : width
}

/**
 * Aims the tabs that stay at their widths again while tabs close, once the
 * strip's room changed under them, as when a close ends its overflow and the
 * overflow button goes: the motion is retargeted, not followed by another.
 */
export function resettleClosing(strip: HTMLElement): void {
  if ([...strip.children].some(isLeavingTab)) {
    settleStaying(strip, [])
  }
}

export function afterLeaveTab(el: Element): void {
  const tab = tabEl(el)
  const parent = tab.parentElement ?? leaveParents.get(tab) ?? null
  leaveParents.delete(tab)
  leaveWidths.delete(tab)
  releaseStrip(parent)
}

/**
 * Starts the tabs that stay toward the widths they take once the closing
 * ones are gone, in the closing tab's motion, so the strip moves once rather
 * than closing first and widening after. A tab on its way in keeps its own
 * motion.
 */
function settleStaying(parent: HTMLElement, closing: HTMLElement[]): void {
  if (prefersReducedMotion()) {
    return
  }
  const children = [...parent.children].filter(isStyledEl)
  const leaving = children.filter((child) => closing.includes(child) || isLeavingTab(child))
  const entering = children.filter((child) => !leaving.includes(child) && enterWidths.has(child))
  const staying = children.filter((child) => !leaving.includes(child) && !entering.includes(child))
  const from = staying.map((child) => child.getBoundingClientRect().width)
  // The measure lays the strip out shorter, and the browser would pull a
  // scroll at its end back to fit that at once: the scroll is put back.
  const scrolled: [HTMLElement, number][] = []
  for (let each: HTMLElement | null = parent; each; each = each.parentElement) {
    if (each.scrollLeft > 0) {
      scrolled.push([each, each.scrollLeft])
    }
  }
  // Out of the flow, which keeps their own motion running, the closing tabs
  // leave the room the others share once they are gone.
  for (const child of leaving) {
    child.style.position = 'absolute'
  }
  // A tab on its way in takes the room it opens to; its least width is not
  // one its motion runs on.
  for (const child of entering) {
    child.style.minWidth = `${enterWidths.get(child)}px`
  }
  for (const child of staying) {
    child.style.transition = 'none'
    child.style.width = ''
    child.style.flex = ''
  }
  const to = staying.map((child) => child.getBoundingClientRect().width)
  for (const child of leaving) {
    child.style.position = ''
  }
  for (const child of entering) {
    child.style.minWidth = '0'
  }
  staying.forEach((child, index) => {
    child.style.width = `${from[index]}px`
    child.style.flex = '0 0 auto'
  })
  void parent.offsetWidth
  for (const [each, left] of scrolled) {
    each.scrollLeft = left
  }
  staying.forEach((child, index) => {
    closeWidths.set(child, to[index]!)
    child.style.transition = `width ${TAB_WIDTH_MS}ms ease-out`
    child.style.width = `${to[index]}px`
  })
}



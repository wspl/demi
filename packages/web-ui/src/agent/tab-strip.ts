/** Chrome-like tab strip motion: lock the current width, then collapse or grow in flow. */

export const TAB_TRANSITION = 'tabs'
export const TAB_WIDTH_MS = 200

const leaveParents = new WeakMap<HTMLElement, HTMLElement>()
const enterWidths = new WeakMap<HTMLElement, number>()

function measureOpenWidth(tab: HTMLElement): number {
  const prevWidth = tab.style.width
  const prevShrink = tab.style.flexShrink
  // No inline width: the tab's own class width is what it opens to.
  tab.style.width = ''
  tab.style.flexShrink = ''
  void tab.offsetWidth
  const width = tab.getBoundingClientRect().width
  tab.style.width = prevWidth
  tab.style.flexShrink = prevShrink
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

/** True while the tab is on its way out and no longer part of the settled layout. */
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

function prefersReducedMotion(): boolean {
  return (
    typeof matchMedia === 'function' &&
    matchMedia('(prefers-reduced-motion: reduce)').matches
  )
}

function clearWidthLock(tab: HTMLElement): void {
  tab.style.width = ''
  tab.style.flexShrink = ''
  tab.style.transition = ''
}

export function beforeEnterTab(el: Element): void {
  const tab = tabEl(el)
  tab.style.minWidth = '0'
  tab.style.overflow = 'hidden'
  tab.style.opacity = '0'
}

export function enterTab(el: Element): void {
  const tab = tabEl(el)
  const width = measureOpenWidth(tab)
  enterWidths.set(tab, width)
  tab.style.flexShrink = '0'
  tab.style.transition = 'none'
  tab.style.width = '0'
  void tab.offsetWidth
  tab.style.transition = ''
  tab.style.width = `${width}px`
  tab.style.opacity = ''
}

/** The width a tab will have once its entrance ends; its current width otherwise. */
export function settledTabWidth(tab: HTMLElement): number {
  return enterWidths.get(tab) ?? tab.getBoundingClientRect().width
}

export function afterEnterTab(el: Element): void {
  const tab = tabEl(el)
  enterWidths.delete(tab)
  tab.style.width = ''
  tab.style.minWidth = ''
  tab.style.overflow = ''
  tab.style.flexShrink = ''
  tab.style.opacity = ''
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

function lockSiblingWidths(tab: HTMLElement): void {
  const parent = tab.parentElement
  if (!parent?.children) {
    return
  }
  for (const child of parent.children) {
    if (!isStyledEl(child) || child === tab) {
      continue
    }
    if (child.classList.contains('tabs-leave-active')) {
      continue
    }
    child.style.width = `${child.getBoundingClientRect().width}px`
    child.style.flexShrink = '0'
  }
}

/** After a close, ease remaining tabs from the squeezed width to the new layout. */
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
    tab.style.transition = 'none'
    tab.style.width = ''
    tab.style.flexShrink = ''
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
    tab.style.flexShrink = '0'
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
  lockSiblingWidths(tab)
  tab.style.width = `${tab.getBoundingClientRect().width}px`
  tab.style.minWidth = '0'
  tab.style.overflow = 'hidden'
  tab.style.flexShrink = '0'
}

export function leaveTab(el: Element): void {
  const tab = tabEl(el)
  tab.style.width = '0'
  tab.style.marginRight = `-${columnGap(tab)}px`
  tab.style.opacity = '0'
}

export function afterLeaveTab(el: Element): void {
  const tab = tabEl(el)
  const parent = tab.parentElement ?? leaveParents.get(tab) ?? null
  leaveParents.delete(tab)
  releaseStrip(parent)
}

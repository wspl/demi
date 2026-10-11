import type { Alignment } from '@floating-ui/vue'

/**
 * The attribute a layout primitive puts on its region's element: the pane
 * whose box holds a trigger, such as the sidebar, the conversation, the work
 * panel, a dialog or a settings page. The nearest one around a trigger is its
 * region; without one, the viewport is.
 */
const REGION_SELECTOR = '[data-region]'

/** A box's horizontal extent, in viewport pixels. */
export interface Span {
  left: number
  width: number
}

/**
 * Which of the trigger's edges a menu opened from it lines up with, so that
 * the menu grows toward the inside of the trigger's region: its end edge for
 * a trigger whose centre lies in the end half of the region, else its start
 * edge. The product lays out left to right, so the end half is the right one.
 */
export function inwardAlignment(trigger: Span, region: Span): Alignment {
  const centre = trigger.left + trigger.width / 2
  return centre > region.left + region.width / 2 ? 'end' : 'start'
}

/** A box in viewport pixels. */
export interface Box {
  x: number
  y: number
  width: number
  height: number
}

/** The box of the region around `el`: its nearest marked region, else the viewport. */
export function regionBox(el: Element | null): Box {
  const region = el?.closest(REGION_SELECTOR)
  if (!region) {
    const root = document.documentElement
    return { x: 0, y: 0, width: root.clientWidth, height: root.clientHeight }
  }
  const rect = region.getBoundingClientRect()
  return { x: rect.left, y: rect.top, width: rect.width, height: rect.height }
}

/** The extent of the region around `el`: its nearest marked region, else the viewport. */
export function regionSpan(el: Element): Span {
  const box = regionBox(el)
  return { left: box.x, width: box.width }
}

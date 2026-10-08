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

/** The extent of the region around `el`: its nearest marked region, else the viewport. */
export function regionSpan(el: Element): Span {
  const region = el.closest(REGION_SELECTOR)
  if (!region)
    return { left: 0, width: document.documentElement.clientWidth }
  const rect = region.getBoundingClientRect()
  return { left: rect.left, width: rect.width }
}

import { clamp } from '@demicodes/utils'
import { onBeforeUnmount, ref, watch } from 'vue'
import type { PixelSize } from '../files/thumbnail'
import { FIELD } from '../overlay/dialogFocus'

/** A window's box on the page, in CSS px from the page's top-left corner. */
export interface WindowRect {
  x: number
  y: number
  width: number
  height: number
}

/** The page a window floats over: the browser window's viewport. */
export interface PageSize {
  width: number
  height: number
}

/** The smallest a window gets, unless the page itself is smaller. */
export const WINDOW_MIN: PageSize = { width: 240, height: 180 }

/** The title bar's height, which the window keeps on the page. */
export const WINDOW_TITLE_BAR_PX = 36

/** Which edges a resize moves: one for an edge, two for a corner. */
export interface WindowEdges {
  left?: boolean
  right?: boolean
  top?: boolean
  bottom?: boolean
}

function minSize(page: PageSize): PageSize {
  return { width: Math.min(WINDOW_MIN.width, page.width), height: Math.min(WINDOW_MIN.height, page.height) }
}

/**
 * The box a window takes on `page`: at least 240 × 180 and no larger than
 * the page, with its title bar wholly on the page; the rest of it may hang
 * below the page's bottom edge. A page that shrinks moves and shrinks the
 * window with it.
 */
export function clampWindowRect(rect: WindowRect, page: PageSize): WindowRect {
  const min = minSize(page)
  const width = Math.round(clamp(rect.width, min.width, page.width))
  const height = Math.round(clamp(rect.height, min.height, page.height))
  return {
    x: Math.round(clamp(rect.x, 0, page.width - width)),
    y: Math.round(clamp(rect.y, 0, Math.max(0, page.height - WINDOW_TITLE_BAR_PX))),
    width,
    height,
  }
}

/** The box a title bar's drag reaches: `start` moved by the pointer's offset, kept on the page. */
export function moveWindowRect(start: WindowRect, dx: number, dy: number, page: PageSize): WindowRect {
  return clampWindowRect({ ...start, x: start.x + dx, y: start.y + dy }, page)
}

/**
 * The box an edge's or a corner's drag reaches: the edges it holds follow
 * the pointer and the opposite ones stay where they were, as a desktop
 * window's do. An edge stops at the page's border and where the window
 * would get smaller than its minimum.
 */
export function resizeWindowRect(
  start: WindowRect,
  edges: WindowEdges,
  dx: number,
  dy: number,
  page: PageSize,
): WindowRect {
  const min = minSize(page)
  let left = start.x
  let right = start.x + start.width
  let top = start.y
  let bottom = start.y + start.height
  if (edges.left)
    left = clamp(left + dx, 0, right - min.width)
  if (edges.right)
    right = clamp(right + dx, left + min.width, page.width)
  if (edges.top)
    top = clamp(top + dy, 0, bottom - min.height)
  if (edges.bottom)
    bottom = clamp(bottom + dy, top + min.height, page.height)
  return clampWindowRect({ x: left, y: top, width: right - left, height: bottom - top }, page)
}

/**
 * The box a window first opens at to show content of `content` pixels
 * under its title bar: the content at its actual size, scaled down to fit
 * at most 60% of the page's width and 70% of its height, centred over
 * `over` (the pane it was opened from). Content of unknown size takes that
 * whole allowance.
 */
export function initialWindowRect(content: PixelSize | null, page: PageSize, over: WindowRect): WindowRect {
  const room = { width: page.width * 0.6, height: page.height * 0.7 - WINDOW_TITLE_BAR_PX }
  const known = content && content.width > 0 && content.height > 0 ? content : null
  const scale = known ? Math.min(1, room.width / known.width, room.height / known.height) : 1
  const width = known ? known.width * scale : room.width
  const height = (known ? known.height * scale : room.height) + WINDOW_TITLE_BAR_PX
  const min = minSize(page)
  const shown = { width: Math.max(width, min.width), height: Math.max(height, min.height) }
  return clampWindowRect({
    x: over.x + (over.width - shown.width) / 2,
    y: over.y + (over.height - shown.height) / 2,
    ...shown,
  }, page)
}

/** What a floating window needs to know of an Escape. */
export interface EscapeKey {
  defaultPrevented: boolean
  isComposing: boolean
  /** The element the key went to, the one with the focus; none for the page itself. */
  target: { closest(selectors: string): unknown } | null
}

/**
 * Whether an Escape closes a floating window. It does not when something
 * used it first (it called preventDefault, as a field reverting its edit
 * does), when an input method composes, when the focus is in a field the
 * user types in, or while another layer that takes Escape is open, a menu or
 * a dialog (`otherLayerOpen`), whichever of them the key reached first.
 */
export function escapeClosesWindow(key: EscapeKey, otherLayerOpen: boolean): boolean {
  if (key.defaultPrevented || key.isComposing || otherLayerOpen)
    return false
  return key.target?.closest(FIELD) == null
}

/** How many floating windows are open on the page. */
const openWindows = ref(0)

/**
 * Counts a floating window while `isOpen` holds, until it unmounts, so a
 * layer beneath the windows knows one stands in front of it.
 */
export function useOpenWindowCount(isOpen: () => boolean): void {
  let counted = false
  function count(open: boolean): void {
    if (open === counted)
      return
    counted = open
    openWindows.value += open ? 1 : -1
  }
  watch(isOpen, count, { immediate: true })
  onBeforeUnmount(() => count(false))
}

/**
 * Whether a floating window stands in front of the page, as the media
 * viewer does: a window beneath it, such as the dock's Agents window,
 * leaves Escape to it, so Escape closes the one in front first.
 */
export function floatingWindowInFront(): boolean {
  return openWindows.value > 0
}

/**
 * Where a watched tab's picture sits in the panel, and what the viewer's
 * pointer means in the tab (`live-view.md` § Modes), by the generation of
 * the picture shown. A Web picture stands unscaled at the panel's top-left
 * corner; Mobile and Custom keep their size, scaled to fit and centred.
 */
import type { TitleText } from '@demicodes/plugin-sdk'
import type { BrowserViewport, CursorRegion } from '../generated/plugin'
import type { LiveStream } from './session'

export interface PanelSize {
  width: number
  height: number
}

export interface Placement {
  /** Panel CSS pixels per tab CSS pixel. */
  scale: number
  left: number
  top: number
  width: number
  height: number
}

/** The largest size the protocol and the Host's capture take. */
const MAX_SIDE = 4096

/** The panel the module hears about: whole CSS pixels within the protocol's range. */
export function panelSize(width: number, height: number): PanelSize {
  const side = (length: number) => Math.max(1, Math.min(MAX_SIDE, Math.round(length)))
  return { width: side(width), height: side(height) }
}

/**
 * The tab CSS pixels a generation's pictures cover: its viewport, less the
 * last row or column of an odd side the encoder cut, at any scale.
 */
export function pictureExtent(stream: LiveStream): PanelSize {
  const pixels = stream.viewport.devicePixelRatio * stream.scale
  return { width: stream.width / pixels, height: stream.height / pixels }
}

/**
 * Where a picture of `stream` sits in the panel. A Web picture stands at its
 * own CSS size from the top-left corner, never scaled: the panel cuts it
 * while it shrinks, and shows white beside it while it grows, until a
 * picture at the new size arrives. Mobile and Custom keep their size, scaled
 * down to fit and centred.
 */
export function placePicture(stream: LiveStream, panel: PanelSize): Placement {
  const extent = pictureExtent(stream)
  if (stream.viewport.mode === 'web') {
    return { scale: 1, left: 0, top: 0, width: extent.width, height: extent.height }
  }
  const scale = Math.min(1, panel.width / stream.viewport.width, panel.height / stream.viewport.height)
  const width = extent.width * scale
  const height = extent.height * scale
  return {
    scale,
    left: Math.max(0, (panel.width - stream.viewport.width * scale) / 2),
    top: Math.max(0, (panel.height - stream.viewport.height * scale) / 2),
    width,
    height,
  }
}

/**
 * How far to move a picture, in CSS pixels, so that it starts on a whole
 * device pixel. A panel whose edge falls between two device pixels, as a
 * percentage split leaves it, would have its every pixel blended from two of
 * the picture's: sharp text turns soft although each frame is exact.
 */
export function deviceSnap(origin: number, devicePixelRatio: number): number {
  const device = origin * devicePixelRatio
  return (Math.round(device) - device) / devicePixelRatio
}

/** The tab's CSS coordinates under a point of the panel. */
export function tabPoint(
  point: { x: number; y: number },
  viewport: BrowserViewport,
  placement: Placement,
): { x: number; y: number } {
  const inside = (value: number, length: number) => Math.max(0, Math.min(length, value))
  return {
    x: inside((point.x - placement.left) / placement.scale, viewport.width),
    y: inside((point.y - placement.top) / placement.scale, viewport.height),
  }
}

/** CSS cursor keywords a page can name and this web browser shows; anything else, such as an image, is not one. */
const CURSOR_KEYWORD = /^[a-z][a-z-]*$/

/**
 * The cursor the page shows at `point` of the tab (`live-view.md` § Input):
 * the last region over the point decides; where none does, or one leaves
 * the cursor to the browser, the observer's resolution at the pointer
 * holds.
 */
export function cursorAt(point: { x: number; y: number }, regions: readonly CursorRegion[], resolved: string): string {
  const region = regions.findLast((candidate) =>
    point.x >= candidate.x
    && point.x < candidate.x + candidate.width
    && point.y >= candidate.y
    && point.y < candidate.y + candidate.height)
  const cursor = region && region.cursor !== 'auto' ? region.cursor : resolved
  return CURSOR_KEYWORD.test(cursor) && cursor !== 'auto' ? cursor : 'default'
}

/** Where a rectangle of the tab lands in the panel, for a control over it. */
export function panelRect(
  rect: { x: number; y: number; width: number; height: number },
  placement: Placement,
): { left: number; top: number; width: number; height: number } {
  return {
    left: placement.left + rect.x * placement.scale,
    top: placement.top + rect.y * placement.scale,
    width: rect.width * placement.scale,
    height: rect.height * placement.scale,
  }
}

/** What the viewport menu offers and shows for a tab. */
export interface ViewportChoice {
  mode: BrowserViewport['mode']
  label: TitleText
  /** Only the agent gives a tab its Custom size; the menu cannot choose one. */
  selectable: boolean
}

export function viewportChoices(viewport: BrowserViewport): ViewportChoice[] {
  const choices: ViewportChoice[] = [
    { mode: 'web', label: 'Web', selectable: true },
    { mode: 'mobile', label: 'Mobile', selectable: true },
  ]
  if (viewport.mode === 'custom') {
    choices.push({
      mode: 'custom',
      label: `Custom ${viewport.width} × ${viewport.height} @${viewport.devicePixelRatio}`,
      selectable: false,
    })
  }
  return choices
}

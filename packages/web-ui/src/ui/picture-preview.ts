import { computed, type ComputedRef } from 'vue'
import { createSharedComposable, useDevicePixelRatio, useWindowSize } from '@vueuse/core'
import { fitScale } from '../files/image-zoom'
import type { PixelSize } from '../files/thumbnail'

/** The most of the page's width and of its height a hover card's picture takes. */
const PAGE_SHARE = 0.7

/** What a hover card's picture is sized against: the page's size in CSS px and the screen's pixel ratio. */
export interface PreviewPage {
  size: PixelSize
  pixelRatio: number
}

/**
 * The CSS size of a picture in its hover card: its own size on the screen, its
 * pixels one to one as a browser shows an image at 100%, scaled down to fit 70%
 * of the page's width and of its height, never enlarged.
 */
export function previewBox(size: PixelSize, pixelRatio: number, page: PixelSize): PixelSize {
  const own = { width: size.width / pixelRatio, height: size.height / pixelRatio }
  const scale = fitScale(own, { width: page.width * PAGE_SHARE, height: page.height * PAGE_SHARE })
  return {
    width: Math.max(1, Math.round(own.width * scale)),
    height: Math.max(1, Math.round(own.height * scale)),
  }
}

/** The space a hover card keeps to its picture, and to the page's edge. */
export const PREVIEW_GAP = 8

/** Which side of its picture a hover card goes, and the size its picture shows at there. */
export interface PreviewPlacement {
  side: 'top' | 'bottom'
  picture: PixelSize
}

/**
 * Where a hover card goes: above its picture when the whole card fits there,
 * below it when it fits there instead, and otherwise on the side with more
 * room, its picture scaled down so the whole card fits that side, so it never
 * covers the picture it shows. `picture` is the picture's size in the card
 * (`previewBox`), `chrome` the height the card adds to it (its padding and
 * caption), and `room` the space between the picture and the page's top and
 * bottom edges; the card keeps `PREVIEW_GAP` to the picture and to the edge.
 */
export function previewPlacement(
  picture: PixelSize,
  chrome: number,
  room: { above: number; below: number },
): PreviewPlacement {
  const above = room.above - 2 * PREVIEW_GAP
  const below = room.below - 2 * PREVIEW_GAP
  const card = picture.height + chrome
  if (card <= above)
    return { side: 'top', picture }
  if (card <= below)
    return { side: 'bottom', picture }
  const side = above >= below ? 'top' : 'bottom'
  const height = Math.max(1, Math.floor((side === 'top' ? above : below) - chrome))
  const scale = fitScale(picture, { width: picture.width, height })
  return { side, picture: { width: Math.max(1, Math.floor(picture.width * scale)), height } }
}

/** The most a picture of unknown size takes before it loads: 70% of the page's width and of its height. */
export function previewLimit(page: PixelSize): PixelSize {
  return { width: Math.floor(page.width * PAGE_SHARE), height: Math.floor(page.height * PAGE_SHARE) }
}

/**
 * The page the hover cards are sized against, live: it follows the window's
 * resizing and the pixel ratio's change as the window moves to another
 * screen. One pair of listeners for the whole page.
 */
export const usePreviewPage: () => ComputedRef<PreviewPage> = createSharedComposable(() => {
  const { width, height } = useWindowSize()
  const { pixelRatio } = useDevicePixelRatio()
  return computed(() => ({ size: { width: width.value, height: height.value }, pixelRatio: pixelRatio.value }))
})

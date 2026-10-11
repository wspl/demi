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
 * The CSS size of a picture in its hover card (`file-previews.md` § A picture's
 * preview on hover, The card): its own size on the screen, its pixels one to
 * one as a browser shows an image at 100%, scaled down to fit 70% of the
 * page's width and of its height, never enlarged.
 */
export function previewBox(size: PixelSize, pixelRatio: number, page: PixelSize): PixelSize {
  const own = { width: size.width / pixelRatio, height: size.height / pixelRatio }
  const scale = fitScale(own, { width: page.width * PAGE_SHARE, height: page.height * PAGE_SHARE })
  return {
    width: Math.max(1, Math.round(own.width * scale)),
    height: Math.max(1, Math.round(own.height * scale)),
  }
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

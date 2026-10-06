import { clamp } from '@demicodes/utils'

/** A picture's size in pixels. */
export interface PixelSize {
  width: number
  height: number
}

/**
 * How tall a thumbnail is: a tool's image and an image in a run of a
 * message's images (`file-previews.md` § Media a tool returned). The row it
 * stands in keeps this height before the image loads and after.
 */
export const THUMBNAIL_HEIGHT = 80

/** The narrowest and the widest a thumbnail is; beyond them the image is cropped. */
const THUMBNAIL_MIN_WIDTH = 64
const THUMBNAIL_MAX_WIDTH = 200

/**
 * The box a thumbnail of an image of `natural` size takes (`file-previews.md`
 * § Media a tool returned): 80 pixels tall, as wide as its proportions make
 * it within 64 and 200 pixels. An image whose proportions fall outside that
 * range fills the box and is cropped (`object-fit: cover`), a tall one to its
 * top and a wide one to its middle. While its size is unknown, before its
 * bytes arrive, the box is the narrowest one.
 *
 * Never enlarged: the box is never taller or wider than the image itself, so
 * an image smaller than the box keeps its own size where it is narrower or
 * shorter. A 50 × 50 icon is 50 × 50, not widened to 64 or raised to 80; a
 * 300 × 40 strip is 200 × 40, cropped to its middle at its own scale.
 */
export function thumbnailBox(natural: PixelSize | null): PixelSize {
  if (natural === null)
    return { width: THUMBNAIL_MIN_WIDTH, height: THUMBNAIL_HEIGHT }
  const scale = Math.min(1, THUMBNAIL_HEIGHT / natural.height)
  const width = clamp(natural.width * scale, THUMBNAIL_MIN_WIDTH, THUMBNAIL_MAX_WIDTH)
  return {
    width: Math.round(Math.min(width, natural.width)),
    height: Math.round(Math.min(THUMBNAIL_HEIGHT, natural.height)),
  }
}

/** The size an image element's bytes have, once it has loaded them; null before. */
export function loadedSize(image: HTMLImageElement): PixelSize | null {
  if (!image.complete || image.naturalWidth === 0)
    return null
  return { width: image.naturalWidth, height: image.naturalHeight }
}

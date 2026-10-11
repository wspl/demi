import { clamp } from '@demicodes/utils'

/** A picture's size in pixels. */
export interface PixelSize {
  width: number
  height: number
}

/** What a thumbnail shows: an image, or a video's first frame. */
export type ThumbnailKind = 'image' | 'video'

/**
 * How tall a thumbnail is at most: one in a run of a message's images and
 * videos (`file-previews.md` § Files named in messages). A medium whose
 * size is unknown until it loads keeps a place this tall before it loads
 * and after (`thumbnailPlaceHeight`).
 */
export const THUMBNAIL_HEIGHT = 80

/** The bounds of a thumbnail's box: its height at most, and its narrowest and widest, beyond which the medium is cropped. */
export interface ThumbnailBounds {
  height: number
  minWidth: number
  maxWidth: number
}

/** A thumbnail in a message's run of pictures: 80 pixels tall, 64 to 200 wide. */
export const MESSAGE_THUMBNAIL: ThumbnailBounds = { height: THUMBNAIL_HEIGHT, minWidth: 64, maxWidth: 200 }

/**
 * A picture in a row's own line, a tool's or a notice's
 * (`file-previews.md` § Media a tool returned): 20 pixels tall, 16 to 48
 * wide.
 */
export const ROW_PICTURE: ThumbnailBounds = { height: 20, minWidth: 16, maxWidth: 48 }

/**
 * The box a thumbnail of a medium of `natural` size takes within `bounds`
 * (`file-previews.md` § Media a tool returned): as tall as the bounds, as
 * wide as its proportions make it within their widths. A medium whose
 * proportions fall outside that range fills the box and is cropped
 * (`object-fit: cover`), a tall one to its top and a wide one to its
 * middle. While its size is unknown, an image's box is the narrowest one
 * and a video's is 16:9, as most recordings are, until its first frame
 * arrives.
 *
 * Never enlarged: the box is never taller or wider than the medium itself,
 * so one smaller than the box keeps its own size where it is narrower or
 * shorter. In a message's run, a 50 × 50 icon is 50 × 50, not widened to 64
 * or raised to 80; a 300 × 40 strip is 200 × 40, cropped to its middle at
 * its own scale.
 */
export function thumbnailBox(
  natural: PixelSize | null,
  kind: ThumbnailKind = 'image',
  bounds: ThumbnailBounds = MESSAGE_THUMBNAIL,
): PixelSize {
  if (natural === null) {
    return kind === 'video'
      ? { width: Math.round(bounds.height * 16 / 9), height: bounds.height }
      : { width: bounds.minWidth, height: bounds.height }
  }
  const scale = Math.min(1, bounds.height / natural.height)
  const width = clamp(natural.width * scale, bounds.minWidth, bounds.maxWidth)
  return {
    width: Math.round(Math.min(width, natural.width)),
    height: Math.round(Math.min(bounds.height, natural.height)),
  }
}

/**
 * How tall a thumbnail's place in a message's run is, a row of the run
 * being as tall as its tallest place (`file-previews.md` § Files named in
 * messages), given the size
 * `declared` known before the medium's bytes arrive, from its reference or
 * its attachment's record: its box's height when that size is known, so four
 * 171 × 40 frames stand in a row 40 pixels tall with no empty band under
 * them; 80 pixels when it is known only once the bytes load, such as a
 * video's before its first frame, so the row never shrinks when they do.
 */
export function thumbnailPlaceHeight(declared: PixelSize | null, kind: ThumbnailKind): number {
  return thumbnailBox(declared, kind).height
}

/**
 * The size a medium's reference or an attachment's record carries
 * (`runtime.md` § Media), which is known before its bytes arrive; null when
 * it carries none.
 */
export function declaredSize(declared: { width?: number; height?: number }): PixelSize | null {
  const { width, height } = declared
  if (width === undefined || height === undefined || width === 0 || height === 0)
    return null
  return { width, height }
}

/**
 * The size an image's or a video's bytes have, once it has loaded them: an
 * image's pixels, a video's frame from its metadata; null before.
 */
export function loadedSize(element: HTMLImageElement | HTMLVideoElement): PixelSize | null {
  if (element instanceof HTMLVideoElement) {
    if (element.readyState < HTMLMediaElement.HAVE_METADATA || element.videoWidth === 0)
      return null
    return { width: element.videoWidth, height: element.videoHeight }
  }
  if (!element.complete || element.naturalWidth === 0)
    return null
  return { width: element.naturalWidth, height: element.naturalHeight }
}

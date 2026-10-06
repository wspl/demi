import { clamp } from '@demicodes/utils'

/** A picture's size in pixels. */
export interface PixelSize {
  width: number
  height: number
}

/** What a thumbnail shows: an image, or a video's first frame. */
export type ThumbnailKind = 'image' | 'video'

/**
 * How tall a thumbnail is: a tool's image or video and one in a run of a
 * message's images and videos (`file-previews.md` § Media a tool returned).
 * The row it stands in keeps this height before the medium loads and after.
 */
export const THUMBNAIL_HEIGHT = 80

/** The narrowest and the widest a thumbnail is; beyond them the medium is cropped. */
const THUMBNAIL_MIN_WIDTH = 64
const THUMBNAIL_MAX_WIDTH = 200

/**
 * The box of a video's thumbnail while its size is unknown, until its first
 * frame arrives: 16:9, as most recordings are.
 */
const UNKNOWN_VIDEO_BOX: PixelSize = { width: Math.round(THUMBNAIL_HEIGHT * 16 / 9), height: THUMBNAIL_HEIGHT }

/**
 * The box a thumbnail of a medium of `natural` size takes (`file-previews.md`
 * § Media a tool returned): 80 pixels tall, as wide as its proportions make
 * it within 64 and 200 pixels. A medium whose proportions fall outside that
 * range fills the box and is cropped (`object-fit: cover`), a tall one to its
 * top and a wide one to its middle. While its size is unknown, an image's
 * box is the narrowest one and a video's is 16:9.
 *
 * Never enlarged: the box is never taller or wider than the medium itself,
 * so one smaller than the box keeps its own size where it is narrower or
 * shorter. A 50 × 50 icon is 50 × 50, not widened to 64 or raised to 80; a
 * 300 × 40 strip is 200 × 40, cropped to its middle at its own scale.
 */
export function thumbnailBox(natural: PixelSize | null, kind: ThumbnailKind = 'image'): PixelSize {
  if (natural === null)
    return kind === 'video' ? UNKNOWN_VIDEO_BOX : { width: THUMBNAIL_MIN_WIDTH, height: THUMBNAIL_HEIGHT }
  const scale = Math.min(1, THUMBNAIL_HEIGHT / natural.height)
  const width = clamp(natural.width * scale, THUMBNAIL_MIN_WIDTH, THUMBNAIL_MAX_WIDTH)
  return {
    width: Math.round(Math.min(width, natural.width)),
    height: Math.round(Math.min(THUMBNAIL_HEIGHT, natural.height)),
  }
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

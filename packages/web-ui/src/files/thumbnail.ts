import { clamp } from '@demicodes/utils'
import { fitScale } from './image-zoom'

/** A picture's size in pixels. */
export interface PixelSize {
  width: number
  height: number
}

/** What a thumbnail shows: an image, or a video's first frame. */
export type ThumbnailKind = 'image' | 'video'

/** The bounds of a thumbnail's box: its height at most, and its narrowest and widest, beyond which the medium is cropped. */
export interface ThumbnailBounds {
  height: number
  minWidth: number
  maxWidth: number
}

/**
 * A picture in a row's own line, a tool's or a notice's: 20 pixels tall, 16 to
 * 48 wide.
 */
export const ROW_PICTURE: ThumbnailBounds = { height: 20, minWidth: 16, maxWidth: 48 }

/**
 * The box a thumbnail of a medium of `natural` size takes within `bounds`: as
 * tall as the bounds, as wide as its proportions make it within their widths.
 *
 * - A medium whose proportions fall outside that range fills the box and is
 *   cropped (`object-fit: cover`), a tall one to its top and a wide one to its
 *   middle.
 * - While its size is unknown, an image's box is the narrowest one and a
 *   video's is 16:9, as most recordings are, until its first frame arrives.
 * - It is never enlarged: the box is never taller or wider than the medium
 *   itself, so one smaller than the box keeps its own size where it is narrower
 *   or shorter. A 16 × 16 icon stays 16 × 16.
 */
export function thumbnailBox(natural: PixelSize | null, kind: ThumbnailKind, bounds: ThumbnailBounds): PixelSize {
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

/** The space between two tiles of a message, side by side and between rows. */
export const TILE_GAP = 8
/** How tall a message's tile is at most. */
export const TILE_HEIGHT = 240

/**
 * The box a picture of a message fits in, lone or in a run: a third of `width`,
 * the width its pictures stand in, less the two gaps between three tiles, and 240
 * pixels tall, so three tiles fill a line.
 *
 * A 760-pixel message's is 248 × 240.
 */
export function tileFrame(width: number): PixelSize {
  return { width: Math.max(0, Math.floor((width - 2 * TILE_GAP) / 3)), height: TILE_HEIGHT }
}

/** A recording's proportions while its size is unknown, as most recordings are, and larger than any tile. */
const UNKNOWN_VIDEO: PixelSize = { width: 1600, height: 900 }

/**
 * The box a message's picture of `natural` size takes, lone or in a run, in a
 * message whose pictures stand `width` pixels wide: scaled down with its
 * proportions kept to fit its tile's frame, never enlarged, never cropped.
 *
 * In a 760-pixel message a 2560 × 1720 screenshot is 248 × 167, a 1000 × 8000
 * page 30 × 240.
 *
 * A video whose size is unknown until its first frame takes a 16:9 box as wide
 * as the frame. An image whose size is unknown has none (null) until its bytes
 * give it one.
 */
export function tileBox(natural: PixelSize | null, kind: ThumbnailKind, width: number): PixelSize | null {
  const frame = tileFrame(width)
  const size = natural ?? (kind === 'video' ? UNKNOWN_VIDEO : null)
  if (size === null)
    return null
  const scale = fitScale(size, frame)
  return {
    width: Math.min(frame.width, Math.round(size.width * scale)),
    height: Math.min(frame.height, Math.round(size.height * scale)),
  }
}

/**
 * The size a medium's reference or an attachment's record carries, which is
 * known before its bytes arrive; null when it carries none.
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

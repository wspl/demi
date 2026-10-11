import { clamp } from '@demicodes/utils'
import type { PixelSize } from './thumbnail'

/**
 * How a picture is zoomed and moved in its frame, as an image viewer does
 * it (`file-previews.md` § The media viewer, Zoom and move): the media
 * viewer's window and the File view's image show it through `ImagePreview`.
 * Sizes and points are in the frame's CSS px, from its top-left corner.
 */

/** The most a picture is enlarged: eight times its actual size. */
export const MAX_ZOOM = 8

/**
 * How far a press moves before it is a drag rather than a click, in CSS px:
 * the platforms' drag threshold (Windows' `SM_CXDRAG` is 4 px; macOS starts
 * a drag after a few points).
 */
export const DRAG_THRESHOLD_PX = 4

/** A point of the frame. */
export interface FramePoint {
  x: number
  y: number
}

/** How the picture stands in the frame: its scale of the actual size, and where its top-left corner is. */
export interface ImageView {
  scale: number
  x: number
  y: number
}

/** Scales this close are the same: a fitted picture stays fitted through rounding. */
const SAME_SCALE = 1e-9

/** The scale that fits `image` in `frame`, scaled down and never enlarged. */
export function fitScale(image: PixelSize, frame: PixelSize): number {
  return Math.min(1, frame.width / image.width, frame.height / image.height)
}

/** Whether `view` shows the picture at its fitted size. */
export function isFitted(view: ImageView, image: PixelSize, frame: PixelSize): boolean {
  return Math.abs(view.scale - fitScale(image, frame)) < SAME_SCALE
}

/**
 * Where the picture's corner may stand on one axis: centred where the
 * picture is no longer than the frame, and otherwise where no empty band
 * shows on a side that still has pixels beyond the frame.
 */
function clampAxis(at: number, length: number, frame: number): number {
  return length <= frame ? (frame - length) / 2 : clamp(at, frame - length, 0)
}

/** `view` kept in the frame: centred on an axis where the picture is smaller, with no empty band on one where it is larger. */
export function clampView(view: ImageView, image: PixelSize, frame: PixelSize): ImageView {
  return {
    scale: view.scale,
    x: clampAxis(view.x, image.width * view.scale, frame.width),
    y: clampAxis(view.y, image.height * view.scale, frame.height),
  }
}

/** The picture fitted to the frame and centred in it, as it opens. */
export function fittedView(image: PixelSize, frame: PixelSize): ImageView {
  return clampView({ scale: fitScale(image, frame), x: 0, y: 0 }, image, frame)
}

/**
 * `view` zoomed to `scale` about `at`, so the spot of the picture under it
 * stays under it: from the fitted size, or the actual size where that is
 * smaller, up to eight times the actual size.
 */
export function zoomAbout(view: ImageView, scale: number, at: FramePoint, image: PixelSize, frame: PixelSize): ImageView {
  const next = clamp(scale, fitScale(image, frame), MAX_ZOOM)
  const ratio = next / view.scale
  return clampView({
    scale: next,
    x: at.x - (at.x - view.x) * ratio,
    y: at.y - (at.y - view.y) * ratio,
  }, image, frame)
}

/**
 * The scale a turn of the wheel makes of `scale`: `deltaY` px of scrolling
 * down zooms out and up zooms in, by the same factor either way. A
 * trackpad's pinch comes as a wheel with the Control key, in smaller steps,
 * so it counts for more.
 */
export function wheelScale(scale: number, deltaY: number, pinch: boolean): number {
  return scale * Math.exp(-deltaY * (pinch ? 0.01 : 0.002))
}

/** The picture moved by a drag of (`dx`, `dy`) from `start`, kept in the frame. */
export function pannedView(start: ImageView, dx: number, dy: number, image: PixelSize, frame: PixelSize): ImageView {
  return clampView({ scale: start.scale, x: start.x + dx, y: start.y + dy }, image, frame)
}

/** Whether a press released (`dx`, `dy`) from where it started is a click rather than a drag. */
export function isClick(dx: number, dy: number): boolean {
  return Math.hypot(dx, dy) < DRAG_THRESHOLD_PX
}

/**
 * What a click at `at` makes of `view`: a fitted picture goes to its actual
 * size about the clicked spot, and any other zoom back to the fitted size.
 */
export function toggledView(view: ImageView, at: FramePoint, image: PixelSize, frame: PixelSize): ImageView {
  return isFitted(view, image, frame) ? zoomAbout(view, 1, at, image, frame) : fittedView(image, frame)
}

/**
 * `view` after the frame went from `from` to `to`: a fitted picture fits the
 * new frame, and a zoomed one keeps its zoom, the spot at the frame's centre
 * staying at its centre.
 */
export function resizedView(view: ImageView, image: PixelSize, from: PixelSize, to: PixelSize): ImageView {
  if (isFitted(view, image, from))
    return fittedView(image, to)
  return clampView({
    scale: view.scale,
    x: view.x + (to.width - from.width) / 2,
    y: view.y + (to.height - from.height) / 2,
  }, image, to)
}

const ZOOM = new Intl.NumberFormat('en-US', { style: 'percent', maximumFractionDigits: 0 })

/** A zoom as the title bar and the File view show it: "150%". */
export function formatZoom(scale: number): string {
  return ZOOM.format(scale)
}

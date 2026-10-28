// The camera of a demo's screen: it brings close the part of the screen the
// scene points at, scaling the page's root inside the frame, so the page's
// content and where the camera points change in the same frame, as a play
// opens on its first shot.
import { CAMERA_MS, type CameraRect } from './message'

/** How far the camera comes in at most: past it, the app's text grows coarse. */
const MAX_ZOOM = 1.6
/** The room the camera leaves around what it brings close, in screen pixels. */
const MARGIN = 16

/** The camera's zoom for `rect` on a screen of `width` × `height`, and the screen's point at the view's top left corner. */
function viewOf(rect: CameraRect | null, width: number, height: number): { zoom: number; x: number; y: number } {
  if (!rect)
    return { zoom: 1, x: 0, y: 0 }
  const fit = Math.min(width / (rect.width + 2 * MARGIN), height / (rect.height + 2 * MARGIN))
  const zoom = Math.min(MAX_ZOOM, Math.max(1, fit))
  const shown = { width: width / zoom, height: height / zoom }
  const clamp = (value: number, max: number): number => Math.min(max, Math.max(0, value))
  return {
    zoom,
    x: clamp(rect.x + rect.width / 2 - shown.width / 2, width - shown.width),
    y: clamp(rect.y + rect.height / 2 - shown.height / 2, height - shown.height),
  }
}

// A sheet of its own, which the play's changes to the page's head never touch.
const sheet = new CSSStyleSheet()
document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet]

/** Points the camera at `rect`, or at the whole screen for `null`: it cuts there, or moves in `CAMERA_MS`. */
export function aim(rect: CameraRect | null, cut: boolean): void {
  const { zoom, x, y } = viewOf(rect, window.innerWidth, window.innerHeight)
  const motion = cut ? 'none' : `transform ${CAMERA_MS}ms cubic-bezier(0.45, 0, 0.2, 1)`
  sheet.replaceSync(`html { transform-origin: 0 0; transform: scale(${zoom}) translate(${-x}px, ${-y}px); transition: ${motion}; }`)
}

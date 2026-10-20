import { expect, test } from 'bun:test'
import {
  clampView,
  fittedView,
  isClick,
  pannedView,
  rescaledView,
  resizedView,
  toggledView,
  wheelScale,
  zoomAbout,
  type FramePoint,
  type ImageView,
} from '../image-zoom'

// Zooming and moving a picture in the media viewer and the File view, on
// numbers alone: a few milliseconds.

const shot = { width: 1280, height: 720 }
const frame = { width: 640, height: 480 }

/** The spot of the picture, in its own pixels, under a point of the frame. */
function spotUnder(view: ImageView, at: FramePoint): FramePoint {
  return { x: (at.x - view.x) / view.scale, y: (at.y - view.y) / view.scale }
}

test('a picture opens fitted, scaled down and never enlarged, and centred', () => {
  expect(fittedView(shot, frame)).toEqual({ scale: 0.5, x: 0, y: 60 })
  expect(fittedView({ width: 100, height: 50 }, frame)).toEqual({ scale: 1, x: 270, y: 215 })
})

test('the wheel zooms about the pointer: the spot under it stays under it', () => {
  // A window of the picture's proportions, so the picture is larger than it both ways once zoomed in.
  const room = { width: 640, height: 360 }
  const fitted = fittedView(shot, room)
  const at = { x: 500, y: 300 }
  let view = fitted
  for (const deltaY of [-100, -100, -300, 100]) {
    const next = zoomAbout(view, wheelScale(view.scale, deltaY, false), at, shot, room)
    expect(spotUnder(next, at).x).toBeCloseTo(spotUnder(view, at).x, 6)
    expect(spotUnder(next, at).y).toBeCloseTo(spotUnder(view, at).y, 6)
    view = next
  }
  expect(view.scale).toBeGreaterThan(fitted.scale)
  // Scrolling down zooms out as much as scrolling up zoomed in; a pinch counts for more per pixel.
  expect(wheelScale(wheelScale(1, -100, false), 100, false)).toBeCloseTo(1, 9)
  expect(wheelScale(1, -10, true)).toBeGreaterThan(wheelScale(1, -10, false))
})

test('zoom stops at the fitted size, or the actual size when smaller, and at eight times the actual size', () => {
  const at = { x: 320, y: 240 }
  expect(zoomAbout(fittedView(shot, frame), 0.1, at, shot, frame)).toEqual(fittedView(shot, frame))
  expect(zoomAbout(fittedView(shot, frame), 20, at, shot, frame).scale).toBe(8)
  const icon = { width: 100, height: 50 }
  expect(zoomAbout(fittedView(icon, frame), 0.5, at, icon, frame).scale).toBe(1)
})

test('a zoomed picture never leaves an empty band on a side that still has pixels', () => {
  const actual = clampView({ scale: 1, x: 0, y: 0 }, shot, frame)
  // Dragged far right and down, its top-left corner stops at the frame's.
  expect(pannedView(actual, 5000, 5000, shot, frame)).toEqual({ scale: 1, x: 0, y: 0 })
  // Dragged far left and up, its bottom-right corner stops at the frame's.
  expect(pannedView(actual, -5000, -5000, shot, frame)).toEqual({ scale: 1, x: 640 - 1280, y: 480 - 720 })
  // Wider than the frame but not taller: it moves sideways and stays centred up and down.
  const wide = { width: 1280, height: 200 }
  expect(pannedView({ scale: 1, x: -100, y: 140 }, -50, 300, wide, frame)).toEqual({ scale: 1, x: -150, y: 140 })
  // A fitted picture does not move.
  expect(pannedView(fittedView(shot, frame), 80, 80, shot, frame)).toEqual(fittedView(shot, frame))
})

test('a click without a drag toggles between the fitted and the actual size about the clicked spot', () => {
  expect(isClick(0, 0)).toBe(true)
  expect(isClick(3, 0)).toBe(true)
  expect(isClick(3, 3)).toBe(false)
  expect(isClick(0, -12)).toBe(false)
  const fitted = fittedView(shot, frame)
  const at = { x: 400, y: 200 }
  const actual = toggledView(fitted, at, shot, frame)
  expect(actual.scale).toBe(1)
  expect(spotUnder(actual, at)).toEqual(spotUnder(fitted, at))
  expect(toggledView(actual, { x: 10, y: 10 }, shot, frame)).toEqual(fitted)
  // From any other zoom, a click fits it again.
  expect(toggledView({ scale: 3, x: -900, y: -600 }, at, shot, frame)).toEqual(fitted)
})

test('resizing the frame keeps a fitted picture fitted and a zoomed one at its zoom', () => {
  const larger = { width: 1000, height: 800 }
  expect(resizedView(fittedView(shot, frame), shot, frame, larger)).toEqual(fittedView(shot, larger))
  const zoomed = resizedView({ scale: 2, x: -1000, y: -500 }, shot, frame, larger)
  expect(zoomed).toEqual({ scale: 2, x: -820, y: -340 })
})

test('a picture at 100% is one pixel per device pixel: a retina screenshot is half its pixels wide on the page', () => {
  // 2880 × 1800 pixels on two device pixels per page pixel: 1440 × 900 page px is the actual size.
  const actual = { width: 1440, height: 900 }
  const room = { width: 1440, height: 900 }
  expect(fittedView(actual, room)).toEqual({ scale: 1, x: 0, y: 0 })
  expect(fittedView(actual, { width: 720, height: 450 }).scale).toBe(0.5)
})

test('another display\'s pixel ratio changes the zoom the picture reads at, not its size on the page', () => {
  // Fitted at 100% on two device pixels per page pixel; moved to one, the actual size doubles and the same 1440 page px read 50%.
  const view = { scale: 1, x: 0, y: 0 }
  const moved = rescaledView(view, 1 / 2)
  expect(moved).toEqual({ scale: 0.5, x: 0, y: 0 })
  expect(2880 * moved.scale).toBe(1440)
})

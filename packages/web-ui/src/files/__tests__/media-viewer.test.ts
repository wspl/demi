import { expect, test } from 'bun:test'
import { shallowRef } from 'vue'
import { WINDOW_TITLE_BAR_PX } from '../../ui/floating-window'
import { createMediaViewer, type ViewerPlace } from '../media-viewer'

// The page's one viewer, with a fixed page instead of the DOM: a few milliseconds.

const PLACE: ViewerPlace = { page: { width: 1440, height: 900 }, pixelRatio: 1 }
const shot = { kind: 'image' as const, src: '/blob/a', name: 'login-page.png', size: { width: 400, height: 300 } }
const other = { kind: 'image' as const, src: '/blob/b', name: 'dashboard.png', size: { width: 800, height: 600 } }

test('the first picture opens at its actual size, centred on the page', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  expect(viewer.geometry.value?.rect).toEqual({ x: 520, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
})

test('on a display of two device pixels per page pixel a picture opens at half its pixels', () => {
  const viewer = createMediaViewer(() => ({ ...PLACE, pixelRatio: 2 }))
  viewer.show({ ...shot, size: { width: 800, height: 600 } })
  expect(viewer.geometry.value?.rect).toEqual({ x: 520, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
})

test('another picture shown in the open window takes its own size, centred on the page, wherever the user put the window', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  viewer.place({ x: 500, y: 300, width: 500, height: 336 })
  viewer.show({ ...other, size: { width: 600, height: 400 } })
  expect(viewer.shown.value?.name).toBe('dashboard.png')
  expect(viewer.geometry.value?.rect).toEqual({ x: 420, y: 232, width: 600, height: 400 + WINDOW_TITLE_BAR_PX })
})

test('the same picture shown again leaves the window as the user left it', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.show(shot)
  expect(viewer.geometry.value?.rect).toEqual({ x: 20, y: 40, width: 500, height: 360 })
})

test('after a close the next picture opens centred on the page, not where the last one was left', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.close()
  expect(viewer.shown.value).toBeNull()
  viewer.show({ ...other, size: { width: 600, height: 400 } })
  expect(viewer.geometry.value?.rect).toEqual({ x: 420, y: 232, width: 600, height: 400 + WINDOW_TITLE_BAR_PX })
})

test('a picture of unknown size takes its size once it loads, centred on the page, and only then', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show({ ...shot, size: null })
  expect(viewer.geometry.value?.rect).toEqual({ x: 288, y: 135, width: 864, height: 630 })
  viewer.measured({ width: 400, height: 300 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 520, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
  viewer.measured({ width: 800, height: 600 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 520, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
})

test('a window the user moved before its picture loaded stays where they put it', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show({ ...shot, size: null })
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.measured({ width: 400, height: 300 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 20, y: 40, width: 500, height: 360 })
})

// Stepping through the pictures of the transcript the viewer was opened from.

const names = ['a', 'b', 'c'].map((name) => ({ kind: 'image' as const, src: `/blob/${name}`, name: `${name}.png`, size: { width: 400, height: 300 } }))
const [a, b, c] = names as [(typeof names)[number], (typeof names)[number], (typeof names)[number]]

test('a picture opened from a transcript tells its place among the transcript\'s, and one opened on its own tells none', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(b, () => names)
  expect(viewer.position.value).toEqual({ at: 2, of: 3 })
  viewer.show(b)
  expect(viewer.position.value).toBeNull()
})

test('stepping shows the picture before or after, and stops at the first and the last', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(a, () => names)
  viewer.step(-1)
  expect(viewer.shown.value).toBe(a)
  viewer.step(1)
  viewer.step(1)
  expect(viewer.shown.value).toBe(c)
  expect(viewer.position.value).toEqual({ at: 3, of: 3 })
  viewer.step(1)
  expect(viewer.shown.value).toBe(c)
  viewer.step(-1)
  expect(viewer.shown.value).toBe(b)
})

test('stepping to a picture opens the window at that picture\'s own size, centred on the page, and drops the size the user dragged', () => {
  const viewer = createMediaViewer(() => PLACE)
  const wide = { ...b, size: { width: 600, height: 400 } }
  viewer.show(a, () => [a, wide])
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.step(1)
  expect(viewer.geometry.value?.rect).toEqual({ x: 420, y: 232, width: 600, height: 400 + WINDOW_TITLE_BAR_PX })
  viewer.step(-1)
  expect(viewer.geometry.value?.rect).toEqual({ x: 520, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
})

test('the same picture standing twice is two places, and stepping gets past it', () => {
  const viewer = createMediaViewer(() => PLACE)
  const media = [a, b, a, c]
  viewer.show(a, () => media)
  viewer.step(1)
  viewer.step(1)
  expect(viewer.position.value).toEqual({ at: 3, of: 4 })
  viewer.step(1)
  expect(viewer.shown.value).toBe(c)
})

test('the same picture in two rows is told apart by the row it was opened from', () => {
  const viewer = createMediaViewer(() => PLACE)
  const media = [{ ...a, row: 'r1' }, b, { ...a, row: 'r2' }, c]
  viewer.show({ ...a, row: 'r2' }, () => media)
  expect(viewer.position.value).toEqual({ at: 3, of: 4 })
  viewer.step(-1)
  expect(viewer.shown.value).toBe(b)
})

test('the viewer follows its transcript while it is open: a picture added before this one moves its place', () => {
  const viewer = createMediaViewer(() => PLACE)
  const media = shallowRef([b, c])
  viewer.show(b, () => media.value)
  expect(viewer.position.value).toEqual({ at: 1, of: 2 })
  media.value = [a, b, c]
  expect(viewer.position.value).toEqual({ at: 2, of: 3 })
  viewer.step(-1)
  expect(viewer.shown.value).toBe(a)
})

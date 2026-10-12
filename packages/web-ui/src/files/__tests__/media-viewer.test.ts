import { expect, test } from 'bun:test'
import { WINDOW_TITLE_BAR_PX } from '../../ui/floating-window'
import { createMediaViewer, type ViewerPlace } from '../media-viewer'

// The page's one viewer, with a fixed page instead of the DOM: a few milliseconds.

const PLACE: ViewerPlace = { page: { width: 1440, height: 900 }, pixelRatio: 1, over: { x: 300, y: 0, width: 1140, height: 900 } }
const shot = { kind: 'image' as const, src: '/blob/a', name: 'login-page.png', size: { width: 400, height: 300 } }
const other = { kind: 'image' as const, src: '/blob/b', name: 'dashboard.png', size: { width: 800, height: 600 } }

test('the first picture opens at its actual size, centred over the conversation', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  expect(viewer.geometry.value?.rect).toEqual({ x: 670, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
})

test('on a display of two device pixels per page pixel a picture opens at half its pixels', () => {
  const viewer = createMediaViewer(() => ({ ...PLACE, pixelRatio: 2 }))
  viewer.show({ ...shot, size: { width: 800, height: 600 } })
  expect(viewer.geometry.value?.rect).toEqual({ x: 670, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
})

test('another picture shown in the open window resizes it about its centre, in place of the first', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  viewer.place({ x: 500, y: 300, width: 500, height: 336 })
  viewer.show({ ...other, size: { width: 600, height: 400 } })
  expect(viewer.shown.value?.name).toBe('dashboard.png')
  expect(viewer.geometry.value?.rect).toEqual({ x: 450, y: 250, width: 600, height: 400 + WINDOW_TITLE_BAR_PX })
})

test('the same picture shown again leaves the window as the user left it', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.show(shot)
  expect(viewer.geometry.value?.rect).toEqual({ x: 20, y: 40, width: 500, height: 360 })
})

test('after a close the next picture opens centred over the conversation, not where the last one was left', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.close()
  expect(viewer.shown.value).toBeNull()
  viewer.show({ ...other, size: { width: 600, height: 400 } })
  expect(viewer.geometry.value?.rect).toEqual({ x: 570, y: 232, width: 600, height: 400 + WINDOW_TITLE_BAR_PX })
})

test('a picture of unknown size takes its size once it loads, and only then', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show({ ...shot, size: null })
  expect(viewer.geometry.value?.rect).toEqual({ x: 438, y: 135, width: 864, height: 630 })
  viewer.measured({ width: 400, height: 300 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 670, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
  viewer.measured({ width: 800, height: 600 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 670, y: 282, width: 400, height: 300 + WINDOW_TITLE_BAR_PX })
})

test('a window the user moved before its picture loaded stays where they put it', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show({ ...shot, size: null })
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.measured({ width: 400, height: 300 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 20, y: 40, width: 500, height: 360 })
})

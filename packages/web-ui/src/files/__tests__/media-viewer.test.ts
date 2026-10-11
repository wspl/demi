import { expect, test } from 'bun:test'
import { createMediaViewer, type ViewerPlace } from '../media-viewer'

// The page's one viewer, with a fixed page instead of the DOM: a few milliseconds.

const PLACE: ViewerPlace = { page: { width: 1440, height: 900 }, over: { x: 300, y: 0, width: 1140, height: 900 } }
const shot = { kind: 'image' as const, src: '/blob/a', name: 'login-page.png', size: { width: 400, height: 300 } }
const other = { kind: 'image' as const, src: '/blob/b', name: 'dashboard.png', size: { width: 800, height: 600 } }

test('opening another medium shows it in the same window, in place of the first', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  const first = viewer.geometry.value
  viewer.show(other)
  expect(viewer.shown.value?.name).toBe('dashboard.png')
  expect(viewer.geometry.value).toBe(first)
})

test('the window opens again where the user left it, at the size they gave it', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show(shot)
  expect(viewer.geometry.value?.rect).toEqual({ x: 670, y: 282, width: 400, height: 336 })
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.close()
  expect(viewer.shown.value).toBeNull()
  viewer.show(other)
  expect(viewer.geometry.value?.rect).toEqual({ x: 20, y: 40, width: 500, height: 360 })
})

test('a first medium of unknown size sizes the window once it loads, and only then', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show({ ...shot, size: null })
  expect(viewer.geometry.value?.rect).toEqual({ x: 438, y: 135, width: 864, height: 630 })
  viewer.measured({ width: 400, height: 300 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 670, y: 282, width: 400, height: 336 })
  viewer.measured({ width: 800, height: 600 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 670, y: 282, width: 400, height: 336 })
})

test('a window the user moved before its medium loaded stays where they put it', () => {
  const viewer = createMediaViewer(() => PLACE)
  viewer.show({ ...shot, size: null })
  viewer.place({ x: 20, y: 40, width: 500, height: 360 })
  viewer.measured({ width: 400, height: 300 })
  expect(viewer.geometry.value?.rect).toEqual({ x: 20, y: 40, width: 500, height: 360 })
})

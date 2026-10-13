import { expect, test } from 'bun:test'
import {
  DOCK_WINDOW_SHARE,
  dockWindowBounds,
  dockWindowHeight,
  dockWindowShare,
} from '../dock-window'

const chrome = { header: 40, bar: 32, line: 22 }

test('the smallest window holds its header, its bar and three lines of the transcript', () => {
  expect(dockWindowBounds(chrome, 500)).toEqual({ min: 40 + 32 + 3 * 22, max: 500 })
  expect(dockWindowBounds({ ...chrome, bar: 0 }, 500).min).toBe(40 + 3 * 22)
})

test('a height is a share of the area, so a large window stays large in a small area', () => {
  const tall = dockWindowShare(600, 1000)
  expect(tall).toBe(0.6)
  expect(dockWindowHeight(tall, 1000, dockWindowBounds(chrome, 900))).toBe(600)
  // In a 500px area the same share is 300px.
  expect(dockWindowHeight(tall, 500, dockWindowBounds(chrome, 400))).toBe(300)
})

test('a stored share never leaves the edge out of reach: it is held within the room', () => {
  const bounds = dockWindowBounds(chrome, 400)
  expect(dockWindowHeight(0.95, 500, bounds)).toBe(400)
  expect(dockWindowHeight(0, 500, bounds)).toBe(bounds.min)
  // Less room than the smallest window: the window takes what there is.
  const cramped = dockWindowBounds(chrome, 100)
  expect(cramped).toEqual({ min: 100, max: 100 })
  expect(dockWindowHeight(0.5, 500, cramped)).toBe(100)
})

test('an unmeasured area stores the default share', () => {
  expect(dockWindowShare(300, 0)).toBe(DOCK_WINDOW_SHARE.default)
})

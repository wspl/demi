import { expect, test } from 'bun:test'
import { clampSize, sizeFromDrag, sizeFromKey } from '../resize-handle'

const bounds = { min: 200, max: 400 }

test('a size stays inside its bounds and on whole pixels', () => {
  expect(clampSize(120, bounds)).toBe(200)
  expect(clampSize(999, bounds)).toBe(400)
  expect(clampSize(256.4, bounds)).toBe(256)
})

test('a drag moves the edge in the pane’s direction', () => {
  expect(sizeFromDrag(256, 40, 'start', bounds)).toBe(296)
  expect(sizeFromDrag(256, 40, 'end', bounds)).toBe(216)
  expect(sizeFromDrag(256, -500, 'start', bounds)).toBe(200)
  expect(sizeFromDrag(256, 500, 'start', bounds)).toBe(400)
})

test('the arrows along the axis step the edge; Shift steps four times', () => {
  const vertical = { ...bounds, orientation: 'vertical' as const, side: 'start' as const, step: 16 }
  expect(sizeFromKey('ArrowRight', false, 256, vertical)).toBe(272)
  expect(sizeFromKey('ArrowLeft', false, 256, vertical)).toBe(240)
  expect(sizeFromKey('ArrowRight', true, 256, vertical)).toBe(320)
  expect(sizeFromKey('ArrowUp', false, 256, vertical)).toBeNull()
  expect(sizeFromKey('a', false, 256, vertical)).toBeNull()

  const horizontal = { ...vertical, orientation: 'horizontal' as const, side: 'end' as const }
  expect(sizeFromKey('ArrowDown', false, 256, horizontal)).toBe(240)
  expect(sizeFromKey('ArrowUp', false, 256, horizontal)).toBe(272)
})

test('Home and End go to the bounds', () => {
  const options = { ...bounds, orientation: 'vertical' as const, side: 'start' as const, step: 16 }
  expect(sizeFromKey('Home', false, 256, options)).toBe(200)
  expect(sizeFromKey('End', false, 256, options)).toBe(400)
})

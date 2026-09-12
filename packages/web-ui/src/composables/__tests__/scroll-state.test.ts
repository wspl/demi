import { expect, test } from 'bun:test'
import {
  scrollPositionSchema,
  persistedScrollStateSchema,
} from '../scroll-state'

test('scroll snapshots reject arrays, non-finite coordinates and malformed anchors', () => {
  expect(
    scrollPositionSchema.parse({ top: 3.5, anchor: null, offset: -2 }),
  ).toEqual({ top: 3.5, anchor: null, offset: -2 })
  for (const value of [
    [],
    { top: -1, anchor: null, offset: 0 },
    { top: 0, anchor: {}, offset: 0 },
    { top: 0, anchor: null, offset: Infinity },
  ]) {
    expect(scrollPositionSchema.safeParse(value).success).toBe(false)
  }
  const valid = {
    anchor: { blockId: 'b', anchorIndex: 0, offsetPx: -2, scrollTop: 10 },
    heightCache: new Map([['b', 12.5]]),
  }
  expect(persistedScrollStateSchema.safeParse(valid).success).toBe(true)
  expect(
    persistedScrollStateSchema.safeParse({
      ...valid,
      heightCache: new Map([['b', -1]]),
    }).success,
  ).toBe(false)
})
